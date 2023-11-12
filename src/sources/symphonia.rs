use crate::{
    buffer::PlaybackBuffer,
    bus::{Event, EventBus},
    constants::SAMPLE_RATE,
    mixer::{MixerInput, Sample},
    playback::PlaybackAction,
};
use anyhow::{Context, Result};
use futures::TryStreamExt;
use itertools::Itertools;
use std::path::Path;
use std::{fs::File, sync::Arc};
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::{audio::SampleBuffer, io::MediaSource};
use symphonia::core::{codecs::DecoderOptions, io::ReadOnlySource};
use tokio::sync::{mpsc, Mutex};
use tokio_util::io::StreamReader;
use youtube_dl::{download_yt_dlp, YoutubeDl};

#[derive(Clone, Debug)]
pub enum SymphoniaAction {
    PlayFile { file_path: String },
    PlayYtUrl { url: String },
    Pause,
    Resume,
}

pub async fn init(bus: &EventBus) -> Result<MixerInput> {
    let yt_dlp_binary_exists = tokio::task::spawn_blocking(|| Path::new("./yt-dlp").exists())
        .await
        .unwrap();

    if !yt_dlp_binary_exists {
        info!("Downloading yt-dlp binary");
        download_yt_dlp(".").await?;
    }

    let (tx, rx) = mpsc::channel(128);
    let playback_buf = Arc::new(Mutex::new(PlaybackBuffer::default()));

    start_decode_event_loop(bus.clone(), playback_buf.clone());
    start_emit_sample_loop(bus.clone(), tx, playback_buf);

    Ok(rx)
}

fn start_decode_event_loop(bus: EventBus, playback_buf: Arc<Mutex<PlaybackBuffer>>) {
    tokio::spawn(async move {
        // Check for any new events on the bus
        let mut bus_tx = bus.subscribe();

        loop {
            let event = bus_tx.recv().await.unwrap();

            if let Event::Symphonia(action) = event {
                let playback_buf = playback_buf.clone();
                tokio::spawn(async move {
                    let result = handle_incoming_event(action, playback_buf).await;

                    if let Err(e) = result {
                        error!("Error while handling incoming event: {:?}", e);
                    }
                });
            }
        }
    });
}

async fn handle_incoming_event(
    action: SymphoniaAction,
    playback_buf: Arc<Mutex<PlaybackBuffer>>,
) -> Result<()> {
    match action {
        SymphoniaAction::PlayFile { file_path } => {
            {
                let mut playback_buf = playback_buf.lock().await;
                playback_buf.clear();
            }

            {
                let playback_buf = playback_buf.clone();

                // Create a media source. Note that the MediaSource trait is automatically implemented for File,
                // among other types.
                let source = Box::new(File::open(Path::new(&file_path))?);
                let mss = MediaSourceStream::new(source, Default::default());
                tokio::task::spawn_blocking(|| decode_source(mss, playback_buf)).await??
            };

            info!("Finished decoding audio from {file_path}");

            {
                let mut playback_buf = playback_buf.lock().await;
                playback_buf.set_eof(true);
            }
        }
        SymphoniaAction::PlayYtUrl { url } => {
            {
                let mut playback_buf = playback_buf.lock().await;
                playback_buf.clear();
            }

            {
                let playback_buf = playback_buf.clone();

                let mss = get_yt_media_source_stream(url.clone()).await?;
                tokio::task::spawn_blocking(|| decode_source(mss, playback_buf)).await??
            };

            info!("Finished decoding audio from {url}");

            {
                let mut playback_buf = playback_buf.lock().await;
                playback_buf.set_eof(true);
            }
        }
        SymphoniaAction::Pause => {
            let mut playback_buf = playback_buf.lock().await;
            playback_buf.set_paused(true);
        }
        SymphoniaAction::Resume => {
            let mut playback_buf = playback_buf.lock().await;
            playback_buf.set_paused(false);
        }
    }

    Ok(())
}

fn start_emit_sample_loop(
    bus: EventBus,
    tx: mpsc::Sender<Sample>,
    playback_buf: Arc<Mutex<PlaybackBuffer>>,
) {
    tokio::spawn(async move {
        loop {
            let (sample, decoder_hit_eof) = {
                let mut playback_buf = playback_buf.lock().await;
                (playback_buf.next_sample(), playback_buf.is_eof())
            };

            if sample.is_none() && decoder_hit_eof {
                let mut playback_buf = playback_buf.lock().await;
                playback_buf.clear();
                bus.send(Event::Playback(PlaybackAction::EndOfSong))
                    .unwrap();
            }

            tx.send(sample.unwrap_or_default())
                .await
                .expect("Expected mixer channel to never close");
        }
    });
}

pub async fn get_yt_media_source_stream(url: String) -> Result<MediaSourceStream> {
    let output = YoutubeDl::new(url)
        .youtube_dl_path("./yt-dlp")
        .extract_audio(true)
        // until symphonia has opus support
        .format("bestaudio[ext=m4a]")
        .run_async()
        .await?
        .into_single_video();

    let video = output.context("No video found")?;

    debug!(
        "Found video {:?} with duration {:?}",
        &video.title, &video.duration
    );

    let url = video.url.context("No URL found in yt-dlp JSON!")?;
    let stream = reqwest::get(&url)
        .await?
        .bytes_stream()
        .map_err(|e| futures::io::Error::new(std::io::ErrorKind::Other, e));

    let read = StreamReader::new(stream);

    // let reader = BufReader::new(stream.into_async_read());
    let sync_reader = tokio_util::io::SyncIoBridge::new(read);

    let source = Box::new(ReadOnlySource::new(sync_reader)) as Box<dyn MediaSource>;

    Ok(MediaSourceStream::new(source, Default::default()))
}

pub fn decode_source(
    mss: MediaSourceStream,
    playback_buf: Arc<Mutex<PlaybackBuffer>>,
) -> Result<()> {
    // Create a hint to help the format registry guess what format reader is appropriate. In this
    // example we'll leave it empty.
    let hint = Hint::new();

    // Use the default options when reading and decoding.
    let format_opts: FormatOptions = Default::default();
    let metadata_opts: MetadataOptions = Default::default();
    let decoder_opts: DecoderOptions = Default::default();

    // Probe the media source stream for a format.
    let probed =
        symphonia::default::get_probe().format(&hint, mss, &format_opts, &metadata_opts)?;

    // Get the format reader yielded by the probe operation.
    let mut format = probed.format;

    // Get the default track.
    let track = format
        .default_track()
        .context("Could not find any tracks in file")?;

    // Create a decoder for the track.
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &decoder_opts)?;

    // Store the track identifier, we'll use it to filter packets.
    let track_id = track.id;

    let mut sample_count = 0;
    let mut sample_buf = None;

    loop {
        // Get the next packet from the format reader.
        let packet = format.next_packet();

        // Symphonia seems to return UnexpectedEof even if the EOF was expected,
        // handle this gracefully
        let packet = match &packet {
            Err(symphonia::core::errors::Error::IoError(e))
                if e.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                return Ok(());
            }
            _ => packet?,
        };

        // If the packet does not belong to the selected track, skip it.
        if packet.track_id() != track_id {
            continue;
        }

        // Decode the packet into audio samples
        let audio_buf = decoder.decode(&packet)?;

        // The decoded audio samples may now be accessed via the audio buffer if per-channel
        // slices of samples in their native decoded format is desired. Use-cases where
        // the samples need to be accessed in an interleaved order or converted into
        // another sample format, or a byte buffer is required, are covered by copying the
        // audio buffer into a sample buffer or raw sample buffer, respectively. In the
        // example below, we will copy the audio buffer into a sample buffer in an
        // interleaved order while also converting to a i16 sample format.

        // If this is the *first* decoded packet, create a sample buffer matching the
        // decoded audio buffer format.
        if sample_buf.is_none() {
            // Get the audio buffer specification.
            let spec = *audio_buf.spec();

            // Get the capacity of the decoded buffer. Note: This is capacity, not length!
            let duration = audio_buf.capacity() as u64;

            // Create the i16 sample buffer.
            sample_buf = Some(SampleBuffer::<i16>::new(duration, spec));
        }

        // Copy the decoded audio buffer into the sample buffer in an interleaved format.
        if let Some(buf) = &mut sample_buf {
            buf.copy_interleaved_ref(audio_buf);

            // The samples may now be access via the `samples()` function.
            let samples = buf.samples();
            sample_count += samples.len() / 2;
            trace!(
                "\rDecoded {:.2} seconds",
                sample_count as f64 / SAMPLE_RATE as f64
            );

            let samples: Vec<Sample> = samples.iter().copied().tuples().collect();

            // Write samples to the buffer
            {
                let mut playback_buf = playback_buf.blocking_lock();
                playback_buf.push_samples(samples);
            }
        }
    }
}
