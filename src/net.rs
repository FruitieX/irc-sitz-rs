use anyhow::Result;
use byteorder::{LittleEndian, WriteBytesExt};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

// Import the tokio and hound crates
use hound::{SampleFormat, WavSpec};

use crate::mixer::MixerChannel;

// Define some constants for the audio parameters
const SAMPLE_RATE: u32 = 44100; // 44.1 kHz sample rate
const BIT_DEPTH: u16 = 16; // 16 bits per sample
const CHANNELS: u16 = 2; // Stereo channel

pub async fn start(mixer_channel: MixerChannel) -> Result<()> {
    // Create a TCP listener that binds to the local address 127.0.0.1:7878
    let listener = TcpListener::bind("127.0.0.1:7878").await?;
    println!("Listening on 127.0.0.1:7878");

    loop {
        // Accept a connection and get the stream
        let (mut stream, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);

        // Spawn a new task to handle incoming samples
        let mut mixer_channel = mixer_channel.clone();

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

            // Create a loop to write audio samples to the stream
            loop {
                mixer_channel
                    .changed()
                    .await
                    .expect("Expected mixer channel to never close");

                let samples = mixer_channel.borrow_and_update().clone();

                let mut wav_data: Vec<u8> = Vec::with_capacity(samples.len() * 2);

                for (left, right) in samples {
                    WriteBytesExt::write_i16::<LittleEndian>(&mut wav_data, left).unwrap();
                    WriteBytesExt::write_i16::<LittleEndian>(&mut wav_data, right).unwrap();
                }

                if let Err(e) = stream.write_all(wav_data.as_slice()).await {
                    eprintln!("Failed to write sample: {}", e);
                    break;
                }
            }
        });
    }
}
