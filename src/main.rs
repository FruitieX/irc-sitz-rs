use async_speed_limit::Limiter;
// Import the tokio and hound crates
use hound::{SampleFormat, WavSpec};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

// Define some constants for the audio parameters
const SAMPLE_RATE: u32 = 44100; // 44.1 kHz sample rate
const BIT_DEPTH: u16 = 16; // 16 bits per sample
const CHANNELS: u16 = 2; // Stereo channel
const FREQUENCY: f64 = 440.0; // 440 Hz sine wave frequency
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a TCP listener that binds to the local address 127.0.0.1:7878
    let listener = TcpListener::bind("127.0.0.1:7878").await?;
    println!("Listening on 127.0.0.1:7878");

    // Create a loop to accept incoming connections
    loop {
        // Accept a connection and get the stream
        let (mut stream, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);

        // Spawn a new task to handle the connection
        tokio::spawn(async move {
            // Create a WavSpec object to specify the audio properties
            let spec = WavSpec {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: BIT_DEPTH,
                sample_format: SampleFormat::Int,
            };

            let v = spec.into_header_for_infinite_file();

            // Write the wav header to the stream using the hound crate
            // This will allow the VLC player to recognize the stream as a wav file
            if let Err(e) = stream.write_all(&v[..]).await {
                eprintln!("Failed to write wav header: {}", e);
                return;
            }

            // Initialize a phase variable to keep track of the sine wave phase
            let mut phase = 0.0;

            let limiter = <Limiter>::new(SAMPLE_RATE as f64 * (BIT_DEPTH as f64 / 8.0) * CHANNELS as f64);
            let mut stream = limiter.limit(stream);

            // Create a loop to write audio samples to the stream
            loop {
                // Generate a sine wave sample using the phase
                let sample: i16 = sine_wave(phase);

                // Write the samples to the stream as a little-endian byte array
                // This will match the wav format specification

                // Left channel
                if let Err(e) = stream.write_all(&sample.to_le_bytes()).await {
                    eprintln!("Failed to write sample: {}", e);
                    break;
                }

                // Right channel
                if let Err(e) = stream.write_all(&sample.to_le_bytes()).await {
                    eprintln!("Failed to write sample: {}", e);
                    break;
                }

                // Increment the phase by the frequency divided by the sample rate
                phase += FREQUENCY / SAMPLE_RATE as f64;

                // Wrap the phase around 1.0 to avoid overflow
                phase %= 1.0;
            }
        });
    }
}
