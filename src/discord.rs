//! Discord integration for the sitzning bot.
//!
//! This module provides Discord support including:
//! - Message mirroring between Discord and IRC
//! - Slash commands for all bot commands
//! - Rich embeds for queue status, song info, etc.
//! - Reaction-based bingo (react to signal you found the song)
//! - Song request autocomplete dropdown

use crate::{
    config::{Config, DiscordConfig},
    event::{Event, EventBus},
    message::{CountdownValue, MessageAction, NowPlayingInfo, Platform, RichContent},
    mixer::MixerAction,
    playback::{PlaybackAction, Song, MAX_SONG_DURATION},
    songbook::SongbookSong,
    songleader::SongleaderAction,
    sources::espeak::{Priority, TextToSpeechAction},
    youtube::get_yt_song_info,
};
use anyhow::Result;
use poise::serenity_prelude::{
    self as serenity, ChannelId, CreateEmbed, CreateMessage, GuildId, Http,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Discord bot state shared across handlers
struct BotState {
    bus: EventBus,
    config: Config,
    channel_id: ChannelId,
    /// Message ID of the current bingo announcement (for reaction tracking)
    bingo_message_id: Option<serenity::MessageId>,
    /// HTTP client for sending messages (set when bot is ready)
    http: Option<Arc<Http>>,
}

type Context<'a> = poise::Context<'a, Arc<RwLock<BotState>>, anyhow::Error>;

/// Initialize the Discord bot
pub async fn init(bus: &EventBus, config: &Config, discord_config: &DiscordConfig) -> Result<()> {
    let channel_id = ChannelId::new(discord_config.discord_channel_id);
    let guild_id = GuildId::new(discord_config.discord_guild_id);
    let token = discord_config.discord_token.clone();

    let state = Arc::new(RwLock::new(BotState {
        bus: bus.clone(),
        config: config.clone(),
        channel_id,
        bingo_message_id: None,
        http: None,
    }));

    // Start the outgoing message handler
    start_outgoing_message_handler(bus.clone(), state.clone());

    let state_for_setup = state.clone();

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
            ],
            event_handler: |ctx, event, _framework, data| Box::pin(event_handler(ctx, event, data)),
            ..Default::default()
        })
        .setup(move |ctx, _ready, framework| {
            let state = state_for_setup.clone();
            Box::pin(async move {
                // Store the HTTP client for message sending
                {
                    let mut state_write = state.write().await;
                    state_write.http = Some(ctx.http.clone());
                }

                // Register commands for the specific guild (faster updates during development)
                poise::builtins::register_in_guild(ctx, &framework.options().commands, guild_id)
                    .await?;
                info!("Discord bot ready and commands registered!");
                Ok(state)
            })
        })
        .build();

    // Build and start the serenity client
    let intents = serenity::GatewayIntents::GUILD_MESSAGES
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MESSAGE_REACTIONS;

    let client = serenity::ClientBuilder::new(&token, intents)
        .framework(framework)
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
                        state
                            .bus
                            .send(Event::Songleader(SongleaderAction::Bingo { nick }));
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
        _ => None,
    }
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
                                let embed = create_queue_embed(
                                    now_playing.as_ref(),
                                    next_up.as_ref(),
                                    queue_length,
                                    queue_duration_mins,
                                    is_playing,
                                );
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
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
                                let embed = CreateEmbed::new()
                                    .title("❌ Error")
                                    .description(message)
                                    .color(0xff0000);
                                channel_id
                                    .send_message(&http, CreateMessage::new().embed(embed))
                                    .await
                            }
                            None => {
                                // Plain text message
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
                        // Mirror messages from other platforms
                        let source_name = match source {
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

/// Play a YouTube video or search for one
#[poise::command(slash_command)]
async fn play(
    ctx: Context<'_>,
    #[description = "YouTube URL or search terms"] url_or_search: String,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();

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

    let song = get_yt_song_info(url_or_search, nick).await;

    match song {
        Ok(song) if song.duration > MAX_SONG_DURATION.as_secs() => {
            ctx.say(format!(
                "❌ Song is too long! Max duration is {} minutes.",
                MAX_SONG_DURATION.as_secs() / 60
            ))
            .await?;
        }
        Ok(song) => {
            state
                .bus
                .send(Event::Playback(PlaybackAction::Enqueue { song }));
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
    let state = ctx.data().read().await;
    state
        .bus
        .send(Event::TextToSpeech(TextToSpeechAction::Speak {
            text: text.clone(),
            prio: Priority::Low,
        }));
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
    state
        .bus
        .send(Event::Songleader(SongleaderAction::Tempo { nick }));
    ctx.say("⏭️ Tempo!").await?;
    Ok(())
}

/// Signal that you're ready to sing (found the song page)
#[poise::command(slash_command)]
async fn bingo(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let nick = ctx.author().name.clone();
    state
        .bus
        .send(Event::Songleader(SongleaderAction::Bingo { nick }));
    ctx.say("🎯 Bingo!").await?;
    Ok(())
}

/// Signal that the song is finished
#[poise::command(slash_command)]
async fn skal(ctx: Context<'_>) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    state.bus.send(Event::Songleader(SongleaderAction::Skål));
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
    let state = ctx.data().read().await;
    state.bus.send(Event::Songleader(SongleaderAction::Help));
    ctx.say("ℹ️ Fetching help...").await?;
    Ok(())
}

/// Admin commands for the songleader
#[poise::command(
    slash_command,
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
    #[description = "Volume level (0.0 - 1.0)"] volume: f64,
) -> Result<(), anyhow::Error> {
    let state = ctx.data().read().await;
    let volume = volume.clamp(0.0, 1.0);
    state
        .bus
        .send(Event::Mixer(MixerAction::SetSecondaryChannelVolume(volume)));
    ctx.say(format!("🔊 Volume set to {:.0}%", volume * 100.0))
        .await?;
    Ok(())
}

// ============================================================================
// Rich Embed Builders
// ============================================================================

/// Create a rich embed for queue status
pub fn create_queue_embed(
    now_playing: Option<&NowPlayingInfo>,
    next_up: Option<&Song>,
    queue_length: usize,
    queue_duration_mins: u64,
    is_playing: bool,
) -> CreateEmbed {
    let mut embed = CreateEmbed::new()
        .title(if is_playing {
            "▶️ Now Playing"
        } else {
            "⏸️ Paused"
        })
        .color(if is_playing { 0x00ff00 } else { 0xffaa00 });

    if let Some(np_info) = now_playing {
        let song = &np_info.song;
        let progress_secs = np_info.progress_secs;
        let duration = song.duration;

        // Create progress bar
        let progress_pct = if duration > 0 {
            (progress_secs as f64 / duration as f64 * 100.0) as usize
        } else {
            0
        };
        let filled = progress_pct / 5; // 20 segments
        let empty = 20 - filled;
        let progress_bar = format!("{}{}", "▓".repeat(filled), "░".repeat(empty));

        let progress_str = format!(
            "{}:{:02} / {}:{:02}",
            progress_secs / 60,
            progress_secs % 60,
            duration / 60,
            duration % 60
        );

        embed = embed
            .field("🎵 Song", &song.title, false)
            .field("👤 Queued by", &song.queued_by, true)
            .field("📺 Channel", &song.channel, true)
            .field("Progress", format!("{progress_bar}\n{progress_str}"), false)
            .url(&song.url);
    } else {
        embed = embed.description("No song currently playing");
    }

    if let Some(next) = next_up {
        embed = embed.field("⏭️ Next up", &next.title, false);
    }

    embed = embed.footer(serenity::CreateEmbedFooter::new(format!(
        "Queue: {} songs ({} min)",
        queue_length, queue_duration_mins
    )));

    embed
}

/// Create a rich embed for song enqueued
pub fn create_enqueue_embed(song: &Song, time_until_playback_mins: u64) -> CreateEmbed {
    CreateEmbed::new()
        .title("✅ Added to Queue")
        .color(0x00ff00)
        .field("🎵 Song", &song.title, false)
        .field("📺 Channel", &song.channel, true)
        .field("👤 Queued by", &song.queued_by, true)
        .field(
            "⏱️ Time until playback",
            format!("{} min", time_until_playback_mins),
            true,
        )
        .url(&song.url)
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
