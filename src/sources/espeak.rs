//! Text-to-speech audio source using espeak-ng.
//!
//! Receives speech requests via events, generates audio at 22050Hz,
//! resamples to 48kHz, and provides samples to the mixer.

#![allow(non_upper_case_globals)]

use crate::{
    buffer::PlaybackBuffer,
    constants::ESPEAK_SAMPLE_RATE,
    event::{Event, EventBus},
    sources::{Sample, OUTPUT_SAMPLE_RATE},
};
use rubato::{FftFixedIn, Resampler};
use serde::Deserialize;
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub enum Priority {
    #[default]
    Low,
    High,
}

#[derive(Clone, Debug)]
pub enum TextToSpeechAction {
    Speak {
        text: String,
        prio: Priority,
    },
    #[allow(dead_code)]
    AllowLowPrio,
    #[allow(dead_code)]
    DisallowLowPrio,
}

/// Shared buffer type for TTS audio
pub type TtsBuffer = Arc<Mutex<PlaybackBuffer>>;

/// Create a new TTS buffer
pub fn create_buffer() -> TtsBuffer {
    Arc::new(Mutex::new(PlaybackBuffer::new()))
}

/// Initialize the TTS system and return a shared buffer for audio output.
pub fn init(bus: &EventBus) -> TtsBuffer {
    let buffer = create_buffer();
    start_speak_event_loop(bus.clone(), buffer.clone());
    buffer
}

fn start_speak_event_loop(bus: EventBus, buffer: TtsBuffer) {
    tokio::spawn(async move {
        let mut subscriber = bus.subscribe();

        // Create resampler: 22050Hz -> 48000Hz
        let resampler = FftFixedIn::<f64>::new(
            ESPEAK_SAMPLE_RATE as usize,
            OUTPUT_SAMPLE_RATE as usize,
            1024, // chunk size
            2,    // sub-chunks
            1,    // mono input (espeak generates mono)
        );

        let resampler = match resampler {
            Ok(r) => Arc::new(Mutex::new(r)),
            Err(e) => {
                error!("Failed to create espeak resampler: {e}");
                return;
            }
        };

        loop {
            let event = subscriber.recv().await;

            if let Event::TextToSpeech(TextToSpeechAction::Speak { text, prio }) = event {
                let is_high_prio = prio == Priority::High;

                // Generate speech in blocking task
                let spoken =
                    tokio::task::spawn_blocking(move || espeakng_sys_example::speak(&text)).await;

                let spoken = match spoken {
                    Ok(spoken) => spoken,
                    Err(e) => {
                        error!("Error while calling espeakng: {e:?}");
                        continue;
                    }
                };

                // Process and resample audio
                let resampled = resample_audio(&spoken.wav, &resampler);

                // Add to buffer
                if let Ok(mut buf) = buffer.lock() {
                    if is_high_prio {
                        buf.clear();
                    }

                    // Add silence padding before and after (in output sample rate)
                    let silence_samples = (OUTPUT_SAMPLE_RATE as usize) / 10; // 100ms
                    let silence: Vec<Sample> = vec![(0, 0); silence_samples];

                    buf.push_samples(silence.clone());
                    buf.push_samples(resampled);
                    buf.push_samples(silence);
                }
            }
        }
    });
}

/// Resample mono i16 audio from 22050Hz to 48000Hz stereo.
fn resample_audio(input: &[i16], resampler: &Arc<Mutex<FftFixedIn<f64>>>) -> Vec<Sample> {
    if input.is_empty() {
        return vec![];
    }

    let mut resampler = match resampler.lock() {
        Ok(r) => r,
        Err(_) => return vec![],
    };

    // Convert i16 to f64 normalized
    let input_f64: Vec<f64> = input.iter().map(|&s| s as f64 / 32768.0).collect();

    // Wrap in channel vec (mono)
    let input_channels = vec![input_f64];

    // Process in chunks
    let chunk_size = resampler.input_frames_max();
    let mut output = Vec::new();

    for chunk_start in (0..input_channels[0].len()).step_by(chunk_size) {
        let chunk_end = (chunk_start + chunk_size).min(input_channels[0].len());
        let chunk: Vec<Vec<f64>> = input_channels
            .iter()
            .map(|ch| ch[chunk_start..chunk_end].to_vec())
            .collect();

        // Pad last chunk if needed
        let chunk: Vec<Vec<f64>> = if chunk[0].len() < chunk_size {
            chunk
                .into_iter()
                .map(|mut ch| {
                    ch.resize(chunk_size, 0.0);
                    ch
                })
                .collect()
        } else {
            chunk
        };

        match resampler.process(&chunk, None) {
            Ok(resampled) => {
                if !resampled.is_empty() && !resampled[0].is_empty() {
                    // Convert f64 back to i16 stereo samples
                    for &sample in &resampled[0] {
                        let s = (sample * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        output.push((s, s)); // Mono to stereo
                    }
                }
            }
            Err(e) => {
                warn!("Resampling error: {e}");
            }
        }
    }

    // Reset resampler for next use
    resampler.reset();

    output
}

// espeakng-sys example code (unchanged from original)
mod espeakng_sys_example {
    use espeakng_sys::*;
    use lazy_static::lazy_static;
    use std::cell::Cell;
    use std::ffi::{c_void, CString};
    use std::os::raw::{c_char, c_int, c_short};
    use std::sync::{Mutex, MutexGuard};

    const VOICE_NAME: &str = "Finnish";
    const BUFF_LEN: i32 = 500;
    const OPTIONS: i32 = 0;

    lazy_static! {
        static ref AUDIO_RETURN: Mutex<Cell<Vec<i16>>> = Mutex::new(Cell::new(Vec::default()));
        static ref AUDIO_BUFFER: Mutex<Cell<Vec<i16>>> = Mutex::new(Cell::new(Vec::default()));
    }

    pub struct Spoken {
        pub wav: Vec<i16>,
        #[allow(dead_code)]
        pub sample_rate: i32,
    }

    pub fn speak(text: &str) -> Spoken {
        let output: espeak_AUDIO_OUTPUT = espeak_AUDIO_OUTPUT_AUDIO_OUTPUT_RETRIEVAL;

        AUDIO_RETURN.plock().set(Vec::default());
        AUDIO_BUFFER.plock().set(Vec::default());

        let path: *const c_char = std::ptr::null();
        let voice_name_cstr = CString::new(VOICE_NAME).expect("Failed to convert &str to CString");
        let voice_name = voice_name_cstr.as_ptr();

        let sample_rate = unsafe { espeak_Initialize(output, BUFF_LEN, path, OPTIONS) };

        unsafe {
            espeak_SetVoiceByName(voice_name as *const c_char);
            espeak_SetSynthCallback(Some(synth_callback))
        }

        // Filter out null bytes to prevent CString::new from panicking
        let filtered_text: String = text.chars().filter(|&c| c != '\0').collect();
        let text_cstr =
            CString::new(filtered_text).expect("Filtered text should not contain nulls");

        let position = 0u32;
        let position_type: espeak_POSITION_TYPE = 0;
        let end_position = 0u32;
        let flags = espeakCHARS_AUTO;
        let identifier = std::ptr::null_mut();
        let user_data = std::ptr::null_mut();

        unsafe {
            espeak_Synth(
                text_cstr.as_ptr() as *const c_void,
                BUFF_LEN as usize,
                position,
                position_type,
                end_position,
                flags,
                identifier,
                user_data,
            );
        }

        match unsafe { espeak_Synchronize() } {
            espeak_ERROR_EE_OK => {}
            espeak_ERROR_EE_INTERNAL_ERROR => {
                todo!()
            }
            _ => unreachable!(),
        }

        let result = AUDIO_RETURN.plock().take();

        unsafe {
            espeak_Terminate();
        }

        Spoken {
            wav: result,
            sample_rate,
        }
    }

    unsafe extern "C" fn synth_callback(
        wav: *mut c_short,
        sample_count: c_int,
        events: *mut espeak_EVENT,
    ) -> c_int {
        let mut events_copy = events;
        let mut elem_count = 0;
        while (*events_copy).type_ != espeak_EVENT_TYPE_espeakEVENT_LIST_TERMINATED {
            elem_count += 1;
            events_copy = events_copy.add(1);
        }

        let event_slice = std::slice::from_raw_parts_mut(events, elem_count);
        let event_vec = event_slice
            .iter_mut()
            .map(|f| *f)
            .collect::<Vec<espeak_EVENT>>();

        let mut wav_vec = if sample_count == 0 {
            vec![]
        } else {
            let wav_slice = std::slice::from_raw_parts_mut(wav, sample_count as usize);
            wav_slice.iter_mut().map(|f| *f).collect::<Vec<i16>>()
        };

        let mut is_end = false;
        for event in event_vec {
            if event
                .type_
                .eq(&espeak_EVENT_TYPE_espeakEVENT_MSG_TERMINATED)
            {
                is_end = true;
            }
        }

        if is_end {
            AUDIO_RETURN.plock().set(AUDIO_BUFFER.plock().take());
        } else {
            let mut curr_data = AUDIO_BUFFER.plock().take();
            curr_data.append(&mut wav_vec);
            AUDIO_BUFFER.plock().set(curr_data);
        }

        0
    }

    trait PoisonlessLock<T> {
        fn plock(&self) -> MutexGuard<'_, T>;
    }

    impl<T> PoisonlessLock<T> for Mutex<T> {
        fn plock(&self) -> MutexGuard<'_, T> {
            match self.lock() {
                Ok(l) => l,
                Err(e) => e.into_inner(),
            }
        }
    }
}
