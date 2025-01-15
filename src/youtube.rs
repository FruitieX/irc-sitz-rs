use crate::playback::Song;
use anyhow::{Context, Result};
use std::path::Path;
use symphonia::core::io::MediaSource;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::io::ReadOnlySource;
use tokio::io::AsyncBufReadExt;
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
    // Spawn yt-dlp ourselves so we can capture stdout as a stream
    let mut cmd = tokio::process::Command::new("./yt-dlp")
        .arg(url)
        .arg("--no-progress")
        // this speeds up the process slightly but maybe reduces compatibility
        // .arg("--extractor-args")
        // .arg("youtube:player_client=tv")
        // until symphonia has opus support
        .arg("--format")
        .arg("bestaudio[ext=m4a]")
        .arg("-o")
        .arg("-")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = cmd.stdout.take().context("Failed to get yt-dlp stdout")?;
    let stderr = cmd.stderr.take().context("Failed to get yt-dlp stderr")?;

    tokio::spawn(async move {
        let output = cmd.wait_with_output().await.unwrap();
        if !output.status.success() {
            error!(
                "yt-dlp failed (exit code {code}): {stderr:?}",
                code = output.status.code().unwrap_or_default(),
                stderr = output.stderr
            );
        }
    });

    tokio::spawn(async move {
        // Print stderr to log
        let mut reader = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            debug!("yt-dlp stderr: {}", line);
        }
    });

    let sync_reader = tokio_util::io::SyncIoBridge::new(stdout);

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
        .extra_arg("--no-playlist")
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
