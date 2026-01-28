use crate::{
    constants::SAMPLE_RATE,
    event::{Event, EventBus},
};
use anyhow::Result;
use tokio::sync::{mpsc, watch};

/// Number of audio samples to process per chunk.
/// Smaller values reduce latency but increase CPU overhead.
const TARGET_CHUNK_SIZE: usize = 128;

/// Microseconds per second, used for timing calculations
const MICROS_PER_SECOND: f64 = 1_000_000.0;

#[derive(Clone, Debug)]
pub enum MixerAction {
    DuckSecondaryChannels,
    UnduckSecondaryChannels,
    SetSecondaryChannelVolume(f64),
    SetSecondaryChannelDuckedVolume(f64),
}

/// Volume multiplier for the primary audio channel (TTS/speech).
/// Set above 1.0 to make speech louder than music.
const PRIMARY_CHANNEL_VOLUME: f64 = 1.25;

/// Default volume for secondary channels (music) during normal playback.
/// Range: 0.0 (silent) to 1.0 (full volume).
const INIT_SECONDARY_CHANNEL_VOLUME_TARGET: f64 = 0.75;

/// Volume for secondary channels when "ducked" (during TTS playback).
/// Lower than normal to make speech more audible over music.
const INIT_SECONDARY_CHANNEL_VOLUME_TARGET_DUCKED: f64 = 0.2;

/// Rate at which volume fades between current and target levels.
/// Applied per-sample, so actual fade speed depends on sample rate.
/// At 44100 Hz, this gives approximately 4.4 seconds for a full 0-1 transition.
const VOLUME_FADE_RATE: f64 = 0.0001;

/// Threshold below which volume is snapped to target to avoid endless tiny adjustments.
const VOLUME_SNAP_THRESHOLD: f64 = 0.001;

pub type Sample = (i16, i16);
pub type MixerInput = mpsc::Receiver<Sample>;
pub type MixerOutput = watch::Receiver<Vec<Sample>>;

pub fn init(bus: &EventBus, mut sources: Vec<MixerInput>) -> Result<MixerOutput> {
    let (tx, rx) = watch::channel(Default::default());

    let bus = bus.clone();
    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let mut sample_send_count = 0;

        let mut current_secondary_volume = INIT_SECONDARY_CHANNEL_VOLUME_TARGET;
        let mut duck_secondary_channels = false;

        let mut adjusted_secondary_volume = INIT_SECONDARY_CHANNEL_VOLUME_TARGET;
        let mut adjusted_secondary_volume_ducked = INIT_SECONDARY_CHANNEL_VOLUME_TARGET_DUCKED;

        let mut subscriber = bus.subscribe();

        loop {
            while let Ok(event) = subscriber.try_recv() {
                match event {
                    Event::Mixer(MixerAction::DuckSecondaryChannels) => {
                        duck_secondary_channels = true;
                    }
                    Event::Mixer(MixerAction::UnduckSecondaryChannels) => {
                        duck_secondary_channels = false;
                    }
                    Event::Mixer(MixerAction::SetSecondaryChannelVolume(volume)) => {
                        adjusted_secondary_volume = volume;
                    }
                    Event::Mixer(MixerAction::SetSecondaryChannelDuckedVolume(volume)) => {
                        adjusted_secondary_volume_ducked = volume;
                    }
                    _ => {}
                }
            }

            let sleep_time = std::time::Duration::from_micros(
                ((TARGET_CHUNK_SIZE as f64 / SAMPLE_RATE as f64) * MICROS_PER_SECOND) as u64,
            );

            let expected_sent_samples =
                ((start_time.elapsed() + sleep_time).as_secs_f64() * SAMPLE_RATE as f64) as u64;

            let chunk_size = (expected_sent_samples - sample_send_count) as usize;
            let mut chunk = Vec::with_capacity(chunk_size);

            let target_secondary_volume = if duck_secondary_channels {
                adjusted_secondary_volume_ducked
            } else {
                adjusted_secondary_volume
            };

            for _ in 0..chunk_size {
                let mut left: i16 = 0;
                let mut right: i16 = 0;

                let secondary_volume_delta: f64 =
                    target_secondary_volume - current_secondary_volume;

                // Slowly fade secondary channels towards the target volume
                if secondary_volume_delta.abs() < VOLUME_SNAP_THRESHOLD {
                    current_secondary_volume = target_secondary_volume;
                } else if secondary_volume_delta.is_sign_positive() {
                    current_secondary_volume += VOLUME_FADE_RATE;
                } else {
                    current_secondary_volume -= VOLUME_FADE_RATE;
                };

                let mut first_source = true;
                for source in &mut sources {
                    let sample = source.recv().await.expect("Expected source to never close");
                    let volume = if first_source {
                        PRIMARY_CHANNEL_VOLUME
                    } else {
                        current_secondary_volume
                    };
                    left = left.saturating_add((sample.0 as f64 * volume) as i16);
                    right = right.saturating_add((sample.1 as f64 * volume) as i16);

                    first_source = false;
                }

                // Write the sample to the buffer
                chunk.push((left, right));
            }

            tx.send(chunk)
                .expect("Expected mixer channel to never close");
            sample_send_count += chunk_size as u64;

            tokio::time::sleep(sleep_time).await;
        }
    });

    Ok(rx)
}
