use crate::playback::Song;
use anyhow::{Context, Result};
use futures::TryStreamExt;
use std::path::Path;
use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::io::ReadOnlySource;
use tokio_util::io::StreamReader;
use youtube_dl::{download_yt_dlp, YoutubeDl};

pub async fn init() -> anyhow::Result<()> {
    let yt_dlp_binary_exists =
        tokio::task::spawn_blocking(|| Path::new("./yt-dlp").exists()).await?;

    if !yt_dlp_binary_exists {
        info!("Downloading yt-dlp binary");
        download_yt_dlp(".").await?;
    }

    Ok(())
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

pub async fn get_yt_song_info(url_or_search_terms: String, queued_by: String) -> Result<Song> {
    let output = YoutubeDl::new(url_or_search_terms.clone())
        .youtube_dl_path("./yt-dlp")
        .extract_audio(true)
        // until symphonia has opus support
        .format("bestaudio[ext=m4a]")
        .extra_arg("--default-search")
        .extra_arg("ytsearch")
        .run_async()
        .await?;

    let single_video = output.clone().into_single_video();
    let first_match = single_video.or_else(|| {
        let playlist = output.into_playlist()?;
        let entries = playlist.entries?;
        entries.first().cloned()
    });

    let video = first_match.context("No video found")?;
    let id = video.id;
    let url = format!("https://youtu.be/{}", id);
    let title = video.title.context("No title found in yt-dlp JSON!")?;
    let channel = video.channel.context("No channel found in yt-dlp JSON!")?;
    let duration = video
        .duration
        .context("No duration found in yt-dlp JSON!")?
        .as_u64()
        .context("Invalid duration in yt-dlp JSON")?;

    Ok(Song {
        id,
        url,
        title,
        channel,
        duration,
        queued_by,
    })
}
