use anyhow::Result;
use byteorder::{LittleEndian, WriteBytesExt};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

// Import the tokio and hound crates
use hound::{SampleFormat, WavSpec};

use crate::constants::{BIT_DEPTH, CHANNELS, SAMPLE_RATE};
use crate::mixer::MixerOutput;

pub async fn start(source: MixerOutput) -> Result<()> {
    // Create a TCP listener that binds to the local address 127.0.0.1:7878
    let listener = TcpListener::bind("127.0.0.1:7878").await?;
    println!("Listening on 127.0.0.1:7878");

    loop {
        // Accept a connection and get the stream
        let (mut stream, addr) = listener.accept().await?;
        println!("Accepted connection from {}", addr);

        // Spawn a new task to handle incoming samples
        let mut source = source.clone();

        // Spawn a new task to handle the connection
        tokio::spawn(async move {
            // Create a WavSpec object to specify the audio properties
            let spec = WavSpec {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                bits_per_sample: BIT_DEPTH,
                sample_format: SampleFormat::Int,
            };

            // Write the wav header to the stream using the hound crate
            // This will allow players to recognize the stream as a wav file
            let header = spec.into_header_for_infinite_file();
            if let Err(e) = stream.write_all(&header[..]).await {
                eprintln!("Failed to write wav header: {}", e);
                return;
            }

            // Create a loop to write audio samples to the stream
            loop {
                source
                    .changed()
                    .await
                    .expect("Expected mixer channel to never close");

                let samples = source.borrow_and_update().clone();
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
