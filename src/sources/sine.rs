use tokio::sync::mpsc;

use crate::{mixer::MixerInput, constants::SAMPLE_RATE};

pub fn start(f: f64) -> MixerInput {
    let (tx, rx) = mpsc::channel(128);

    tokio::spawn(async move {
        // Initialize a phase variable to keep track of the sine wave phase
        let mut phase = 0.0;

        loop {
            // Generate a sine wave sample
            let sample: i16 = sine_wave(phase);

            // Write the sample to the buffer
            tx.send((sample, sample))
                .await
                .expect("Expected mixer channel to never close");

            // Increment the phase by the frequency divided by the sample rate
            phase += f / SAMPLE_RATE as f64;

            // Wrap the phase around 1.0 to avoid overflow
            phase %= 1.0;
        }
    });

    rx
}

const AMPLITUDE: f64 = 0.5; // 50% amplitude

// Define a helper function to generate a sine wave sample given a phase
fn sine_wave(phase: f64) -> i16 {
    // Convert the phase to radians and take the sine
    let sample = (phase * std::f64::consts::PI * 2.0).sin();
    // Scale the sample by the amplitude and the maximum value of i16
    let amplitude = i16::MAX as f64 * AMPLITUDE;
    // Cast the sample to i16 and return it
    (sample * amplitude) as i16
}
