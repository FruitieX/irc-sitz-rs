//! Discord integration for the sitzning bot.
//!
//! This module provides Discord support including:
//! - Message mirroring between Discord and IRC
//! - Slash commands for all bot commands
//! - Rich embeds for queue status, song info, etc.
//! - Reaction-based bingo (react to signal you found the song)
//! - Song request autocomplete dropdown
//! - Voice channel audio streaming

use crate::{
    config::{Config, DiscordConfig},
    event::{Event, EventBus},
    message::{CountdownValue, MessageAction, NowPlayingInfo, Platform, RichContent},
    mixer::Mixer,
    playback::{PlaybackAction, SharedPlayback, Song, MAX_SONG_DURATION},
    songbook::SongbookSong,
    songleader::{Mode, SharedSongleader, SongleaderAction},
    sources::{
        espeak::{Priority, TextToSpeechAction},
        Sample,
    },
    youtube::{get_yt_song_info, search_yt},
};
use anyhow::Result;
use poise::serenity_prelude::{
    self as serenity, ChannelId, CreateEmbed, CreateMessage, EditMessage, GuildId, Http,
    ReactionType,
};
use songbird::{input::Input, tracks::Track, SerenityInit};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::RwLock;

/// Discord bot state shared across handlers
struct BotState {
    bus: EventBus,
    config: Config,
    channel_id: ChannelId,
    /// Message ID of the current bingo announcement (for reaction tracking)
    bingo_message_id: Option<serenity::MessageId>,
    /// Message ID of the current now-playing message (for progress updates and skip reactions)
    now_playing_message_id: Option<serenity::MessageId>,
    /// Current song ID being played (to detect song changes)
    current_song_id: Option<String>,
    /// Users who have voted to skip the current song
    skip_votes: HashSet<String>,
    /// HTTP client for sending messages (set when bot is ready)
    http: Option<Arc<Http>>,
    /// Pull-based mixer for voice channel streaming
    mixer: Arc<StdMutex<Mixer>>,
    /// Shared playback state for reading queue/progress info
    playback: SharedPlayback,
    /// Shared songleader state for reading mode info
    songleader: SharedSongleader,
}

type Context<'a> = poise::Context<'a, Arc<RwLock<BotState>>, anyhow::Error>;

// ============================================================================
// Voice Audio Source
// ============================================================================

use songbird::input::RawAdapter;
use std::{
    io::{Read, Seek, SeekFrom},
    sync::Mutex as StdMutex,
};
use symphonia::core::io::MediaSource;

/// Pull-based audio source that reads from Mixer on-demand.
/// Songbird's audio thread calls Read::read() which pulls mixed audio from source buffers.
struct MixerAudioSource {
    mixer: Arc<StdMutex<Mixer>>,
}

impl MixerAudioSource {
    fn new(mixer: Arc<StdMutex<Mixer>>) -> Self {
        Self { mixer }
    }
}

/// Convert i16 stereo samples to f32 bytes directly into output buffer
fn samples_to_f32_bytes_into(samples: &[Sample], buf: &mut [u8]) -> usize {
    let mut offset = 0;
    for (left, right) in samples {
        let left_f32 = *left as f32 / 32768.0;
        let right_f32 = *right as f32 / 32768.0;

        if offset + 8 <= buf.len() {
            buf[offset..offset + 4].copy_from_slice(&left_f32.to_le_bytes());
            buf[offset + 4..offset + 8].copy_from_slice(&right_f32.to_le_bytes());
            offset += 8;
        }
    }
    offset
}

impl Read for MixerAudioSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Calculate how many samples we need (f32 stereo = 8 bytes per sample)
        let samples_needed = buf.len() / 8;

        // Pull samples from the mixer
        let samples = if let Ok(mut mixer) = self.mixer.lock() {
            mixer.pull_samples(samples_needed)
        } else {
            vec![(0i16, 0i16); samples_needed]
        };

        // Convert to f32 bytes
        let bytes_written = samples_to_f32_bytes_into(&samples, buf);

        // Pad with silence if needed
        if bytes_written < buf.len() {
            buf[bytes_written..].fill(0);
        }

        Ok(buf.len())
    }
}

impl Seek for MixerAudioSource {
    fn seek(&mut self, _pos: SeekFrom) -> std::io::Result<u64> {
        // Live audio source doesn't support seeking
        Ok(0)
    }
}

impl MediaSource for MixerAudioSource {
    fn is_seekable(&self) -> bool {
        false
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

/// Create a songbird Input from the mixer
fn create_voice_input(mixer: Arc<StdMutex<Mixer>>) -> Input {
    let source = MixerAudioSource::new(mixer);
    let adapter = RawAdapter::new(source, 48000, 2);

    adapter.into()
}

/// Number of votes required to skip a song
const SKIP_VOTES_REQUIRED: usize = 4;

/// Initialize the Discord bot
pub async fn init(
    bus: &EventBus,
    config: &Config,
    discord_config: &DiscordConfig,
    mixer: Arc<StdMutex<Mixer>>,
    playback: SharedPlayback,
    songleader: SharedSongleader,
) -> Result<()> {
    let channel_id = ChannelId::new(discord_config.discord_channel_id);
    let guild_id = GuildId::new(discord_config.discord_guild_id);
    let voice_channel_id = discord_config.discord_voice_channel_id.map(ChannelId::new);
    let token = discord_config.discord_token.clone();

    let state = Arc::new(RwLock::new(BotState {
        bus: bus.clone(),
        config: config.clone(),
        channel_id,
        bingo_message_id: None,
        now_playing_message_id: None,
        current_song_id: None,
        skip_votes: HashSet::new(),
        http: None,
        mixer: mixer.clone(),
        playback,
        songleader,
    }));

    // Start the outgoing message handler
    start_outgoing_message_handler(bus.clone(), state.clone());

    // Start the progress bar update loop
    start_progress_update_loop(state.clone());

    let state_for_setup = state.clone();
    let mixer_for_setup = mixer;

    // Build the poise framework with slash commands
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                play(),
                queue(),
                remove(),
                speak(),
                request(),
                tempo(),
                bingo(),
                skal(),
                list_songs(),
                help(),
                song_admin(),
                music_admin(),
                voice_admin(),
            ],
            event_handler: |ctx, event, _framework, data| Box::pin(event_handler(ctx, event, data)),
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let state = state_for_setup.clone();
            let mixer = mixer_for_setup.clone();
            let voice_channel_id = voice_channel_id;
            Box::pin(async move {
                // Store the HTTP client for message sending
                {
                    let mut state_write = state.write().await;
                    state_write.http = Some(ctx.http.clone());
                }

                // Register commands for the specific guild (faster updates during development)
                poise::builtins::register_in_guild(ctx, &framework.options().commands, guild_id)
                    .await?;

                // Auto-join voice channel if configured
                if let Some(vc_id) = voice_channel_id {
                    let manager = songbird::get(ctx)
                        .await
                        .expect("Songbird Voice client placed in at initialisation.");

                    match manager.join(guild_id, vc_id).await {
                        Ok(handler_lock) => {
                            let mut handler = handler_lock.lock().await;
                            let input = create_voice_input(mixer.clone());
                            let track = Track::new(input);
                            handler.play_only(track);
                            info!("Auto-joined voice channel {}", vc_id);
                        }
                        Err(e) => {
                            error!("Failed to auto-join voice channel: {:?}", e);
                        }
                    }
                }

                info!("Discord bot ready and commands registered!");
                Ok(state)
            })
        })
        .build();

    // Build and start the serenity client
    let intents = serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MESSAGE_REACTIONS
        | serenity::GatewayIntents::GUILD_VOICE_STATES;

    let client = serenity::ClientBuilder::new(&token, intents)
        .framework(framework)
        .register_songbird()
        .await?;

    // Spawn the Discord client in a separate task
    tokio::spawn(async move {
        let mut client = client;
        if let Err(e) = client.start().await {
            error!("Discord client error: {:?}", e);
        }
    });

    Ok(())
}

/// Handle Discord events (messages, reactions, etc.)
async fn event_handler(
    _ctx: &serenity::Context,
    event: &serenity::FullEvent,
    data: &Arc<RwLock<BotState>>,
) -> Result<(), anyhow::Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            let state = data.read().await;

            // Only process messages from the configured channel
            if new_message.channel_id != state.channel_id {
                return Ok(());
            }

            // Ignore bot messages
            if new_message.author.bot {
                return Ok(());
            }

            // Mirror the message to other platforms
            state.bus.send_message(MessageAction::Mirror {
                username: new_message.author.name.clone(),
                text: new_message.content.clone(),
                source: Platform::Discord,
            });

            // Also try to parse as a text command (for users who type !commands)
            if let Some(action) = text_message_to_action(
                &new_message.content,
                &new_message.author.name,
                &state.config,
            )
            .await
            {
                info!(
                    "Discord text command from {}: {}",
                    new_message.author.name, new_message.content
                );
                state.bus.send(action);
            }
        }
        serenity::FullEvent::ReactionAdd { add_reaction } => {
            let state = data.read().await;

            // Check if this is a reaction to the bingo message
            if let Some(bingo_msg_id) = state.bingo_message_id {
                if add_reaction.message_id == bingo_msg_id {
                    // Get the user who reacted
                    if let Some(user) = &add_reaction.member {
                        let nick = user.nick.clone().unwrap_or_else(|| user.user.name.clone());
                        info!("Discord bingo reaction from {nick}");
                        state
                            .bus
                            .send(Event::Songleader(SongleaderAction::Bingo { nick }));
                    }
                }
            }

            // Check if this is a skip reaction on the now-playing message
            if let Some(np_msg_id) = state.now_playing_message_id {
                if add_reaction.message_id == np_msg_id {
                    // Check if it's the skip emoji
                    if matches!(&add_reaction.emoji, ReactionType::Unicode(s) if s == "⏭️") {
                        if let Some(user) = &add_reaction.member {
                            // Ignore bot reactions
                            if user.user.bot {
                                return Ok(());
                            }

                            let nick = user.nick.clone().unwrap_or_else(|| user.user.name.clone());
                            drop(state); // Release read lock before write

                            let mut state_write = data.write().await;
                            state_write.skip_votes.insert(nick.clone());
                            let vote_count = state_write.skip_votes.len();

                            info!("Skip vote from {nick}: {vote_count}/{SKIP_VOTES_REQUIRED}");

                            if vote_count >= SKIP_VOTES_REQUIRED {
                                // Check if playback should_play is true (don't skip if paused)
                                let should_skip = {
                                    let playback = state_write.playback.read().await;
                                    playback.state.should_play && playback.state.is_playing
                                };

                                if should_skip {
                                    info!("Skip vote threshold reached, skipping song");
                                    state_write.bus.send(Event::Playback(PlaybackAction::Next));
                                    state_write.skip_votes.clear();
                                } else {
                                    info!("Skip vote threshold reached but playback is paused, not skipping");
                                }
                            }
                        }
                    }
                }
            }
        }
        _ => {}
    }

    Ok(())
}

/// Parse a text message as a bot command (for users who prefer !commands)
async fn text_message_to_action(text: &str, nick: &str, config: &Config) -> Option<Event> {
    let mut cmd_split = text.split_whitespace();
    let cmd = cmd_split.next()?;

    match cmd {
        "!play" | "!p" => {
            let words: Vec<&str> = cmd_split.collect();
            let url_or_search_terms = words.join(" ");

            if config.songbook.songbook_re.is_match(&url_or_search_terms) {
                return Some(Event::Songleader(SongleaderAction::RequestSongUrl {
                    url: url_or_search_terms,
                    queued_by: nick.to_string(),
                }));
            }

            if url_or_search_terms.is_empty() {
                return None;
            }

            let song = get_yt_song_info(url_or_search_terms, nick.to_string()).await;

            match song {
                Ok(song) if song.duration > MAX_SONG_DURATION.as_secs() => None,
                Ok(song) => Some(Event::Playback(PlaybackAction::Enqueue { song })),
                Err(_) => None,
            }
        }
        "!tempo" | "tempo" => Some(Event::Songleader(SongleaderAction::Tempo {
            nick: nick.to_string(),
        })),
        "!bingo" | "bingo" => Some(Event::Songleader(SongleaderAction::Bingo {
            nick: nick.to_string(),
        })),
        "!skål" | "skål" => Some(Event::Songleader(SongleaderAction::Skål)),
        "!help" => Some(Event::Songleader(SongleaderAction::Help)),
        "!ls" => Some(Event::Songleader(SongleaderAction::ListSongs)),
        _ => None,
    }
}

/// Updates the now-playing message with current progress every 10 seconds
fn start_progress_update_loop(state: Arc<RwLock<BotState>>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));

        loop {
            interval.tick().await;

            let state_guard = state.read().await;

            // Skip if bot not ready or no now_playing message
            let (http, channel_id, msg_id) =
                match (&state_guard.http, state_guard.now_playing_message_id) {
                    (Some(http), Some(msg_id)) => (http.clone(), state_guard.channel_id, msg_id),
                    _ => continue,
                };

            // Read playback state
            let playback = state_guard.playback.read().await;
            let is_playing = playback.state.is_playing;

            // Only update if playing
            if !is_playing {
                continue;
            }

            let now_playing = playback
                .state
                .queued_songs
                .first()
                .map(|song| NowPlayingInfo {
                    song: song.clone(),
                    progress_secs: playback.state.playback_progress,
                });

            let next_up = playback.state.queued_songs.get(1).cloned();
            let queue_length = playback.state.queued_songs.len();
            let queue_duration_mins = {
                let total_secs: u64 = playback.state.queued_songs.iter().map(|s| s.duration).sum();
                total_secs.saturating_sub(playback.state.playback_progress) / 60
            };
            let upcoming_songs: Vec<Song> = playback
                .state
                .queued_songs
                .iter()
                .skip(1)
                .take(9)
                .cloned()
                .collect();

            drop(playback);
            drop(state_guard);

            // Build updated embed
            let embed = create_queue_embed_extended(
                now_playing.as_ref(),
                next_up.as_ref(),
                &upcoming_songs,
                queue_length,
                queue_duration_mins,
                is_playing,
            );

            // Edit the message
            if let Err(e) = channel_id
                .edit_message(&http, msg_id, EditMessage::new().embed(embed))
                .await
            {
                // Message might have been deleted, clear the ID
                if let serenity::Error::Http(serenity::HttpError::UnsuccessfulRequest(resp)) = &e {
                    if resp.status_code == serenity::StatusCode::NOT_FOUND {
                        let mut state_write = state.write().await;
                        state_write.now_playing_message_id = None;
                    }
                }
                debug!("Failed to update now-playing message: {:?}", e);
            }
        }
    });
}

/// Handles outgoing messages from the event bus to Discord
fn start_outgoing_message_handler(bus: EventBus, state: Arc<RwLock<BotState>>) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await;

            if let Event::Message(action) = event {
                // Get HTTP client and channel - skip if not ready yet
                let (http, channel_id) = {
                    let state_guard = state.read().await;
                    match &state_guard.http {
                        Some(http) => (http.clone(), state_guard.channel_id),
                        None => continue, // Bot not ready yet
                    }
                };

                match action {
                    MessageAction::Send { source, text, rich } if source != Platform::Discord => {
                        // Send message with optional rich embed
                        let result = match rich {
                            Some(RichContent::QueueStatus {
                                now_playing,
                                next_up,
                                queue_length,
                                queue_duration_mins,
                                is_playing,
                            }) => {
                                // Get upcoming songs from playback state
                                let upcoming_songs = {
                                    let state_guard = state.read().await;
                                    let playback = state_guard.playback.read().await;
                                    playback
                                        .state
                                        .queued_songs
                                        .iter()
                                        .skip(1)
                                        .take(9)
                                        .cloned()
                                        .collect::<Vec<_>>()
                                };

                                let embed = create_queue_embed_extended(
                                    now_playing.as_ref(),
                                    next_up.as_ref(),
                                    &upcoming_songs,
                                    queue_length,
                                    queue_duration_mins,
                                    is_playing,
                                );
                                let msg_result = channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await;

                                // Track message ID, reset skip votes on song change, add skip reaction
                                if let Ok(msg) = &msg_result {
                                    let mut state_write = state.write().await;

                                    // Check if song changed
                                    let new_song_id =
                                        now_playing.as_ref().map(|np| np.song.id.clone());
                                    if state_write.current_song_id != new_song_id {
                                        state_write.skip_votes.clear();
                                        state_write.current_song_id = new_song_id;
                                    }

                                    state_write.now_playing_message_id = Some(msg.id);

                                    // Add skip reaction if playing
                                    if is_playing && now_playing.is_some() {
                                        let http = http.clone();
                                        let msg_id = msg.id;
                                        let channel = channel_id;
                                        tokio::spawn(async move {
                                            if let Err(e) = channel
                                                .create_reaction(
                                                    &http,
                                                    msg_id,
                                                    ReactionType::Unicode("⏭️".to_string()),
                                                )
                                                .await
                                            {
                                                debug!("Failed to add skip reaction: {:?}", e);
                                            }
                                        });
                                    }
                                }

                                msg_result
                            }
                            Some(RichContent::SongEnqueued {
                                song,
                                time_until_playback_mins,
                            }) => {
                                let embed = create_enqueue_embed(&song, time_until_playback_mins);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::BingoAnnouncement { song }) => {
                                let embed = create_bingo_embed(&song);
                                let msg_result = channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await;

                                // Store the message ID for reaction tracking
                                if let Ok(msg) = &msg_result {
                                    let mut state_write = state.write().await;
                                    state_write.bingo_message_id = Some(msg.id);
                                }

                                msg_result
                            }
                            Some(RichContent::SongRequestList { songs }) => {
                                let embed = create_song_list_embed(&songs);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::Help { songbook_url }) => {
                                let embed = create_help_embed(&songbook_url);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::Countdown { value }) => {
                                let embed = create_countdown_embed(&value);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::SongRemoved { title }) => {
                                let embed = CreateEmbed::new()
                                    .title("🗑️ Song Removed")
                                    .description(format!("Removed **{}** from the queue", title))
                                    .color(0xff6600);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::SongRequestAdded { song }) => {
                                let title = song.title.clone().unwrap_or_else(|| song.id.clone());
                                let mut embed = CreateEmbed::new()
                                    .title("🎤 Song Request Added")
                                    .description(format!("Added **{}** to requests", title))
                                    .color(0x00ff00);
                                if let Some(book) = &song.book {
                                    embed = embed.field("📚 Songbook", book, true);
                                }
                                if let Some(url) = &song.url {
                                    embed = embed.field("🔗 Link", url, false);
                                }
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            Some(RichContent::Error { message }) => {
                                // Skip empty error messages
                                if message.trim().is_empty() {
                                    continue;
                                }
                                let embed = CreateEmbed::new()
                                    .title("❌ Error")
                                    .description(message)
                                    .color(0xff0000);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            None => {
                                // Plain text message - skip if empty
                                if text.trim().is_empty() {
                                    continue;
                                }
                                channel_id
                                    .send_message(&http, CreateMessage::new().content(&text))
                                    .await
                            }
                        };

                        if let Err(e) = result {
                            error!("Failed to send Discord message: {:?}", e);
                        }
                    }
                    MessageAction::Mirror {
                        username,
                        text,
                        source,
                    } if source != Platform::Discord => {
                        // Skip empty messages
                        if text.trim().is_empty() {
                            continue;
                        }
                        // Mirror messages from other platforms
                        let source_name = match source {
                            #[cfg(feature = "irc")]
                            Platform::Irc => "IRC",
                            Platform::Bot => "Bot",
                            #[allow(unreachable_patterns)]
                            _ => "Unknown",
                        };
                        let content = format!("**[{source_name}] {username}:** {text}");
                        if let Err(e) = channel_id
                            .send_message(&http, CreateMessage::new().content(&content))
                            .await
                        {
                            error!("Failed to mirror message to Discord: {:?}", e);
                        }
                    }
                    MessageAction::StoreBingoMessageId { message_id } => {
                        let mut state_write = state.write().await;
                        state_write.bingo_message_id = Some(serenity::MessageId::new(message_id));
                    }
                    _ => {}
                }
            }
        }
    });
}

// ============================================================================
// Slash Commands
// ============================================================================

/// Autocomplete for YouTube search
async fn autocomplete_youtube<'a>(
    _ctx: Context<'a>,
    partial: &'a str,
) -> Vec<poise::serenity_prelude::AutocompleteChoice> {
    // Don't search if it looks like a URL
    if partial.contains("://") || partial.contains("youtu") {
        return vec![];
    }

    match search_yt(partial, 10).await {
        Ok(results) => results
            .into_iter()
            .map(|(title, url)| poise::serenity_prelude::AutocompleteChoice::new(title, url))
            .collect(),
        Err(e) => {
            warn!("YouTube autocomplete failed: {e}");
            vec![]
        }
    }
}

/// Play a YouTube video or search for one
#[poise::command(slash_command)]
async fn play(
    ctx: Context<'_>,
    #[description = "YouTube URL or search terms"]
    #[autocomplete = "autocomplete_youtube"]
    url_or_search: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    info!("Discord /play from {nick}: {url_or_search}");

    // Check if it's a songbook URL
    if state.config.songbook.songbook_re.is_match(&url_or_search) {
        state
            .bus
            .send(Event::Songleader(SongleaderAction::RequestSongUrl {
                url: url_or_search,
                queued_by: nick,
            }));
        ctx.say("🎵 Looking up song...").await?;
        return Ok(());
    }

    ctx.defer().await?;

    let song = get_yt_song_info(url_or_search.clone(), nick.clone()).await;

    match song {
        Ok(song) if song.duration > MAX_SONG_DURATION.as_secs() => {
            ctx.say(format!(
                "❌ Song is too long! Max duration is {} minutes.",
                MAX_SONG_DURATION.as_secs() / 60
            ))
            .await?;
        }
        Ok(song) => {
            let title = song.title.clone();
            let url = song.url.clone();
            state
                .bus
                .send(Event::Playback(PlaybackAction::Enqueue { song }));
            // Mirror to IRC
            state.bus.send_message(MessageAction::Mirror {
                username: nick,
                text: format!("!p {url}"),
                source: Platform::Discord,
            });
            ctx.say(format!("🎵 Added **{title}** to the queue"))
                .await?;
        }
        Err(e) => {
            ctx.say(format!("❌ Error: {e}")).await?;
        }
    }

    Ok(())
}

/// Show the current queue
#[poise::command(slash_command)]
async fn queue(
    ctx: Context<'_>,
    #[description = "Position in queue to show"] position: Option<usize>,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Playback(PlaybackAction::ListQueue {
        offset: position,
    }));
    ctx.say("📋 Fetching queue...").await?;
    Ok(())
}

/// Remove your most recently queued song
#[poise::command(slash_command)]
async fn remove(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();
    state
        .bus
        .send(Event::Playback(PlaybackAction::RmSongByNick { nick }));
    ctx.say("🗑️ Removing your song...").await?;
    Ok(())
}

/// Make the bot say something with text-to-speech
#[poise::command(slash_command)]
async fn speak(
    ctx: Context<'_>,
    #[description = "Text to speak"] text: String,
) -> Result<(), anyhow::Error> {
    let username = ctx.author().name.clone();
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::TextToSpeech(TextToSpeechAction::Speak {
            text: text.clone(),
            prio: Priority::Low,
        }));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username,
        text: format!("!speak {text}"),
        source: Platform::Discord,
    });
    ctx.say(format!("🗣️ Speaking: {text}")).await?;
    Ok(())
}

/// Request a song to sing
#[poise::command(slash_command)]
async fn request(
    ctx: Context<'_>,
    #[description = "Song URL from songbook"]
    #[autocomplete = "autocomplete_song"]
    song_url: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    info!("Discord /request from {nick}: {song_url}");

    state
        .bus
        .send(Event::Songleader(SongleaderAction::RequestSongUrl {
            url: song_url,
            queued_by: nick,
        }));
    ctx.say("🎤 Adding song request...").await?;
    Ok(())
}

/// Autocomplete for song requests
async fn autocomplete_song<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> Vec<poise::serenity_prelude::AutocompleteChoice> {
    // For now, return empty - we'd need to implement a song search API
    // This is a placeholder for future songbook search functionality
    let state = ctx.data().read().await;
    let songbook_url = &state.config.songbook.songbook_url;

    // Return some example suggestions based on partial input
    if partial.is_empty() {
        vec![poise::serenity_prelude::AutocompleteChoice::new(
            "Paste a songbook URL or type to search",
            format!("{songbook_url}/"),
        )]
    } else {
        vec![poise::serenity_prelude::AutocompleteChoice::new(
            format!("Search for: {partial}"),
            format!("{songbook_url}/search?q={partial}"),
        )]
    }
}

/// Vote to advance to the next song
#[poise::command(slash_command)]
async fn tempo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    // Check if we're in tempo mode
    let songleader = state.songleader.read().await;
    let mode = &songleader.state.mode;

    if !matches!(mode, Mode::Tempo { .. }) {
        let msg = match mode {
            Mode::Inactive => "❌ The party hasn't started yet. Use `/song_admin begin` to start.",
            Mode::Starting => "❌ The party is starting, please wait...",
            Mode::Bingo { .. } => "❌ We're waiting for bingo! Use `/bingo` instead.",
            Mode::Singing => "❌ A song is being sung! Use `/skal` when it's finished.",
            Mode::Tempo { .. } => unreachable!(),
        };
        ctx.say(msg).await?;
        return Ok(());
    }

    drop(songleader);

    info!("Discord /tempo from {nick}");
    state.bus.send(Event::Songleader(SongleaderAction::Tempo {
        nick: nick.clone(),
    }));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username: nick,
        text: "!tempo".to_string(),
        source: Platform::Discord,
    });
    ctx.say("⏭️ Tempo!").await?;
    Ok(())
}

/// Signal that you're ready to sing (found the song page)
#[poise::command(slash_command)]
async fn bingo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    // Check if we're in bingo mode
    let songleader = state.songleader.read().await;
    let mode = &songleader.state.mode;

    if !matches!(mode, Mode::Bingo { .. }) {
        let msg = match mode {
            Mode::Inactive => "❌ The party hasn't started yet. Use `/song_admin begin` to start.",
            Mode::Starting => "❌ The party is starting, please wait...",
            Mode::Tempo { .. } => "❌ We're in tempo mode. Use `/tempo` to vote for the next song.",
            Mode::Singing => "❌ A song is being sung! Use `/skal` when it's finished.",
            Mode::Bingo { .. } => unreachable!(),
        };
        ctx.say(msg).await?;
        return Ok(());
    }

    drop(songleader);

    info!("Discord /bingo from {nick}");
    state.bus.send(Event::Songleader(SongleaderAction::Bingo {
        nick: nick.clone(),
    }));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username: nick,
        text: "!bingo".to_string(),
        source: Platform::Discord,
    });
    ctx.say("🎯 Bingo!").await?;
    Ok(())
}

/// Signal that the song is finished
#[poise::command(slash_command)]
async fn skal(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;

    // Check if we're in singing mode
    let songleader = state.songleader.read().await;
    let mode = &songleader.state.mode;

    if !matches!(mode, Mode::Singing) {
        let msg = match mode {
            Mode::Inactive => "❌ The party hasn't started yet. Use `/song_admin begin` to start.",
            Mode::Starting => "❌ The party is starting, please wait...",
            Mode::Tempo { .. } => {
                "❌ No song is being sung. Use `/tempo` to vote for the next song."
            }
            Mode::Bingo { .. } => {
                "❌ We're waiting for bingo! Use `/bingo` when you've found the song."
            }
            Mode::Singing => unreachable!(),
        };
        ctx.say(msg).await?;
        return Ok(());
    }

    drop(songleader);

    info!("Discord /skal from {}", ctx.author().name);
    state.bus.send(Event::Songleader(SongleaderAction::Skål));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username: ctx.author().name.clone(),
        text: "!skål".to_string(),
        source: Platform::Discord,
    });
    ctx.say("🍻 Skål!").await?;
    Ok(())
}

/// List current song requests
#[poise::command(slash_command, rename = "songs")]
async fn list_songs(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Songleader(SongleaderAction::ListSongs));
    ctx.say("📜 Fetching song requests...").await?;
    Ok(())
}

/// Show help
#[poise::command(slash_command)]
async fn help(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let songbook_url = {
        let state = ctx.data().read().await;
        state.config.songbook.songbook_url.clone()
    };
    let embed = create_help_embed(&songbook_url);
    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

/// Admin commands for the songleader
#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
    subcommands(
        "song_begin",
        "song_end",
        "song_pause",
        "song_force_tempo",
        "song_force_bingo",
        "song_force_singing"
    )
)]
async fn song_admin(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, rename = "begin")]
async fn song_begin(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Songleader(SongleaderAction::Begin));
    ctx.say("🎉 Starting the party!").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "end")]
async fn song_end(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Songleader(SongleaderAction::End));
    ctx.say("🔚 Ending the party...").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "pause")]
async fn song_pause(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Songleader(SongleaderAction::Pause));
    ctx.say("⏸️ Pausing songleader").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "force-tempo")]
async fn song_force_tempo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Songleader(SongleaderAction::ForceTempo));
    ctx.say("⏭️ Forcing tempo mode").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "force-bingo")]
async fn song_force_bingo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Songleader(SongleaderAction::ForceBingo));
    ctx.say("🎯 Forcing bingo mode").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "force-singing")]
async fn song_force_singing(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Songleader(SongleaderAction::ForceSinging));
    ctx.say("🎤 Forcing singing mode").await?;
    Ok(())
}

/// Admin commands for music playback
#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
    subcommands(
        "music_next",
        "music_prev",
        "music_pause",
        "music_resume",
        "music_volume"
    )
)]
async fn music_admin(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, rename = "next")]
async fn music_next(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Playback(PlaybackAction::Next));
    ctx.say("⏭️ Skipping to next song").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "prev")]
async fn music_prev(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Playback(PlaybackAction::Prev));
    ctx.say("⏮️ Going to previous song").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "pause")]
async fn music_pause(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Playback(PlaybackAction::Pause));
    ctx.say("⏸️ Pausing playback").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "resume")]
async fn music_resume(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Playback(PlaybackAction::Play));
    ctx.say("▶️ Resuming playback").await?;
    Ok(())
}

#[poise::command(slash_command, rename = "volume")]
async fn music_volume(
    ctx: Context<'_>,
    #[description = "Volume level (0.0 - 1.0)"] _volume: f64,
) -> Result<(), anyhow::Error> {
    // Volume control is now automatic via ducking
    ctx.say("🔊 Volume is now automatically controlled (music ducks when TTS plays)")
        .await?;
    Ok(())
}

/// Admin commands for voice channel
#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
    subcommands("voice_join", "voice_leave")
)]
async fn voice_admin(_ctx: Context<'_>) -> Result<(), anyhow::Error> {
    Ok(())
}

#[poise::command(slash_command, rename = "join")]
async fn voice_join(
    ctx: Context<'_>,
    #[description = "Voice channel ID to join"] channel_id: Option<String>,
) -> Result<(), anyhow::Error> {
    let guild_id = ctx
        .guild_id()
        .ok_or_else(|| anyhow::anyhow!("Not in a guild"))?;

    // Try to get channel from argument, or from user's current voice channel
    let vc_id = if let Some(id_str) = channel_id {
        ChannelId::new(id_str.parse()?)
    } else {
        // Try to find user's current voice channel
        let guild = ctx
            .guild()
            .ok_or_else(|| anyhow::anyhow!("Could not get guild"))?;
        let user_id = ctx.author().id;
        let channel = guild
            .voice_states
            .get(&user_id)
            .and_then(|state| state.channel_id)
            .ok_or_else(|| {
                anyhow::anyhow!("You must be in a voice channel or provide a channel ID")
            })?;
        channel
    };

    ctx.defer().await?;

    let manager = songbird::get(ctx.serenity_context())
        .await
        .expect("Songbird Voice client placed in at initialisation.");

    // Get the mixer from state
    let mixer = {
        let state = ctx.data().read().await;
        state.mixer.clone()
    };

    // Leave current channel if in one
    let _ = manager.leave(guild_id).await;

    match manager.join(guild_id, vc_id).await {
        Ok(handler_lock) => {
            let mut handler = handler_lock.lock().await;
            let input = create_voice_input(mixer);
            let track = Track::new(input);
            handler.play_only(track);
            ctx.say(format!("🔊 Joined voice channel <#{}>", vc_id))
                .await?;
        }
        Err(e) => {
            ctx.say(format!("❌ Failed to join voice channel: {}", e))
                .await?;
        }
    }

    Ok(())
}

#[poise::command(slash_command, rename = "leave")]
async fn voice_leave(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let guild_id = ctx
        .guild_id()
        .ok_or_else(|| anyhow::anyhow!("Not in a guild"))?;

    let manager = songbird::get(ctx.serenity_context())
        .await
        .expect("Songbird Voice client placed in at initialisation.");

    match manager.leave(guild_id).await {
        Ok(_) => {
            ctx.say("🔇 Left voice channel").await?;
        }
        Err(e) => {
            ctx.say(format!("❌ Failed to leave voice channel: {}", e))
                .await?;
        }
    }

    Ok(())
}

// ============================================================================
// Rich Embed Builders
// ============================================================================

/// Create a rich embed for queue status (basic version for backwards compatibility)
pub fn create_queue_embed(
    now_playing: Option<&NowPlayingInfo>,
    next_up: Option<&Song>,
    queue_length: usize,
    queue_duration_mins: u64,
    is_playing: bool,
) -> CreateEmbed {
    create_queue_embed_extended(
        now_playing,
        next_up,
        &[],
        queue_length,
        queue_duration_mins,
        is_playing,
    )
}

/// Create a rich embed for queue status with upcoming songs list
pub fn create_queue_embed_extended(
    now_playing: Option<&NowPlayingInfo>,
    next_up: Option<&Song>,
    upcoming_songs: &[Song],
    queue_length: usize,
    queue_duration_mins: u64,
    is_playing: bool,
) -> CreateEmbed {
    let status_emoji = if is_playing { "▶️" } else { "⏸️" };

    let mut embed = if let Some(np_info) = now_playing {
        let song = &np_info.song;
        let progress_secs = np_info.progress_secs;
        let duration = song.duration;

        // Create progress bar
        let progress_pct = if duration > 0 {
            (progress_secs as f64 / duration as f64 * 100.0) as usize
        } else {
            0
        };
        let filled = (progress_pct / 5).min(20); // 20 segments, clamped
        let empty = 20 - filled;
        let progress_bar = format!("{}{}", "▓".repeat(filled), "░".repeat(empty));

        let progress_str = format!(
            "{}:{:02} / {}:{:02}",
            progress_secs / 60,
            progress_secs % 60,
            duration / 60,
            duration % 60
        );

        // Use song title as the embed title (clickable via .url())
        CreateEmbed::new()
            .title(format!("{status_emoji} {}", song.title))
            .url(&song.url)
            .color(if is_playing { 0x00ff00 } else { 0xffaa00 })
            .field("👤 Queued by", &song.queued_by, true)
            .field("📺 Channel", &song.channel, true)
            .field("Progress", format!("{progress_bar}\n{progress_str}"), false)
    } else {
        CreateEmbed::new()
            .title(format!("{status_emoji} No song playing"))
            .description("Queue is empty!")
            .color(0x808080)
    };

    // Add next up
    if let Some(next) = next_up {
        embed = embed.field(
            "⏭️ Next up",
            format!("[{}]({})", next.title, next.url),
            false,
        );
    }

    // Add upcoming songs list (for Discord only, up to 9 more)
    if !upcoming_songs.is_empty() {
        let upcoming_list: Vec<String> = upcoming_songs
            .iter()
            .enumerate()
            .map(|(i, song)| {
                format!(
                    "{}. [{}]({}) - {}",
                    i + 2,
                    song.title,
                    song.url,
                    song.queued_by
                )
            })
            .collect();
        embed = embed.field("📋 Coming up", upcoming_list.join("\n"), false);
    }

    embed = embed.footer(serenity::CreateEmbedFooter::new(format!(
        "Queue: {} songs ({} min) • React ⏭️ to vote skip",
        queue_length, queue_duration_mins
    )));

    embed
}

/// Create a rich embed for song enqueued
pub fn create_enqueue_embed(song: &Song, time_until_playback_mins: u64) -> CreateEmbed {
    CreateEmbed::new()
        .title(format!("✅ {}", song.title))
        .url(&song.url)
        .color(0x00ff00)
        .description("Added to queue")
        .field("📺 Channel", &song.channel, true)
        .field("👤 Queued by", &song.queued_by, true)
        .field(
            "⏱️ Time until playback",
            format!("{} min", time_until_playback_mins),
            true,
        )
}

/// Create a rich embed for bingo announcement
pub fn create_bingo_embed(song: &SongbookSong) -> CreateEmbed {
    let title = song.title.clone().unwrap_or_else(|| song.id.clone());
    let mut embed = CreateEmbed::new()
        .title("🎯 Next Song Coming Up!")
        .description(format!("**{}**", title))
        .color(0xff6600)
        .field(
            "📖 Instructions",
            "React to this message or type `/bingo` when you've found the song!",
            false,
        );

    if let Some(url) = &song.url {
        embed = embed.field("🔗 Link", url, false);
    }

    if let Some(book) = &song.book {
        embed = embed.field("📚 Songbook", book, true);
    }

    embed
}

/// Create a rich embed for song request list
pub fn create_song_list_embed(songs: &[SongbookSong]) -> CreateEmbed {
    let mut embed = CreateEmbed::new().title("📜 Song Requests").color(0x0099ff);

    if songs.is_empty() {
        embed = embed.description("No songs requested yet! Use `/request` to add one.");
    } else {
        let song_list: Vec<String> = songs
            .iter()
            .enumerate()
            .take(25) // Discord embed field limit
            .map(|(i, song)| {
                let title = song.title.clone().unwrap_or_else(|| song.id.clone());
                let queued_by = song
                    .queued_by
                    .clone()
                    .map(|q| format!(" (by {})", q))
                    .unwrap_or_default();
                format!("{}. {}{}", i + 1, title, queued_by)
            })
            .collect();

        embed = embed.description(song_list.join("\n"));

        if songs.len() > 25 {
            embed = embed.footer(serenity::CreateEmbedFooter::new(format!(
                "... and {} more songs",
                songs.len() - 25
            )));
        }
    }

    embed
}

/// Create a rich embed for help
pub fn create_help_embed(songbook_url: &str) -> CreateEmbed {
    CreateEmbed::new()
        .title("ℹ️ Bot Commands")
        .color(0x5865f2)
        .description("Welcome to the sitzning bot! Here are the available commands:")
        .field(
            "🎵 Music",
            "`/play <url>` - Queue a YouTube video\n\
             `/queue` - Show current queue\n\
             `/remove` - Remove your last queued song",
            false,
        )
        .field(
            "🎤 Singing",
            "`/request <url>` - Request a song to sing\n\
             `/songs` - List song requests\n\
             `/tempo` - Vote for next song\n\
             `/bingo` - Signal you found the song\n\
             `/skal` - Song finished!",
            false,
        )
        .field(
            "💬 Other",
            "`/speak <text>` - Text-to-speech\n\
             `/help` - Show this message",
            false,
        )
        .field(
            "💡 Tip",
            "You can also react to bingo announcements instead of typing `/bingo`!",
            false,
        )
        .footer(serenity::CreateEmbedFooter::new(format!(
            "Songbook: {}",
            songbook_url
        )))
}

/// Create a countdown embed
pub fn create_countdown_embed(value: &CountdownValue) -> CreateEmbed {
    let (text, color) = match value {
        CountdownValue::Three => ("3️⃣", 0xff0000),
        CountdownValue::Two => ("2️⃣", 0xff6600),
        CountdownValue::One => ("1️⃣", 0xffcc00),
        CountdownValue::Now => ("🎤 NOW!", 0x00ff00),
    };

    CreateEmbed::new().title(text).color(color)
}
