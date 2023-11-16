use crate::constants::{BIT_DEPTH, CHANNELS, SAMPLE_RATE};
use crate::mixer::MixerOutput;
use anyhow::Result;
use byteorder::{LittleEndian, WriteBytesExt};
use hound::{SampleFormat, WavSpec};
use std::net::SocketAddr;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

const HTTP: bool = false;
const LISTEN_ADDR: &str = "0.0.0.0:7878";

pub fn init(source: MixerOutput) {
    tokio::spawn(async move {
        // Create a TCP listener that binds to the configured address
        let listener = TcpListener::bind(LISTEN_ADDR).await.unwrap();
        info!("Listening on {LISTEN_ADDR}");

        loop {
            // Accept a connection and get the stream
            let result = accept(&listener, &source).await;

            match result {
                Ok(addr) => info!("Accepted connection from {}", addr),
                Err(e) => error!("Failed to accept connection: {}", e),
            }
        }
    });
}

async fn accept(listener: &TcpListener, source: &MixerOutput) -> Result<SocketAddr> {
    let (mut stream, addr) = listener.accept().await?;

    stream.set_nodelay(true)?;

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

        if HTTP {
            let cors_headers = "Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET\r\nAccess-Control-Allow-Headers: *\r\n";
            // Write HTTP headers to the stream
            let header =
                format!("HTTP/1.1 200 OK\r\n{cors_headers}Content-Type: audio/wav\r\n\r\n");
            if let Err(e) = stream.write_all(header.as_bytes()).await {
                warn!("Failed to write HTTP header to stream: {}", e);
                return;
            };
        }

        // Write the wav header to the stream using the hound crate
        // This will allow players to recognize the stream as a wav file
        let header = spec.into_header_for_infinite_file();
        if let Err(e) = stream.write_all(&header[..]).await {
            warn!("Failed to write wav header to stream: {}", e);
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
                WriteBytesExt::write_i16::<LittleEndian>(&mut wav_data, left).ok();
                WriteBytesExt::write_i16::<LittleEndian>(&mut wav_data, right).ok();
            }

            if let Err(e) = stream.write_all(wav_data.as_slice()).await {
                warn!("Failed to write sample to stream: {}", e);
                break;
            }
        }
    });

    Ok(addr)
}
