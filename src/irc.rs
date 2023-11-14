use crate::{
    event::{Event, EventBus},
    playback::{PlaybackAction, MAX_SONG_DURATION},
    songbook::SongbookSong,
    songleader::SongleaderAction,
    sources::espeak::{Priority, TextToSpeechAction},
    youtube::get_yt_song_info,
};
use anyhow::Result;
use futures::StreamExt;
use irc::client::prelude::*;

#[derive(Clone, Debug)]
pub enum IrcAction {
    SendMsg(String),
}

pub async fn init(bus: &EventBus, config: &crate::config::Config) -> Result<()> {
    let irc_config = Config {
        nickname: Some(config.irc.nickname.clone()),
        server: Some(config.irc.server.clone()),
        channels: vec![config.irc.channel.clone()],
        ..Default::default()
    };

    let irc_channel = config.irc.channel.clone();

    let mut client = Client::from_config(irc_config).await?;

    let irc_sender = client.sender();

    client.identify()?;

    let mut stream = client.stream()?;

    {
        let irc_channel = irc_channel.clone();
        let bus = bus.clone();

        // Loop over incoming IRC messages
        tokio::spawn(async move {
            while let Ok(Some(message)) = stream.next().await.transpose() {
                let target = message.response_target().map(|s| s.to_string());
                let message = message.clone();

                let irc_channel = irc_channel.clone();
                let bus = bus.clone();
                tokio::spawn(async move {
                    let action = message_to_action(&message).await;

                    // Dispatch if msg resulted in action and msg is from target irc_channel
                    if let Some(action) = action {
                        if target == Some(irc_channel) {
                            bus.send(action);
                        }
                    }
                });
            }
        });
    }

    {
        // Loop over incoming bus messages
        let bus = bus.clone();

        tokio::spawn(async move {
            let mut bus = bus.subscribe();

            loop {
                let event = bus.recv().await;

                if let Event::Irc(IrcAction::SendMsg(msg)) = event {
                    let result = irc_sender.send_privmsg(&irc_channel, &msg);

                    if let Err(e) = result {
                        error!("Error while sending IRC message: {:?}", e);
                    }
                }
            }
        });
    }

    Ok(())
}

async fn message_to_action(message: &Message) -> Option<Event> {
    if let Command::PRIVMSG(_channel, text) = &message.command {
        let nick = message.source_nickname()?.to_string();

        // Create an iterator over the words in the message
        let mut cmd_split = text.split_whitespace();

        // Advance the iterator by one to get the first word as the command
        let cmd = cmd_split.next()?;

        match cmd {
            "!p" => {
                let words: Vec<&str> = cmd_split.collect();
                let url_or_search_terms = words.join(" ");
                let song = get_yt_song_info(url_or_search_terms.to_string(), nick).await;

                match song {
                    Ok(song) if song.duration > MAX_SONG_DURATION.as_secs() => {
                        Some(Event::Irc(IrcAction::SendMsg(format!(
                            "Requested song is too long! Max duration is {} seconds.",
                            MAX_SONG_DURATION.as_secs()
                        ))))
                    }
                    Ok(song) => Some(Event::Playback(PlaybackAction::Enqueue { song })),
                    Err(e) => Some(Event::Irc(IrcAction::SendMsg(format!(
                        "Error while getting song info: {e}"
                    )))),
                }
            }
            "!q" => {
                let offset = cmd_split.next();
                let offset = offset.and_then(|offset| offset.parse().ok());

                Some(Event::Playback(PlaybackAction::ListQueue { offset }))
            }
            "!speak" => {
                let words: Vec<&str> = cmd_split.collect();
                let text = words.join(" ");

                Some(Event::TextToSpeech(TextToSpeechAction::Speak {
                    text,
                    prio: Priority::Low,
                }))
            }
            "!request" => {
                let words: Vec<&str> = cmd_split.collect();
                let song = words.join(" ");

                Some(Event::Songleader(SongleaderAction::RequestSongUrl {
                    url: song,
                }))
            }
            "!tempo" => Some(Event::Songleader(SongleaderAction::Tempo { nick })),
            "!bingo" => Some(Event::Songleader(SongleaderAction::Bingo { nick })),
            "!skål" => Some(Event::Songleader(SongleaderAction::Skål)),
            "!ls" => Some(Event::Songleader(SongleaderAction::ListSongs)),
            "!help" => Some(Event::Songleader(SongleaderAction::Help)),

            // "Admin" commands for songleader
            "!song" => {
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
                            };
                            Some(Event::Songleader(SongleaderAction::RequestSong(song)))
                        }
                    }
                    "force-tempo-mode" | "resume" => {
                        Some(Event::Songleader(SongleaderAction::ForceTempo))
                    }
                    "force-bingo-mode" => Some(Event::Songleader(SongleaderAction::ForceBingo)),
                    "force-singing-mode" => Some(Event::Songleader(SongleaderAction::ForceSinging)),
                    "pause" => Some(Event::Songleader(SongleaderAction::Pause)),
                    "end" => Some(Event::Songleader(SongleaderAction::End)),
                    "begin" => Some(Event::Songleader(SongleaderAction::Begin)),
                    "rm" => {
                        let id = cmd_split.next().map(|s| s.to_string());

                        match id {
                            Some(id) => Some(Event::Songleader(SongleaderAction::RmSong { id })),
                            None => Some(Event::Irc(IrcAction::SendMsg(
                                "Error: Missing song ID! Usage: !song rm <song ID>".to_string(),
                            ))),
                        }
                    }
                    _ => None,
                }
            }

            // "Admin" commands for music playback
            "!music" => {
                let subcommand = cmd_split.next()?;

                match subcommand {
                    "next" => Some(Event::Playback(PlaybackAction::Next)),
                    "prev" => Some(Event::Playback(PlaybackAction::Prev)),
                    "play" => Some(Event::Playback(PlaybackAction::Play)),
                    "pause" => Some(Event::Playback(PlaybackAction::Pause)),
                    "rm" => {
                        let offset = cmd_split.next();
                        let offset = offset.and_then(|offset| offset.parse().ok());

                        match offset {
                            Some(offset) => {
                                Some(Event::Playback(PlaybackAction::RmSong { offset }))
                            }
                            None => Some(Event::Irc(IrcAction::SendMsg(
                                "Error: Missing offset! Usage: !music rm <offset>".to_string(),
                            ))),
                        }
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
