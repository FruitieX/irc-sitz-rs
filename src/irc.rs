use crate::{
    config::IrcConfig,
    event::{Event, EventBus},
    message::{MessageAction, Platform},
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
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio::time::sleep;

/// Initial delay before first reconnection attempt
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
/// Maximum delay between reconnection attempts
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30); // 30 seconds
/// Multiplier for exponential backoff
const RECONNECT_BACKOFF_MULTIPLIER: u32 = 2;
/// Anti-flood delay between messages
const ANTI_FLOOD_DELAY: Duration = Duration::from_millis(1200);

#[derive(Clone, Debug)]
pub enum IrcAction {
    SendMsg(String),
}

/// Manages IRC connection state for reconnection handling
struct IrcConnectionState {
    sender: Option<irc::client::Sender>,
    /// Set of channel operators (nicks with @ prefix)
    operators: HashSet<String>,
}

impl IrcConnectionState {
    fn new() -> Self {
        Self {
            sender: None,
            operators: HashSet::new(),
        }
    }
}

pub async fn init(
    bus: &EventBus,
    config: &crate::config::Config,
    irc_config: &IrcConfig,
) -> Result<()> {
    let connection_state = Arc::new(RwLock::new(IrcConnectionState::new()));

    // Spawn the connection manager that handles reconnection with exponential backoff
    start_connection_manager(
        bus.clone(),
        config.clone(),
        irc_config.clone(),
        connection_state.clone(),
    );

    // Spawn the outgoing message handler
    start_outgoing_message_handler(
        bus.clone(),
        irc_config.irc_channel.clone(),
        connection_state,
    );

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
    irc_config: IrcConfig,
    connection_state: Arc<RwLock<IrcConnectionState>>,
) {
    tokio::spawn(async move {
        let mut reconnect_delay = RECONNECT_INITIAL_DELAY;

        loop {
            let use_tls = irc_config.irc_use_tls.unwrap_or(false);
            let default_port = if use_tls { 6697 } else { 6667 };
            let port = irc_config.irc_port.unwrap_or(default_port);
            info!(
                "Attempting to connect to IRC server: {}:{} (TLS: {})",
                irc_config.irc_server, port, use_tls
            );

            match connect_and_run(&bus, &config, &irc_config, &connection_state).await {
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
    irc_config: &IrcConfig,
    connection_state: &Arc<RwLock<IrcConnectionState>>,
) -> Result<()> {
    let use_tls = irc_config.irc_use_tls.unwrap_or(false);
    let default_port = if use_tls { 6697 } else { 6667 };
    let port = irc_config.irc_port.unwrap_or(default_port);

    // Generate alternative nicknames with numbers appended
    let base_nick = &irc_config.irc_nickname;
    let alt_nicks: Vec<String> = (1..=9).map(|i| format!("{base_nick}{i}")).collect();

    let irc_client_config = Config {
        nickname: Some(irc_config.irc_nickname.clone()),
        alt_nicks,
        server: Some(irc_config.irc_server.clone()),
        port: Some(port),
        channels: vec![irc_config.irc_channel.clone()],
        use_tls: Some(use_tls),
        ..Default::default()
    };

    let mut client = Client::from_config(irc_client_config).await?;
    let irc_sender = client.sender();

    client.identify()?;

    // Store the sender for outgoing messages
    {
        let mut state = connection_state.write().await;
        state.sender = Some(irc_sender);
    }

    info!(
        "Successfully connected to IRC server: {}",
        irc_config.irc_server
    );

    let mut stream = client.stream()?;
    let irc_channel = irc_config.irc_channel.clone();

    // Process incoming IRC messages until the stream ends
    while let Some(message_result) = stream.next().await {
        match message_result {
            Ok(message) => {
                // Handle NAMES reply to track channel operators
                if let Command::Response(Response::RPL_NAMREPLY, args) = &message.command {
                    // args[2] is the channel, args[3] is the space-separated list of nicks
                    if args.len() >= 4 && args[2].eq_ignore_ascii_case(&irc_channel) {
                        let mut state = connection_state.write().await;
                        for nick in args[3].split_whitespace() {
                            if let Some(stripped) = nick.strip_prefix('@') {
                                state.operators.insert(stripped.to_lowercase());
                            } else if let Some(stripped) = nick.strip_prefix('+') {
                                // +nick is voiced, not op - ignore the prefix
                                state.operators.remove(&stripped.to_lowercase());
                            } else {
                                state.operators.remove(&nick.to_lowercase());
                            }
                        }
                        debug!("Updated operators: {:?}", state.operators);
                    }
                    continue;
                }

                let target = message.response_target().map(|s| s.to_string());

                let irc_channel = irc_channel.clone();
                let bus = bus.clone();
                let config = config.clone();
                let connection_state = connection_state.clone();

                // Process message in a separate task to avoid blocking the stream
                tokio::spawn(async move {
                    // Only process messages from the target channel
                    if target != Some(irc_channel) {
                        return;
                    }

                    // Check if user is operator
                    let is_operator = if let Some(nick) = message.source_nickname() {
                        let state = connection_state.read().await;
                        state.operators.contains(&nick.to_lowercase())
                    } else {
                        false
                    };

                    // Try to parse as a command
                    let action = message_to_action(&message, &config, is_operator).await;

                    if let Some(action) = action {
                        if let Command::PRIVMSG(_channel, text) = &message.command {
                            if let Some(nick) = message.source_nickname() {
                                info!(
                                    "IRC command from {nick}{}: {text}",
                                    if is_operator { " (op)" } else { "" }
                                );

                                // Mirror tempo/bingo/skål/speak/request commands to Discord as user messages
                                let cmd = text.split_whitespace().next().unwrap_or("");
                                if matches!(
                                    cmd,
                                    "!tempo"
                                        | "tempo"
                                        | "!bingo"
                                        | "bingo"
                                        | "!skål"
                                        | "skål"
                                        | "!speak"
                                        | "!say"
                                        | "!request"
                                        | "!req"
                                        | "!r"
                                        | "!rm"
                                ) {
                                    bus.send_message(MessageAction::Mirror {
                                        username: nick.to_string(),
                                        text: text.clone(),
                                        source: Platform::Irc,
                                    });
                                }

                                // Mirror successful !play commands to Discord
                                if matches!(cmd, "!play" | "!p")
                                    && matches!(
                                        action,
                                        Event::Playback(PlaybackAction::Enqueue { .. })
                                    )
                                {
                                    bus.send_message(MessageAction::Mirror {
                                        username: nick.to_string(),
                                        text: text.clone(),
                                        source: Platform::Irc,
                                    });
                                }
                            }
                        }
                        bus.send(action);
                    } else {
                        // If not a command, mirror to other platforms
                        if let Command::PRIVMSG(_channel, text) = &message.command {
                            if let Some(nick) = message.source_nickname() {
                                bus.send_message(MessageAction::Mirror {
                                    username: nick.to_string(),
                                    text: text.clone(),
                                    source: Platform::Irc,
                                });
                            }
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

            // Handle platform-agnostic Message events
            if let Event::Message(action) = event {
                match action {
                    MessageAction::Send { text, source, .. } => {
                        // Don't echo messages back to IRC if they came from IRC
                        if source == Platform::Irc {
                            continue;
                        }
                        send_multiline_message(&text, &irc_channel, &connection_state).await;
                    }
                    MessageAction::Mirror {
                        username,
                        text,
                        source,
                    } => {
                        // Only mirror messages from other platforms to IRC
                        if source != Platform::Irc {
                            let msg = format!(
                                "<{source}:{username}> {text}",
                                source = match source {
                                    #[cfg(feature = "discord")]
                                    Platform::Discord => "discord",
                                    Platform::Bot => "bot",
                                    Platform::Irc => unreachable!(),
                                }
                            );
                            send_message(&msg, &irc_channel, &connection_state).await;
                        }
                    }
                    #[cfg(feature = "discord")]
                    MessageAction::StoreBingoMessageId { .. } => {}
                }
            }
            // Keep handling legacy IrcAction events for backwards compatibility
            else if let Event::Irc(IrcAction::SendMsg(msg)) = event {
                send_message(&msg, &irc_channel, &connection_state).await;
            }
        }
    });
}

async fn send_message(
    msg: &str,
    irc_channel: &str,
    connection_state: &Arc<RwLock<IrcConnectionState>>,
) {
    let state = connection_state.read().await;

    if let Some(sender) = &state.sender {
        let result = sender.send_privmsg(irc_channel, msg);

        if let Err(e) = result {
            error!("Error while sending IRC message: {:?}", e);
        }
    } else {
        warn!("Cannot send IRC message - not connected: {}", msg);
    }
}

async fn send_multiline_message(
    msg: &str,
    irc_channel: &str,
    connection_state: &Arc<RwLock<IrcConnectionState>>,
) {
    for line in msg.lines() {
        if !line.is_empty() {
            send_message(line, irc_channel, connection_state).await;
            sleep(ANTI_FLOOD_DELAY).await;
        }
    }
}

/// Parses an IRC message and returns the corresponding Event if it's a command.
/// This function is public for testing purposes.
pub async fn message_to_action(
    message: &Message,
    config: &crate::config::Config,
    is_operator: bool,
) -> Option<Event> {
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

            // "Admin" commands for songleader (require channel operator)
            "!song" | "!sing" => {
                if !is_operator {
                    return Some(Event::Irc(IrcAction::SendMsg(
                        "Error: This command requires channel operator status".to_string(),
                    )));
                }

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

            // "Admin" commands for music playback (require channel operator)
            "!music" | "!playback" => {
                if !is_operator {
                    return Some(Event::Irc(IrcAction::SendMsg(
                        "Error: This command requires channel operator status".to_string(),
                    )));
                }

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
