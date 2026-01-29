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
    playback::{PlaybackAction, SharedPlayback, Song, SongVotes, MAX_SONG_DURATION},
    songbook::SongbookSong,
    songleader::{Mode, SharedSongleader, SongleaderAction, NUM_BINGO_NICKS, NUM_TEMPO_NICKS},
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
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tokio::sync::RwLock;

/// Discord bot state shared across handlers
struct BotState {
    bus: EventBus,
    config: Config,
    channel_id: ChannelId,
    /// Message ID of the current bingo announcement (for reaction tracking)
    bingo_message_id: Option<serenity::MessageId>,
    /// Message ID of the current now-playing message (for progress updates)
    now_playing_message_id: Option<serenity::MessageId>,
    /// Current song ID being played (to detect song changes)
    current_song_id: Option<String>,
    /// Users who have voted to skip the current song
    skip_votes: HashSet<String>,
    /// Mapping from song ID to enqueue message ID (for vote reactions)
    enqueue_message_ids: HashMap<String, serenity::MessageId>,
    /// Mapping from queue message ID to song ID (for skip reactions on any queue message)
    queue_message_song_ids: HashMap<serenity::MessageId, String>,
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
const SKIP_VOTES_REQUIRED: usize = 3;

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
        enqueue_message_ids: HashMap::new(),
        queue_message_song_ids: HashMap::new(),
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
                bot_state(),
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

            // Check if this is a skip reaction on a queue message showing the current song
            let is_skip_reaction = matches!(&add_reaction.emoji, ReactionType::Unicode(s) if s == "⏭️")
                && state
                    .queue_message_song_ids
                    .get(&add_reaction.message_id)
                    .map(|song_id| state.current_song_id.as_ref() == Some(song_id))
                    .unwrap_or(false);

            // Check if this is a vote reaction on an enqueue message
            let vote_song_id = state
                .enqueue_message_ids
                .iter()
                .find(|(_, &msg_id)| msg_id == add_reaction.message_id)
                .map(|(song_id, _)| song_id.clone());

            let bus = state.bus.clone();
            drop(state);

            if is_skip_reaction {
                if let Some(user) = &add_reaction.member {
                    // Ignore bot reactions
                    if user.user.bot {
                        return Ok(());
                    }

                    let nick = user.nick.clone().unwrap_or_else(|| user.user.name.clone());
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
                            info!(
                                "Skip vote threshold reached but playback is paused, not skipping"
                            );
                        }
                    }
                }
            }

            if let Some(song_id) = vote_song_id {
                if let Some(user) = &add_reaction.member {
                    // Ignore bot reactions
                    if user.user.bot {
                        return Ok(());
                    }

                    let nick = user.nick.clone().unwrap_or_else(|| user.user.name.clone());

                    match &add_reaction.emoji {
                        ReactionType::Unicode(s) if s == "👍" => {
                            info!("Upvote from {nick} for song {song_id}");
                            bus.send(Event::Playback(PlaybackAction::Upvote {
                                song_id,
                                user: nick,
                            }));
                        }
                        ReactionType::Unicode(s) if s == "👎" => {
                            info!("Downvote from {nick} for song {song_id}");
                            bus.send(Event::Playback(PlaybackAction::Downvote {
                                song_id,
                                user: nick,
                            }));
                        }
                        _ => {}
                    }
                }
            }
        }
        serenity::FullEvent::ReactionRemove { removed_reaction } => {
            let state = data.read().await;

            // Check if this is removing a vote reaction on an enqueue message
            let song_id = state
                .enqueue_message_ids
                .iter()
                .find(|(_, &msg_id)| msg_id == removed_reaction.message_id)
                .map(|(song_id, _)| song_id.clone());

            if let Some(song_id) = song_id {
                if let Some(user_id) = removed_reaction.user_id {
                    let bus = state.bus.clone();
                    let http = state.http.clone();
                    drop(state);

                    // We need to get the username from the user ID
                    if let Some(http) = http {
                        if let Ok(user) = http.get_user(user_id).await {
                            // Ignore bot reactions
                            if user.bot {
                                return Ok(());
                            }

                            let nick = user.name.clone();

                            match &removed_reaction.emoji {
                                ReactionType::Unicode(s) if s == "👍" => {
                                    info!("Removed upvote from {nick} for song {song_id}");
                                    bus.send(Event::Playback(PlaybackAction::RemoveUpvote {
                                        song_id,
                                        user: nick,
                                    }));
                                }
                                ReactionType::Unicode(s) if s == "👎" => {
                                    info!("Removed downvote from {nick} for song {song_id}");
                                    bus.send(Event::Playback(PlaybackAction::RemoveDownvote {
                                        song_id,
                                        user: nick,
                                    }));
                                }
                                _ => {}
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

            let queue_length = playback.state.queued_songs.len().saturating_sub(1);
            let queue_duration_mins = {
                let total_secs: u64 = playback
                    .state
                    .queued_songs
                    .iter()
                    .skip(1)
                    .map(|s| s.duration)
                    .sum();
                total_secs / 60
            };
            let upcoming_songs: Vec<Song> = playback
                .state
                .queued_songs
                .iter()
                .skip(1)
                .take(9)
                .cloned()
                .collect();

            // Get vote info for upcoming songs
            let song_votes: HashMap<String, SongVotes> = upcoming_songs
                .iter()
                .map(|s| (s.id.clone(), playback.get_votes(&s.id)))
                .collect();

            drop(playback);
            drop(state_guard);

            // Build updated embed with vote info
            let embed = create_queue_embed_with_votes(
                now_playing.as_ref(),
                &upcoming_songs,
                &song_votes,
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
                                queue_length,
                                queue_duration_mins,
                                is_playing,
                            }) => {
                                // Get upcoming songs from playback state with vote info
                                let (upcoming_songs, song_votes) = {
                                    let state_guard = state.read().await;
                                    let playback = state_guard.playback.read().await;
                                    let songs: Vec<_> = playback
                                        .state
                                        .queued_songs
                                        .iter()
                                        .skip(1)
                                        .take(9)
                                        .cloned()
                                        .collect();
                                    let votes: HashMap<String, SongVotes> = songs
                                        .iter()
                                        .map(|s| (s.id.clone(), playback.get_votes(&s.id)))
                                        .collect();
                                    (songs, votes)
                                };

                                let embed = create_queue_embed_with_votes(
                                    now_playing.as_ref(),
                                    &upcoming_songs,
                                    &song_votes,
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
                                    let song_changed = state_write.current_song_id != new_song_id;

                                    if song_changed {
                                        state_write.skip_votes.clear();

                                        // Clean up old queue message mappings for the old song
                                        if let Some(old_song_id) = &state_write.current_song_id {
                                            let old_id = old_song_id.clone();
                                            state_write
                                                .queue_message_song_ids
                                                .retain(|_, v| v != &old_id);
                                        }

                                        state_write.current_song_id = new_song_id.clone();

                                        // Clean up enqueue message tracking for the now-playing song
                                        // (it's no longer in the queue, so reactions don't matter)
                                        if let Some(song_id) = &new_song_id {
                                            state_write.enqueue_message_ids.remove(song_id);
                                        }
                                    }

                                    state_write.now_playing_message_id = Some(msg.id);

                                    // Track this message for skip reactions
                                    if let Some(song_id) = &new_song_id {
                                        state_write
                                            .queue_message_song_ids
                                            .insert(msg.id, song_id.clone());
                                    }

                                    // Add skip reaction when a song is playing
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
                                let msg_result = channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await;

                                // Track message ID for vote reactions and add thumbs reactions
                                if let Ok(msg) = &msg_result {
                                    let mut state_write = state.write().await;
                                    state_write
                                        .enqueue_message_ids
                                        .insert(song.id.clone(), msg.id);

                                    // Add vote reactions
                                    let http = http.clone();
                                    let msg_id = msg.id;
                                    let channel = channel_id;
                                    tokio::spawn(async move {
                                        // Add thumbs up
                                        if let Err(e) = channel
                                            .create_reaction(
                                                &http,
                                                msg_id,
                                                ReactionType::Unicode("👍".to_string()),
                                            )
                                            .await
                                        {
                                            debug!("Failed to add thumbs up reaction: {:?}", e);
                                        }
                                        // Add thumbs down
                                        if let Err(e) = channel
                                            .create_reaction(
                                                &http,
                                                msg_id,
                                                ReactionType::Unicode("👎".to_string()),
                                            )
                                            .await
                                        {
                                            debug!("Failed to add thumbs down reaction: {:?}", e);
                                        }
                                    });
                                }

                                msg_result
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
async fn queue(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Playback(PlaybackAction::ListQueue { offset: None }));
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
        .send(Event::Playback(PlaybackAction::RmSongByNick {
            nick: nick.clone(),
        }));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username: nick,
        text: "!rm".to_string(),
        source: Platform::Discord,
    });
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
    #[description = "Song URL from songbook"] song_url: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    info!("Discord /request from {nick}: {song_url}");

    state
        .bus
        .send(Event::Songleader(SongleaderAction::RequestSongUrl {
            url: song_url.clone(),
            queued_by: nick.clone(),
        }));
    // Mirror to IRC
    state.bus.send_message(MessageAction::Mirror {
        username: nick,
        text: format!("!request {song_url}"),
        source: Platform::Discord,
    });
    ctx.say("🎤 Adding song request...").await?;
    Ok(())
}

/// Autocomplete for songbook - just shows the songbook URL
async fn autocomplete_songbook<'a>(
    ctx: Context<'_>,
    _partial: &'a str,
) -> Vec<poise::serenity_prelude::AutocompleteChoice> {
    let state = ctx.data().read().await;
    let songbook_url = &state.config.songbook.songbook_url;

    vec![poise::serenity_prelude::AutocompleteChoice::new(
        format!("Open songbook: {songbook_url}"),
        songbook_url.clone(),
    )]
}

/// Vote to advance to the next song
#[poise::command(slash_command)]
async fn tempo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    // Check if we're in tempo mode and get current vote count
    let songleader = state.songleader.read().await;
    let mode = &songleader.state.mode;

    let (current_votes, already_voted) = match mode {
        Mode::Tempo { nicks, .. } => (nicks.len(), nicks.contains(&nick)),
        _ => {
            let msg = match mode {
                Mode::Inactive => {
                    "❌ The party hasn't started yet. Use `/song_admin begin` to start."
                }
                Mode::Starting => "❌ The party is starting, please wait...",
                Mode::Bingo { .. } => "❌ We're waiting for bingo! Use `/bingo` instead.",
                Mode::Singing => "❌ A song is being sung! Use `/skal` when it's finished.",
                Mode::Tempo { .. } => unreachable!(),
            };
            ctx.say(msg).await?;
            return Ok(());
        }
    };

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

    let response = if already_voted {
        format!("⏭️ You already voted! ({current_votes}/{NUM_TEMPO_NICKS})")
    } else {
        let new_count = current_votes + 1;
        if new_count >= NUM_TEMPO_NICKS {
            "⏭️ Tempo! 🎉 That's enough votes, moving to bingo!".to_string()
        } else {
            let remaining = NUM_TEMPO_NICKS - new_count;
            format!("⏭️ Tempo! ({new_count}/{NUM_TEMPO_NICKS}, need {remaining} more)")
        }
    };
    ctx.say(response).await?;
    Ok(())
}

/// Signal that you're ready to sing (found the song page)
#[poise::command(slash_command)]
async fn bingo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

    // Check if we're in bingo mode and get current vote count
    let songleader = state.songleader.read().await;
    let mode = &songleader.state.mode;

    let (current_votes, already_voted, song_title) = match mode {
        Mode::Bingo { nicks, song } => {
            let title = song.title.clone().unwrap_or_else(|| song.id.clone());
            (nicks.len(), nicks.contains(&nick), title)
        }
        _ => {
            let msg = match mode {
                Mode::Inactive => {
                    "❌ The party hasn't started yet. Use `/song_admin begin` to start."
                }
                Mode::Starting => "❌ The party is starting, please wait...",
                Mode::Tempo { .. } => {
                    "❌ We're in tempo mode. Use `/tempo` to speedup waiting for the next song."
                }
                Mode::Singing => "❌ A song is being sung! Use `/skal` when it's finished.",
                Mode::Bingo { .. } => unreachable!(),
            };
            ctx.say(msg).await?;
            return Ok(());
        }
    };

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

    let response = if already_voted {
        format!("🎯 You already found **{song_title}**! ({current_votes}/{NUM_BINGO_NICKS})")
    } else {
        let new_count = current_votes + 1;
        if new_count >= NUM_BINGO_NICKS {
            format!("🎯 Bingo! 🎉 Everyone's ready to sing **{song_title}**!")
        } else {
            let remaining = NUM_BINGO_NICKS - new_count;
            format!(
                "🎯 Found **{song_title}**! ({new_count}/{NUM_BINGO_NICKS}, need {remaining} more)"
            )
        }
    };
    ctx.say(response).await?;
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
                "❌ No song is being sung. Use `/tempo` to speedup waiting for the next song."
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
        "song_force_singing",
        "song_remove_music",
        "song_remove_song"
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

/// Autocomplete for remove-music - shows users with songs in the music queue
async fn autocomplete_music_users<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> Vec<poise::serenity_prelude::AutocompleteChoice> {
    let state = ctx.data().read().await;
    let playback = state.playback.read().await;

    // Find the last song for each user (skip currently playing song at index 0)
    let mut user_songs: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for song in playback.state.queued_songs.iter().skip(1) {
        user_songs.insert(song.queued_by.clone(), song.title.clone());
    }

    user_songs
        .into_iter()
        .filter(|(user, _)| user.to_lowercase().contains(&partial.to_lowercase()))
        .take(25)
        .map(|(user, title)| {
            let display = format!("{user}: {title}");
            poise::serenity_prelude::AutocompleteChoice::new(display, user)
        })
        .collect()
}

/// Autocomplete for remove-song - shows users with songs in the songbook requests
async fn autocomplete_song_users<'a>(
    ctx: Context<'_>,
    partial: &'a str,
) -> Vec<poise::serenity_prelude::AutocompleteChoice> {
    let state = ctx.data().read().await;
    let songleader = state.songleader.read().await;

    // Find the last song for each user
    let mut user_songs: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for song in &songleader.state.requests {
        if let Some(queued_by) = &song.queued_by {
            let title = song.title.as_ref().unwrap_or(&song.id).clone();
            user_songs.insert(queued_by.clone(), title);
        }
    }

    user_songs
        .into_iter()
        .filter(|(user, _)| user.to_lowercase().contains(&partial.to_lowercase()))
        .take(25)
        .map(|(user, title)| {
            let display = format!("{user}: {title}");
            poise::serenity_prelude::AutocompleteChoice::new(display, user)
        })
        .collect()
}

#[poise::command(slash_command, rename = "remove-music")]
async fn song_remove_music(
    ctx: Context<'_>,
    #[description = "Username whose last music queue entry to remove"]
    #[autocomplete = "autocomplete_music_users"]
    username: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Playback(PlaybackAction::RmSongByNick {
            nick: username.clone(),
        }));
    ctx.say(format!("🗑️ Removing last music queued by {username}"))
        .await?;
    Ok(())
}

#[poise::command(slash_command, rename = "remove-song")]
async fn song_remove_song(
    ctx: Context<'_>,
    #[description = "Username whose last songbook request to remove"]
    #[autocomplete = "autocomplete_song_users"]
    username: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::Songleader(SongleaderAction::RmSongByNick {
            nick: username.clone(),
        }));
    ctx.say(format!("🗑️ Removing last song requested by {username}"))
        .await?;
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

/// Show current bot state (admin only)
#[poise::command(
    slash_command,
    rename = "state",
    required_permissions = "ADMINISTRATOR"
)]
async fn bot_state(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;

    // Get songleader state
    let songleader = state.songleader.read().await;
    let mode_str = match &songleader.state.mode {
        Mode::Inactive => "Inactive".to_string(),
        Mode::Starting => "Starting".to_string(),
        Mode::Tempo { nicks, .. } => format!("Tempo ({}/{} votes)", nicks.len(), NUM_TEMPO_NICKS),
        Mode::Bingo { nicks, song } => {
            let title = song.title.clone().unwrap_or_else(|| song.id.clone());
            format!(
                "Bingo ({}/{} ready) - {}",
                nicks.len(),
                NUM_BINGO_NICKS,
                title
            )
        }
        Mode::Singing => "Singing".to_string(),
    };
    let requests_count = songleader.state.requests.len();
    let first_songs_count = songleader.state.first_songs.len();
    let backup_count = songleader.state.backup.len();
    drop(songleader);

    // Get playback state
    let playback = state.playback.read().await;
    let queue_len = playback.state.queued_songs.len();
    let played_len = playback.state.played_songs.len();
    let is_playing = playback.state.is_playing;
    let should_play = playback.state.should_play;
    let now_playing = playback.state.queued_songs.first().map(|s| s.title.clone());
    let votes_count = playback.state.song_votes.len();
    drop(playback);

    let embed = CreateEmbed::new()
        .title("🔧 Bot State")
        .color(0x5865f2)
        .field(
            "🎤 Songleader",
            format!(
                "**Mode:** {}\n**Requests:** {}\n**First songs:** {}\n**Backup:** {}",
                mode_str, requests_count, first_songs_count, backup_count
            ),
            false,
        )
        .field(
            "🎵 Playback",
            format!(
                "**Now playing:** {}\n**Queue:** {} songs\n**Played:** {} songs\n**Playing:** {}\n**Should play:** {}\n**Songs with votes:** {}",
                now_playing.unwrap_or_else(|| "(nothing)".to_string()),
                queue_len,
                played_len,
                if is_playing { "Yes" } else { "No" },
                if should_play { "Yes" } else { "No" },
                votes_count
            ),
            false,
        );

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

// ============================================================================
// Rich Embed Builders
// ============================================================================

/// Create a rich embed for queue status (basic version for backwards compatibility)
pub fn create_queue_embed(
    now_playing: Option<&NowPlayingInfo>,
    queue_length: usize,
    queue_duration_mins: u64,
    is_playing: bool,
) -> CreateEmbed {
    create_queue_embed_extended(
        now_playing,
        &[],
        queue_length,
        queue_duration_mins,
        is_playing,
    )
}

/// Create a rich embed for queue status with upcoming songs list
pub fn create_queue_embed_extended(
    now_playing: Option<&NowPlayingInfo>,
    upcoming_songs: &[Song],
    queue_length: usize,
    queue_duration_mins: u64,
    is_playing: bool,
) -> CreateEmbed {
    create_queue_embed_with_votes(
        now_playing,
        upcoming_songs,
        &HashMap::new(),
        queue_length,
        queue_duration_mins,
        is_playing,
    )
}

/// Create a rich embed for queue status with upcoming songs list and vote info
pub fn create_queue_embed_with_votes(
    now_playing: Option<&NowPlayingInfo>,
    upcoming_songs: &[Song],
    song_votes: &HashMap<String, SongVotes>,
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
            .title(format!("{status_emoji} Now playing: {}", song.title))
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

    // Add upcoming songs list with vote info (for Discord only, up to 9 more)
    if !upcoming_songs.is_empty() {
        // Sort by vote score for display (highest first)
        let mut songs_with_votes: Vec<_> = upcoming_songs
            .iter()
            .enumerate()
            .map(|(orig_pos, song)| {
                let votes = song_votes.get(&song.id);
                let score = votes.map(|v| v.score()).unwrap_or(0);
                (song, orig_pos, score)
            })
            .collect();

        songs_with_votes.sort_by(|a, b| b.2.cmp(&a.2));

        let upcoming_list: Vec<String> = songs_with_votes
            .iter()
            .enumerate()
            .map(|(display_pos, (song, orig_pos, _score))| {
                let votes = song_votes.get(&song.id);
                let vote_indicator = format_vote_indicator(votes);
                let pos_change = format_position_change(*orig_pos, display_pos);
                format!(
                    "{}. [{}]({}){}{} - {}",
                    display_pos + 2,
                    song.title,
                    song.url,
                    vote_indicator,
                    pos_change,
                    song.queued_by
                )
            })
            .collect();
        embed = embed.field("📋 Up next", upcoming_list.join("\n"), false);
    }

    embed = embed.footer(serenity::CreateEmbedFooter::new(format!(
        "Queue: {} songs ({} min) • React ⏭️ to vote skip",
        queue_length, queue_duration_mins
    )));

    embed
}

/// Format vote indicator for display (e.g., " (+2)" or " (-1)")
fn format_vote_indicator(votes: Option<&SongVotes>) -> String {
    match votes {
        Some(v) if v.score() > 0 => format!(" 👍+{}", v.score()),
        Some(v) if v.score() < 0 => format!(" 👎{}", v.score()),
        _ => String::new(),
    }
}

/// Format position change indicator (e.g., " ↑2" or " ↓1")
fn format_position_change(original_pos: usize, current_pos: usize) -> String {
    if current_pos < original_pos {
        format!(" ↑{}", original_pos - current_pos)
    } else if current_pos > original_pos {
        format!(" ↓{}", current_pos - original_pos)
    } else {
        String::new()
    }
}

/// Create a rich embed for song enqueued
pub fn create_enqueue_embed(song: &Song, time_until_playback_mins: u64) -> CreateEmbed {
    CreateEmbed::new()
        .title(format!("✅ {}", song.title))
        .url(&song.url)
        .color(0x00ff00)
        .description("Added to queue • React 👍/👎 to move up/down")
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
