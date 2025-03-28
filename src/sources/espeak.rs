#![allow(non_upper_case_globals)]
use crate::{
    buffer::PlaybackBuffer,
    event::{Event, EventBus},
    mixer::{MixerAction, MixerInput, Sample},
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub enum Priority {
    #[default]
    Low,
    High,
}

#[derive(Clone, Debug)]
pub enum TextToSpeechAction {
    Speak { text: String, prio: Priority },
    AllowLowPrio,
    DisallowLowPrio,
}

pub fn init(bus: &EventBus) -> MixerInput {
    let (tx, rx) = mpsc::channel(128);
    let playback_buf = Arc::new(Mutex::new(PlaybackBuffer::default()));

    start_speak_event_loop(bus.clone(), playback_buf.clone());
    start_emit_sample_loop(bus.clone(), tx, playback_buf);

    rx
}

fn start_speak_event_loop(bus: EventBus, playback_buf: Arc<Mutex<PlaybackBuffer>>) {
    tokio::spawn(async move {
        // Check for any new events on the bus
        let mut bus = bus.subscribe();

        loop {
            let event = bus.recv().await;

            if let Event::TextToSpeech(TextToSpeechAction::Speak { text, prio }) = event {
                let spoken =
                    tokio::task::spawn_blocking(move || espeakng_sys_example::speak(&text)).await;

                let spoken = match spoken {
                    Ok(spoken) => spoken,
                    Err(e) => {
                        error!("Error while calling espeakng: {:?}", e);
                        continue;
                    }
                };

                let mut playback_buf = playback_buf.lock().await;
                if prio == Priority::High {
                    playback_buf.clear();
                }

                // Add some silence before the sample
                let mut audio = vec![0; 5000];

                audio.extend(spoken.wav);

                // Add some silence after the sample
                audio.extend(vec![0; 5000]);

                let audio: Vec<Sample> = audio.into_iter().map(|sample| (sample, sample)).collect();

                playback_buf.push_samples(audio);
            }
        }
    });
}

fn start_emit_sample_loop(
    bus: EventBus,
    tx: mpsc::Sender<Sample>,
    playback_buf: Arc<Mutex<PlaybackBuffer>>,
) {
    tokio::spawn(async move {
        let mut speaking = false;

        loop {
            let was_speaking = speaking;

            let sample = {
                let mut playback_buf = playback_buf.lock().await;
                let sample = playback_buf.next_sample();

                speaking = sample.is_some();

                sample.unwrap_or_default()
            };

            if speaking != was_speaking {
                if speaking {
                    bus.send(Event::Mixer(MixerAction::DuckSecondaryChannels))
                } else {
                    bus.send(Event::Mixer(MixerAction::UnduckSecondaryChannels))
                }
            }

            // Send the same sample twice to resample from 22050 Hz to to 44100 Hz
            for _ in 0..2 {
                tx.send(sample)
                    .await
                    .expect("Expected mixer channel to never close");
            }
        }
    });
}

// https://github.com/Better-Player/espeakng-sys/tree/9aeadd42772da076c1a1d5fbcd6384b8c9d56bba#example
mod espeakng_sys_example {
    use espeakng_sys::*;
    use lazy_static::lazy_static;
    use std::cell::Cell;
    use std::ffi::{c_void, CString};
    use std::os::raw::{c_char, c_int, c_short};
    use std::sync::{Mutex, MutexGuard};

    /// The name of the voice to use
    const VOICE_NAME: &str = "Finnish";
    /// The length in mS of sound buffers passed to the SynthCallback function.
    const BUFF_LEN: i32 = 500;
    /// Options to set for espeak-ng
    const OPTIONS: i32 = 0;

    lazy_static! {
        /// The complete audio provided by the callback
        static ref AUDIO_RETURN: Mutex<Cell<Vec<i16>>> = Mutex::new(Cell::new(Vec::default()));

        /// Audio buffer for use in the callback
        static ref AUDIO_BUFFER: Mutex<Cell<Vec<i16>>> = Mutex::new(Cell::new(Vec::default()));
    }

    /// Spoken speech
    pub struct Spoken {
        /// The audio data
        pub wav: Vec<i16>,
        /// The sample rate of the audio
        #[allow(dead_code)]
        pub sample_rate: i32,
    }

    /// Perform Text-To-Speech
    pub fn speak(text: &str) -> Spoken {
        let output: espeak_AUDIO_OUTPUT = espeak_AUDIO_OUTPUT_AUDIO_OUTPUT_RETRIEVAL;

        AUDIO_RETURN.plock().set(Vec::default());
        AUDIO_BUFFER.plock().set(Vec::default());

        // The directory which contains the espeak-ng-data directory, or NULL for the default location.
        let path: *const c_char = std::ptr::null();
        let voice_name_cstr = CString::new(VOICE_NAME).expect("Failed to convert &str to CString");
        let voice_name = voice_name_cstr.as_ptr();

        // Returns: sample rate in Hz, or -1 (EE_INTERNAL_ERROR).
        let sample_rate = unsafe { espeak_Initialize(output, BUFF_LEN, path, OPTIONS) };

        unsafe {
            espeak_SetVoiceByName(voice_name as *const c_char);
            espeak_SetSynthCallback(Some(synth_callback))
        }

        let text_cstr = CString::new(text).expect("Failed to convert &str to CString");

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

        // Wait for the speaking to complete
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

    /// int SynthCallback(short *wav, int numsamples, espeak_EVENT *events);
    ///
    /// wav:  is the speech sound data which has been produced.
    /// NULL indicates that the synthesis has been completed.
    ///
    /// numsamples: is the number of entries in wav.  This number may vary, may be less than
    /// the value implied by the buflength parameter given in espeak_Initialize, and may
    /// sometimes be zero (which does NOT indicate end of synthesis).
    ///
    /// events: an array of espeak_EVENT items which indicate word and sentence events, and
    /// also the occurance if <mark> and <audio> elements within the text.  The list of
    /// events is terminated by an event of type = 0.
    ///
    /// Callback returns: 0=continue synthesis,  1=abort synthesis.
    unsafe extern "C" fn synth_callback(
        wav: *mut c_short,
        sample_count: c_int,
        events: *mut espeak_EVENT,
    ) -> c_int {
        // Calculate the length of the events array
        let mut events_copy = events;
        let mut elem_count = 0;
        while (*events_copy).type_ != espeak_EVENT_TYPE_espeakEVENT_LIST_TERMINATED {
            elem_count += 1;
            events_copy = events_copy.add(1);
        }

        // Turn the event array into a Vec.
        // We must clone from the slice, as the provided array's memory is managed by C
        let event_slice = std::slice::from_raw_parts_mut(events, elem_count);
        let event_vec = event_slice
            .iter_mut()
            .map(|f| *f)
            .collect::<Vec<espeak_EVENT>>();

        let mut wav_vec = if sample_count == 0 {
            vec![]
        } else {
            // Turn the audio wav data array into a Vec.
            // We must clone from the slice, as the provided array's memory is managed by C
            let wav_slice = std::slice::from_raw_parts_mut(wav, sample_count as usize);
            wav_slice.iter_mut().map(|f| *f).collect::<Vec<i16>>()
        };

        // Determine if this is the end of the synth
        let mut is_end = false;
        for event in event_vec {
            if event
                .type_
                .eq(&espeak_EVENT_TYPE_espeakEVENT_MSG_TERMINATED)
            {
                is_end = true;
            }
        }

        // If this is the end, we want to set the AUDIO_RETURN
        // Else we want to append to the AUDIO_BUFFER
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
        fn plock(&self) -> MutexGuard<T>;
    }

    impl<T> PoisonlessLock<T> for Mutex<T> {
        fn plock(&self) -> MutexGuard<T> {
            match self.lock() {
                Ok(l) => l,
                Err(e) => e.into_inner(),
            }
        }
    }
}
