use anyhow::Result;
use tokio::sync::watch;

// Define some constants for the audio parameters
const SAMPLE_RATE: u32 = 44100; // 44.1 kHz sample rate
const BIT_DEPTH: u16 = 16; // 16 bits per sample
const CHANNELS: u16 = 2; // Stereo channel
const FREQUENCY: f64 = 440.0; // 440 Hz sine wave frequency
const AMPLITUDE: f64 = 0.5; // 50% amplitude

const TARGET_CHUNK_SIZE: usize = 128;

pub type MixerChannel = watch::Receiver<Vec<(i16, i16)>>;

pub async fn start() -> Result<MixerChannel> {
    let (tx, rx) = watch::channel(Default::default());

    tokio::spawn(async move {
        let start_time = std::time::Instant::now();
        let mut sample_send_count = 0;

        // Initialize a phase variable to keep track of the sine wave phase
        let mut phase = 0.0;

        loop {
            let sleep_time = std::time::Duration::from_micros(
                ((TARGET_CHUNK_SIZE as f64 / SAMPLE_RATE as f64) * 1_000_000.0) as u64,
            );

            let expected_sent_samples =
                ((start_time.elapsed() + sleep_time).as_secs_f64() * SAMPLE_RATE as f64) as u64;

            let chunk_size = (expected_sent_samples - sample_send_count) as usize;
            let mut chunk = Vec::with_capacity(chunk_size);

            for _ in 0..chunk_size {
                // Generate a sine wave sample
                let sample: i16 = sine_wave(phase);

                // Write the sample to the buffer as little-endian
                chunk.push((sample, sample));

                // Increment the phase by the frequency divided by the sample rate
                phase += FREQUENCY / SAMPLE_RATE as f64;

                // Wrap the phase around 1.0 to avoid overflow
                phase %= 1.0;
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

// Define a helper function to generate a sine wave sample given a phase
fn sine_wave(phase: f64) -> i16 {
    // Convert the phase to radians and take the sine
    let sample = (phase * std::f64::consts::PI * 2.0).sin();
    // Scale the sample by the amplitude and the maximum value of i16
    let amplitude = i16::MAX as f64 * AMPLITUDE;
    // Cast the sample to i16 and return it
    (sample * amplitude) as i16
}
