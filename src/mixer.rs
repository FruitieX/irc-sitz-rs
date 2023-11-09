use anyhow::Result;
use tokio::sync::{mpsc, watch};

use crate::constants::SAMPLE_RATE;

const TARGET_CHUNK_SIZE: usize = 128;

pub type MixerInput = mpsc::Receiver<(i16, i16)>;
pub type MixerOutput = watch::Receiver<Vec<(i16, i16)>>;

pub async fn start(mut sources: Vec<MixerInput>) -> Result<MixerOutput> {
    let (tx, rx) = watch::channel(Default::default());

    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let mut sample_send_count = 0;

        loop {
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
                for source in &mut sources {
                    let sample = source.recv().await.unwrap();
                    left = left.saturating_add(sample.0);
                    right = right.saturating_add(sample.0);
                }

                // Write the sample to the buffer
                chunk.push((left, right));
            }

            tx.send(chunk)
                .expect("Expected mixer channel to never close");
            sample_send_count += chunk_size as u64;

            tokio::task::yield_now().await;
            tokio::time::sleep(sleep_time).await;
        }
    });

    Ok(rx)
}
