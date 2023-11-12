use crate::{
    bus::{Event, EventBus},
    playback::{PlaybackAction, Song},
    songleader::SongleaderAction,
    sources::espeak::{Priority, TextToSpeechAction},
};
use anyhow::{Context, Result};
use futures::StreamExt;
use irc::client::prelude::*;
use lazy_static::lazy_static;
use regex::Regex;

#[derive(Clone, Debug)]
pub enum IrcAction {
    SendMsg(String),
}

pub async fn init(bus: &EventBus) -> Result<()> {
    let config = Config::load("Config.toml")?;
    let irc_channel = config
        .channels
        .first()
        .context("Expected channels config to be nonempty")?
        .clone();
    let mut client = Client::from_config(config).await?;

    let irc_sender = client.sender();

    client.identify()?;

    let mut stream = client.stream()?;

    {
        let irc_channel = irc_channel.clone();
        let bus = bus.clone();

        // Loop over incoming IRC messages
        tokio::spawn(async move {
            while let Ok(Some(message)) = stream.next().await.transpose() {
                let action = message_to_action(&message);
                let target = message.response_target();

                // Dispatch if msg resulted in action and msg is from target irc_channel
                if let Some(action) = action {
                    if target == Some(&irc_channel) {
                        bus.send(action).unwrap();
                    }
                }
            }
        });
    }

    {
        // Loop over incoming bus messages
        let bus = bus.clone();

        tokio::spawn(async move {
            let mut bus = bus.subscribe();

            loop {
                let event = bus.recv().await.unwrap();

                if let Event::Irc(IrcAction::SendMsg(msg)) = event {
                    let result = irc_sender.send_privmsg(&irc_channel, &msg);

                    if let Err(e) = result {
                        eprintln!("Error while sending IRC message: {:?}", e);
                    }
                }
            }
        });
    }

    Ok(())
}

fn message_to_action(message: &Message) -> Option<Event> {
    if let Command::PRIVMSG(_channel, text) = &message.command {
        let nick = message.source_nickname()?.to_string();

        // Create an iterator over the words in the message
        let mut cmd_split = text.split_whitespace();

        // Advance the iterator by one to get the first word as the command
        let cmd = cmd_split.next()?;

        match cmd {
            "!p" => {
                let url = cmd_split.next()?;
                lazy_static! {
                    static ref VIDEO_ID_REGEX: Regex =
                        Regex::new(r".*(?:youtu.be/|v/|u/\w/|embed/|watch\?v=)([^#\&\?]*).*")
                            .unwrap();
                };
                let caps = VIDEO_ID_REGEX.captures(url)?;
                let video_id = caps.get(1)?.as_str();
                let song = Song {
                    video_id: video_id.to_string(),
                    url: url.to_string(),
                    queued_by: nick,
                };

                Some(Event::Playback(PlaybackAction::Enqueue { song }))
            }
            "!q" => Some(Event::Playback(PlaybackAction::ListQueue)),
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

                Some(Event::Songleader(SongleaderAction::RequestSong { song }))
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
                    "tempo" => Some(Event::Songleader(SongleaderAction::ForceTempo)),
                    "bingo" => Some(Event::Songleader(SongleaderAction::ForceBingo)),
                    "singing" => Some(Event::Songleader(SongleaderAction::ForceSinging)),
                    "pause" => Some(Event::Songleader(SongleaderAction::Pause)),
                    "end" => Some(Event::Songleader(SongleaderAction::End)),
                    "begin" => Some(Event::Songleader(SongleaderAction::Begin)),
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
                    _ => None,
                }
            }
            _ => None,
        }
    } else {
        None
    }
}
