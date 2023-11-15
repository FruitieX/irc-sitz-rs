use crate::{
    constants::SAMPLE_RATE,
    event::{Event, EventBus},
};
use anyhow::Result;
use tokio::sync::{mpsc, watch};

const TARGET_CHUNK_SIZE: usize = 128;

#[derive(Clone, Debug)]
pub enum MixerAction {
    DuckSecondaryChannels,
    UnduckSecondaryChannels,
    SetSecondaryChannelVolume(f64),
    SetSecondaryChannelDuckedVolume(f64),
}

const PRIMARY_CHANNEL_VOLUME: f64 = 1.25;
const INIT_SECONDARY_CHANNEL_VOLUME_TARGET: f64 = 0.75;
const INIT_SECONDARY_CHANNEL_VOLUME_TARGET_DUCKED: f64 = 0.2;

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
                ((TARGET_CHUNK_SIZE as f64 / SAMPLE_RATE as f64) * 1_000_000.0) as u64,
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
                let correction_rate = 0.0001;
                if secondary_volume_delta.abs() < 0.001 {
                    current_secondary_volume = target_secondary_volume;
                } else if secondary_volume_delta.is_sign_positive() {
                    current_secondary_volume += correction_rate;
                } else {
                    current_secondary_volume -= correction_rate;
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
