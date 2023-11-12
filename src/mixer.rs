use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::{
    bus::{Event, EventBus},
    constants::SAMPLE_RATE,
};

const TARGET_CHUNK_SIZE: usize = 128;

#[derive(Clone, Debug)]
pub enum MixerAction {
    SetSecondaryChannelsVolume { volume: f64 },
}

pub type Sample = (i16, i16);
pub type MixerInput = mpsc::Receiver<Sample>;
pub type MixerOutput = watch::Receiver<Vec<Sample>>;

pub fn init(bus: &EventBus, mut sources: Vec<MixerInput>) -> Result<MixerOutput> {
    let (tx, rx) = watch::channel(Default::default());

    let bus = bus.clone();
    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let mut sample_send_count = 0;

        let mut secondary_volume = 1.0;
        let mut secondary_volume_target = 1.0;

        let mut bus = bus.subscribe();

        loop {
            while let Ok(event) = bus.try_recv() {
                if let Event::Mixer(MixerAction::SetSecondaryChannelsVolume { volume }) = event {
                    secondary_volume_target = volume;
                }
            }

            let sleep_time = std::time::Duration::from_micros(
                ((TARGET_CHUNK_SIZE as f64 / SAMPLE_RATE as f64) * 1_000_000.0) as u64,
            );

            let expected_sent_samples =
                ((start_time.elapsed() + sleep_time).as_secs_f64() * SAMPLE_RATE as f64) as u64;

            let chunk_size = (expected_sent_samples - sample_send_count) as usize;
            let mut chunk = Vec::with_capacity(chunk_size);

            for _ in 0..chunk_size {
                let mut left: i16 = 0;
                let mut right: i16 = 0;

                let secondary_volume_delta = secondary_volume_target - secondary_volume;

                // Slowly fade secondary channels towards the target volume
                let correction_rate = 0.0001;
                if secondary_volume_delta.abs() < 0.001 {
                    secondary_volume = secondary_volume_target;
                } else if secondary_volume_delta.is_sign_positive() {
                    secondary_volume += correction_rate;
                } else {
                    secondary_volume -= correction_rate;
                };

                let mut first_source = true;
                for source in &mut sources {
                    let sample = source.recv().await.expect("Expected source to never close");
                    let volume = if first_source { 1.0 } else { secondary_volume };
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
