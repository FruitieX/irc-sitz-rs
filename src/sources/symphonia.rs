//! Music decoder using symphonia with rubato resampling.
//!
//! Handles decoding from YouTube streams (via yt-dlp) and local files.
//! Resamples to 48kHz using high-quality FFT resampling.

use crate::{
    buffer::PlaybackBuffer,
    event::{Event, EventBus},
    message::MessageAction,
    playback::PlaybackAction,
    sources::{Sample, OUTPUT_SAMPLE_RATE},
    youtube::get_yt_media_source_stream,
};
use anyhow::{Context, Result};
use itertools::Itertools;
use rubato::{FftFixedIn, Resampler};
use std::{
    fs::File,
    path::Path,
    sync::{Arc, Mutex},
};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use tokio::sync::{oneshot, RwLock};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub enum SymphoniaAction {
    PlayFile { file_path: String },
    PlayYtUrl { url: String },
    Stop,
    Pause,
    Resume,
}

/// Shared buffer type for music audio
pub type MusicBuffer = Arc<Mutex<PlaybackBuffer>>;

/// Create a new music buffer
pub fn create_buffer() -> MusicBuffer {
    Arc::new(Mutex::new(PlaybackBuffer::new()))
}

pub async fn init(bus: &EventBus) -> Result<MusicBuffer> {
    let buffer = create_buffer();
    start_decode_event_loop(bus.clone(), buffer.clone());
    Ok(buffer)
}

/// Monitors playback progress and detects end-of-song.
fn start_playback_monitor(bus: EventBus, buffer: MusicBuffer, cancel_token: CancellationToken) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    debug!("Playback monitor cancelled");
                    break;
                }
                _ = interval.tick() => {
                    let (position, is_eof, has_data) = {
                        let buf = buffer.lock().unwrap();
                        (
                            buf.get_total_position_secs(OUTPUT_SAMPLE_RATE) as u64,
                            buf.is_eof(),
                            buf.has_data(),
                        )
                    };

                    bus.send(Event::Playback(PlaybackAction::PlaybackProgress { position }));

                    if is_eof && !has_data {
                        info!("End of song detected");
                        bus.send(Event::Playback(PlaybackAction::EndOfSong));
                        break;
                    }
                }
            }
        }
    });
}

fn start_decode_event_loop(bus: EventBus, buffer: MusicBuffer) {
    tokio::spawn(async move {
        let mut subscriber = bus.subscribe();
        let cancel_decode_tx = Arc::new(RwLock::new(None::<oneshot::Sender<()>>));
        let monitor_cancel = Arc::new(RwLock::new(None::<CancellationToken>));

        loop {
            let event = subscriber.recv().await;

            if let Event::Symphonia(action) = event {
                let buffer = buffer.clone();
                let cancel_decode_tx = cancel_decode_tx.clone();
                let bus = bus.clone();
                let monitor_cancel = monitor_cancel.clone();

                tokio::spawn(async move {
                    let result = handle_action(
                        action,
                        buffer,
                        cancel_decode_tx,
                        bus.clone(),
                        monitor_cancel,
                    )
                    .await;

                    if let Err(e) = result {
                        let msg = format!("Error during music playback: {e}");
                        error!("{msg}");
                        bus.send_message(MessageAction::error(msg));
                    }
                });
            }
        }
    });
}

async fn handle_action(
    action: SymphoniaAction,
    buffer: MusicBuffer,
    cancel_decode_tx: Arc<RwLock<Option<oneshot::Sender<()>>>>,
    bus: EventBus,
    monitor_cancel: Arc<RwLock<Option<CancellationToken>>>,
) -> Result<()> {
    info!("Symphonia action: {action:?}");

    match &action {
        SymphoniaAction::PlayFile { .. } | SymphoniaAction::PlayYtUrl { .. } => {
            // Cancel any existing monitor
            {
                let mut token = monitor_cancel.write().await;
                if let Some(t) = token.take() {
                    t.cancel();
                }
            }

            // Cancel any existing decode task
            let (tx, cancel_rx) = oneshot::channel();
            {
                let mut cancel_tx = cancel_decode_tx.write().await;
                if let Some(old_tx) = cancel_tx.take() {
                    let _ = old_tx.send(());
                }
                *cancel_tx = Some(tx);
            }

            // Clear buffer
            {
                let mut buf = buffer.lock().unwrap();
                buf.clear();
                buf.set_paused(false);
            }

            let (mss, url) = match action {
                SymphoniaAction::PlayFile { file_path } => {
                    let source = Box::new(File::open(Path::new(&file_path))?);
                    (
                        MediaSourceStream::new(source, Default::default()),
                        file_path,
                    )
                }
                SymphoniaAction::PlayYtUrl { url } => {
                    (get_yt_media_source_stream(url.clone()).await?, url)
                }
                _ => unreachable!(),
            };

            // Start playback monitor
            let new_token = CancellationToken::new();
            {
                let mut token = monitor_cancel.write().await;
                *token = Some(new_token.clone());
            }
            start_playback_monitor(bus.clone(), buffer.clone(), new_token);

            // Decode in blocking task
            let result = {
                let buffer = buffer.clone();
                tokio::task::spawn_blocking(move || decode_source(mss, buffer, cancel_rx)).await??
            };

            match result {
                DecodeResult::EndOfFile { total_samples } => {
                    let mut buf = buffer.lock().unwrap();
                    buf.set_eof(true);
                    let duration = total_samples as f64 / OUTPUT_SAMPLE_RATE as f64;
                    info!("Finished decoding {url}: {total_samples} samples = {duration:.2}s");
                }
                DecodeResult::Cancelled => {
                    info!("Cancelled decoding {url}");
                }
            }
        }
        SymphoniaAction::Stop => {
            // Cancel monitor
            {
                let mut token = monitor_cancel.write().await;
                if let Some(t) = token.take() {
                    t.cancel();
                }
            }

            // Cancel decode task
            {
                let mut cancel_tx = cancel_decode_tx.write().await;
                if let Some(tx) = cancel_tx.take() {
                    let _ = tx.send(());
                }
            }

            let mut buf = buffer.lock().unwrap();
            buf.set_paused(true);
        }
        SymphoniaAction::Pause => {
            let mut buf = buffer.lock().unwrap();
            buf.set_paused(true);
        }
        SymphoniaAction::Resume => {
            let mut buf = buffer.lock().unwrap();
            buf.set_paused(false);
        }
    }

    Ok(())
}

pub enum DecodeResult {
    EndOfFile { total_samples: usize },
    Cancelled,
}

/// High water mark - decoder pauses when buffer exceeds this (10 seconds at 48kHz)
const BUFFER_HIGH_WATER: usize = 480000;
/// Low water mark - decoder resumes when buffer drops below this (5 seconds)
const BUFFER_LOW_WATER: usize = 240000;

pub fn decode_source(
    mss: MediaSourceStream,
    buffer: MusicBuffer,
    mut cancel_rx: oneshot::Receiver<()>,
) -> Result<DecodeResult> {
    let hint = Hint::new();
    let format_opts = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let metadata_opts = MetadataOptions::default();
    let decoder_opts = DecoderOptions::default();

    let probed =
        symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;
    let mut format = probed.format;

    let track = format
        .default_track()
        .context("Could not find any tracks")?;

    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;
    let track_id = track.id;

    // Get source sample rate
    let source_rate = track.codec_params.sample_rate.unwrap_or(44100);
    let needs_resampling = source_rate != OUTPUT_SAMPLE_RATE;

    info!("Source sample rate: {source_rate}Hz, needs resampling: {needs_resampling}");

    // Create resampler if needed
    let mut resampler: Option<FftFixedIn<f64>> = if needs_resampling {
        Some(
            FftFixedIn::new(
                source_rate as usize,
                OUTPUT_SAMPLE_RATE as usize,
                1024,
                2,
                2, // stereo
            )
            .context("Failed to create resampler")?,
        )
    } else {
        None
    };

    let mut sample_count = 0usize;
    let mut sample_buf = None;

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(DecodeResult::EndOfFile {
                    total_samples: sample_count,
                });
            }
            Err(e) => return Err(e.into()),
        };

        if packet.track_id() != track_id {
            continue;
        }

        let audio_buf = decoder.decode(&packet)?;

        if sample_buf.is_none() {
            let spec = *audio_buf.spec();
            let duration = audio_buf.capacity() as u64;
            sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
        }

        if let Some(buf) = &mut sample_buf {
            buf.copy_interleaved_ref(audio_buf);
            let raw_samples = buf.samples();

            // Convert to stereo sample pairs
            let source_samples: Vec<Sample> = raw_samples.iter().copied().tuples().collect();

            // Resample if needed
            let output_samples = if let Some(ref mut resampler) = resampler {
                resample_stereo(&source_samples, resampler)?
            } else {
                source_samples
            };

            sample_count += output_samples.len();

            // Check for cancellation
            if !matches!(
                cancel_rx.try_recv(),
                Err(oneshot::error::TryRecvError::Empty)
            ) {
                return Ok(DecodeResult::Cancelled);
            }

            // Push to buffer
            {
                let mut buf = buffer.lock().unwrap();
                buf.push_samples(output_samples);
            }

            // Backpressure
            loop {
                let level = {
                    let buf = buffer.lock().unwrap();
                    buf.buffer_level()
                };

                if level < BUFFER_HIGH_WATER {
                    break;
                }

                // Check cancellation while waiting
                if !matches!(
                    cancel_rx.try_recv(),
                    Err(oneshot::error::TryRecvError::Empty)
                ) {
                    return Ok(DecodeResult::Cancelled);
                }

                std::thread::sleep(std::time::Duration::from_millis(20));

                let level = {
                    let buf = buffer.lock().unwrap();
                    buf.buffer_level()
                };
                if level < BUFFER_LOW_WATER {
                    break;
                }
            }
        }
    }
}

/// Resample stereo audio using rubato
fn resample_stereo(input: &[Sample], resampler: &mut FftFixedIn<f64>) -> Result<Vec<Sample>> {
    if input.is_empty() {
        return Ok(vec![]);
    }

    // Split into left/right channels and convert to f64
    let left: Vec<f64> = input.iter().map(|(l, _)| *l as f64 / 32768.0).collect();
    let right: Vec<f64> = input.iter().map(|(_, r)| *r as f64 / 32768.0).collect();

    let chunk_size = resampler.input_frames_max();
    let mut output = Vec::new();

    for chunk_start in (0..left.len()).step_by(chunk_size) {
        let chunk_end = (chunk_start + chunk_size).min(left.len());
        let mut left_chunk = left[chunk_start..chunk_end].to_vec();
        let mut right_chunk = right[chunk_start..chunk_end].to_vec();

        // Pad last chunk if needed
        if left_chunk.len() < chunk_size {
            left_chunk.resize(chunk_size, 0.0);
            right_chunk.resize(chunk_size, 0.0);
        }

        let input_channels = vec![left_chunk, right_chunk];

        match resampler.process(&input_channels, None) {
            Ok(resampled) => {
                if resampled.len() >= 2 && !resampled[0].is_empty() {
                    for i in 0..resampled[0].len() {
                        let l = (resampled[0][i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        let r = (resampled[1][i] * 32767.0).clamp(-32768.0, 32767.0) as i16;
                        output.push((l, r));
                    }
                }
            }
            Err(e) => {
                warn!("Resampling error: {e}");
            }
        }
    }

    Ok(output)
}
