use crate::{
    event::{Event, EventBus},
    mixer::MixerAction,
    playback::{PlaybackAction, MAX_SONG_DURATION},
    songbook::SongbookSong,
    songleader::SongleaderAction,
    sources::espeak::{Priority, TextToSpeechAction},
    youtube::get_yt_song_info,
};
use anyhow::Result;
use futures::StreamExt;
use irc::client::prelude::*;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Initial delay before first reconnection attempt
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
/// Maximum delay between reconnection attempts
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30); // 30 seconds
/// Multiplier for exponential backoff
const RECONNECT_BACKOFF_MULTIPLIER: u32 = 2;

#[derive(Clone, Debug)]
pub enum IrcAction {
    SendMsg(String),
}

/// Manages IRC connection state for reconnection handling
struct IrcConnectionState {
    sender: Option<irc::client::Sender>,
}

impl IrcConnectionState {
    fn new() -> Self {
        Self { sender: None }
    }
}

pub async fn init(bus: &EventBus, config: &crate::config::Config) -> Result<()> {
    let connection_state = Arc::new(RwLock::new(IrcConnectionState::new()));

    // Spawn the connection manager that handles reconnection with exponential backoff
    start_connection_manager(bus.clone(), config.clone(), connection_state.clone());

    // Spawn the outgoing message handler
    start_outgoing_message_handler(bus.clone(), config.irc.channel.clone(), connection_state);

    Ok(())
}

/// Manages the IRC connection lifecycle with automatic reconnection using exponential backoff.
///
/// When the connection drops, it will attempt to reconnect with increasing delays:
/// - 1st attempt: 1 second
/// - 2nd attempt: 2 seconds
/// - 3rd attempt: 4 seconds
/// - ... up to a maximum of 30 seconds between attempts
fn start_connection_manager(
    bus: EventBus,
    config: crate::config::Config,
    connection_state: Arc<RwLock<IrcConnectionState>>,
) {
    tokio::spawn(async move {
        let mut reconnect_delay = RECONNECT_INITIAL_DELAY;

        loop {
            info!("Attempting to connect to IRC server: {}", config.irc.server);

            match connect_and_run(&bus, &config, &connection_state).await {
                Ok(()) => {
                    // Connection closed gracefully (unlikely in normal operation)
                    info!("IRC connection closed, will reconnect");
                }
                Err(e) => {
                    error!("IRC connection error: {:?}", e);
                }
            }

            // Clear the sender since we're disconnected
            {
                let mut state = connection_state.write().await;
                state.sender = None;
            }

            // Wait before reconnecting with exponential backoff
            warn!(
                "Reconnecting to IRC in {} seconds...",
                reconnect_delay.as_secs()
            );
            tokio::time::sleep(reconnect_delay).await;

            // Increase delay for next attempt (exponential backoff)
            reconnect_delay = std::cmp::min(
                reconnect_delay * RECONNECT_BACKOFF_MULTIPLIER,
                RECONNECT_MAX_DELAY,
            );
        }
    });
}

/// Establishes an IRC connection and processes incoming messages until disconnection.
/// Returns Ok(()) on graceful disconnect, Err on connection failure.
async fn connect_and_run(
    bus: &EventBus,
    config: &crate::config::Config,
    connection_state: &Arc<RwLock<IrcConnectionState>>,
) -> Result<()> {
    let irc_config = Config {
        nickname: Some(config.irc.nickname.clone()),
        server: Some(config.irc.server.clone()),
        channels: vec![config.irc.channel.clone()],
        ..Default::default()
    };

    let mut client = Client::from_config(irc_config).await?;
    let irc_sender = client.sender();

    client.identify()?;

    // Store the sender for outgoing messages
    {
        let mut state = connection_state.write().await;
        state.sender = Some(irc_sender);
    }

    info!(
        "Successfully connected to IRC server: {}",
        config.irc.server
    );

    let mut stream = client.stream()?;
    let irc_channel = config.irc.channel.clone();

    // Process incoming IRC messages until the stream ends
    while let Some(message_result) = stream.next().await {
        match message_result {
            Ok(message) => {
                let target = message.response_target().map(|s| s.to_string());

                let irc_channel = irc_channel.clone();
                let bus = bus.clone();
                let config = config.clone();

                // Process message in a separate task to avoid blocking the stream
                tokio::spawn(async move {
                    let action = message_to_action(&message, &config).await;

                    // Dispatch if msg resulted in action and msg is from target irc_channel
                    if let Some(action) = action {
                        if target == Some(irc_channel) {
                            bus.send(action);
                        }
                    }
                });
            }
            Err(e) => {
                error!("Error receiving IRC message: {:?}", e);
                // Connection error - break out to trigger reconnection
                return Err(e.into());
            }
        }
    }

    // Stream ended - connection was closed
    Ok(())
}

/// Handles outgoing IRC messages from the event bus.
/// If the connection is down, messages are logged and dropped.
fn start_outgoing_message_handler(
    bus: EventBus,
    irc_channel: String,
    connection_state: Arc<RwLock<IrcConnectionState>>,
) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await;

            if let Event::Irc(IrcAction::SendMsg(msg)) = event {
                let state = connection_state.read().await;

                if let Some(sender) = &state.sender {
                    let result = sender.send_privmsg(&irc_channel, &msg);

                    if let Err(e) = result {
                        error!("Error while sending IRC message: {:?}", e);
                    }
                } else {
                    warn!("Cannot send IRC message - not connected: {}", msg);
                }
            }
        }
    });
}

async fn message_to_action(message: &Message, config: &crate::config::Config) -> Option<Event> {
    if let Command::PRIVMSG(_channel, text) = &message.command {
        let nick = message.source_nickname()?.to_string();

        // Create an iterator over the words in the message
        let mut cmd_split = text.split_whitespace();

        // Advance the iterator by one to get the first word as the command
        let cmd = cmd_split.next()?;

        match cmd {
            "!play" | "!p" => {
                let words: Vec<&str> = cmd_split.collect();
                let url_or_search_terms = words.join(" ");

                let matches_songbook_url =
                    config.songbook.songbook_re.is_match(&url_or_search_terms);

                if matches_songbook_url {
                    return Some(Event::Songleader(SongleaderAction::RequestSongUrl {
                        url: url_or_search_terms,
                        queued_by: nick,
                    }));
                }

                if url_or_search_terms.is_empty() {
                    return Some(Event::Irc(IrcAction::SendMsg(
                        "Error: Missing YouTube URL or search terms! Usage: !play <URL or search terms>"
                            .to_string(),
                    )));
                }

                let song = get_yt_song_info(url_or_search_terms.to_string(), nick).await;

                match song {
                    Ok(song) if song.duration > MAX_SONG_DURATION.as_secs() => {
                        Some(Event::Irc(IrcAction::SendMsg(format!(
                            "Requested song is too long! Max duration is {} minutes.",
                            MAX_SONG_DURATION.as_secs() / 60
                        ))))
                    }
                    Ok(song) => Some(Event::Playback(PlaybackAction::Enqueue { song })),
                    Err(e) => Some(Event::Irc(IrcAction::SendMsg(format!(
                        "Error while getting song info: {e}"
                    )))),
                }
            }
            "!queue" | "!q" | "!np" => {
                let offset = cmd_split.next();
                let offset = offset.and_then(|offset| offset.parse().ok());

                Some(Event::Playback(PlaybackAction::ListQueue { offset }))
            }
            "!rm" => Some(Event::Playback(PlaybackAction::RmSongByNick { nick })),
            "!speak" | "!say" => {
                let words: Vec<&str> = cmd_split.collect();
                let text = words.join(" ");

                Some(Event::TextToSpeech(TextToSpeechAction::Speak {
                    text,
                    prio: Priority::Low,
                }))
            }
            "!request" | "!req" | "!r" | "!add" => {
                let words: Vec<&str> = cmd_split.collect();
                let song = words.join(" ");

                Some(Event::Songleader(SongleaderAction::RequestSongUrl {
                    url: song,
                    queued_by: nick,
                }))
            }
            "!tempo" | "tempo" => Some(Event::Songleader(SongleaderAction::Tempo { nick })),
            "!bingo" | "bingo" => Some(Event::Songleader(SongleaderAction::Bingo { nick })),
            "!skål" | "skål" => Some(Event::Songleader(SongleaderAction::Skål)),
            "!ls" => Some(Event::Songleader(SongleaderAction::ListSongs)),
            "!help" => Some(Event::Songleader(SongleaderAction::Help)),

            // "Admin" commands for songleader
            "!song" | "!sing" => {
                let subcommand = cmd_split.next()?;

                match subcommand {
                    "force-request" => {
                        let title: Vec<&str> = cmd_split.collect();
                        let title = title.join(" ");

                        if title.is_empty() {
                            Some(Event::Irc(IrcAction::SendMsg(
                                "Error: Missing song name! Usage: !song force-request <song name>"
                                    .to_string(),
                            )))
                        } else {
                            let song = SongbookSong {
                                id: title.to_string(),
                                url: None,
                                title: Some(title.to_string()),
                                book: None,
                                queued_by: Some(nick),
                            };
                            Some(Event::Songleader(SongleaderAction::RequestSong { song }))
                        }
                    }
                    "force-tempo-mode" | "resume" => {
                        Some(Event::Songleader(SongleaderAction::ForceTempo))
                    }
                    "force-bingo-mode" => Some(Event::Songleader(SongleaderAction::ForceBingo)),
                    "force-singing-mode" => Some(Event::Songleader(SongleaderAction::ForceSinging)),
                    "pause" => Some(Event::Songleader(SongleaderAction::Pause)),
                    "end" | "finish" => Some(Event::Songleader(SongleaderAction::End)),
                    "begin" => Some(Event::Songleader(SongleaderAction::Begin)),
                    "list" | "queue" => Some(Event::Songleader(SongleaderAction::ListSongs)),
                    "rm" => {
                        let id: Vec<&str> = cmd_split.collect();
                        let id = id.join(" ");

                        if id.is_empty() {
                            return Some(Event::Songleader(SongleaderAction::RmSongByNick {
                                nick,
                            }));
                        }

                        Some(Event::Songleader(SongleaderAction::RmSongById { id }))
                    }
                    _ => None,
                }
            }

            // "Admin" commands for music playback
            "!music" | "!playback" => {
                let subcommand = cmd_split.next()?;

                match subcommand {
                    "next" | "skip" => Some(Event::Playback(PlaybackAction::Next)),
                    "prev" => Some(Event::Playback(PlaybackAction::Prev)),
                    "play" | "resume" => Some(Event::Playback(PlaybackAction::Play)),
                    "pause" => Some(Event::Playback(PlaybackAction::Pause)),
                    "rm" => {
                        let pos_or_nick = cmd_split.next();

                        match pos_or_nick {
                            Some(pos_or_nick) => {
                                let pos = pos_or_nick.parse().ok();

                                match pos {
                                    Some(pos) => {
                                        Some(Event::Playback(PlaybackAction::RmSongByPos { pos }))
                                    }
                                    None => Some(Event::Playback(PlaybackAction::RmSongByNick {
                                        nick: pos_or_nick.to_string(),
                                    })),
                                }
                            }
                            None => Some(Event::Playback(PlaybackAction::RmSongByNick { nick })),
                        }
                    }
                    "volume" => {
                        let volume: f64 =
                            cmd_split.next().and_then(|volume| volume.parse().ok())?;
                        let volume = volume.clamp(0.0, 1.0);

                        Some(Event::Mixer(MixerAction::SetSecondaryChannelVolume(volume)))
                    }
                    "volume-ducked" => {
                        let volume: f64 =
                            cmd_split.next().and_then(|volume| volume.parse().ok())?;
                        let volume = volume.clamp(0.0, 1.0);

                        Some(Event::Mixer(MixerAction::SetSecondaryChannelDuckedVolume(
                            volume,
                        )))
                    }
                    "!queue" | "!q" => {
                        let offset = cmd_split.next();
                        let offset = offset.and_then(|offset| offset.parse().ok());

                        Some(Event::Playback(PlaybackAction::ListQueue { offset }))
                    }

                    _ => None,
                }
            }
            _ => None,
        }
    } else {
        None
    }
}
