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
    info!("Starting yt-dlp stream for: {url}");

    // Spawn yt-dlp ourselves so we can capture stdout as a stream
    let mut cmd = tokio::process::Command::new("./yt-dlp")
        .arg(url.clone())
        .arg("--no-progress")
        // this speeds up the process slightly but maybe reduces compatibility
        // .arg("--extractor-args")
        // .arg("youtube:player_client=tv")
        // until symphonia has opus support
        // 2026 yt-dlp fix:
        // https://github.com/yt-dlp/yt-dlp/issues/15712
        .arg("--extractor-args")
        .arg("youtube:player_client=default,ios,-android_sdkless;formats=missing_pot")
        .arg("--format")
        .arg("ba[protocol=m3u8_native]/b[protocol=m3u8_native]")
        // .arg("bestaudio[ext=m4a]")
        .arg("-o")
        .arg("-")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = cmd.stdout.take().context("Failed to get yt-dlp stdout")?;
    let stderr = cmd.stderr.take().context("Failed to get yt-dlp stderr")?;

    let url_for_log = url.clone();
    tokio::spawn(async move {
        let output = cmd.wait_with_output().await.unwrap();
        if output.status.success() {
            info!("yt-dlp stream completed successfully for: {url_for_log}");
        } else {
            let code = output.status.code().unwrap_or_default();
            // Exit code 1 with empty stderr typically means the stream was cancelled (e.g. skip)
            if code == 1 && output.stderr.is_empty() {
                info!("yt-dlp stream cancelled for: {url_for_log}");
            } else {
                error!(
                    "yt-dlp failed (exit code {code}): {stderr:?}",
                    stderr = output.stderr
                );
            }
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
    info!("Fetching song info for: {url_or_search_terms}");

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

    info!("Found song: {title} by {channel} (id: {id}, duration: {duration}s)");

    Ok(Song {
        id,
        url,
        title,
        channel,
        duration,
        queued_by,
    })
}

/// Search YouTube and return multiple results for autocomplete
pub async fn search_yt(query: &str, max_results: usize) -> Result<Vec<(String, String)>> {
    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    info!("Searching YouTube for: {query} (max {max_results} results)");

    let output = YoutubeDl::new(format!("ytsearch{max_results}:{query}"))
        .youtube_dl_path("./yt-dlp")
        .extra_arg("--flat-playlist")
        .extra_arg("--no-playlist")
        .run_async()
        .await?;

    let playlist = match output.into_playlist() {
        Some(p) => p,
        None => return Ok(vec![]),
    };

    let entries = playlist.entries.unwrap_or_default();

    Ok(entries
        .into_iter()
        .filter_map(|v| {
            let title = v.title?;
            let id = v.id;
            let url = format!("https://youtu.be/{id}");
            // Truncate title if too long for Discord's 100 char limit
            let display = if title.len() > 95 {
                format!("{}...", &title[..92])
            } else {
                title
            };
            Some((display, url))
        })
        .collect())
}
