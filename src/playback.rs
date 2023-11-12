use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::{
    bus::{Event, EventBus},
    irc::IrcAction,
    sources::symphonia::SymphoniaAction,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Song {
    pub url: String,
    pub video_id: String,
    pub queued_by: String,
}

#[derive(Clone, Debug)]
pub enum PlaybackAction {
    Enqueue { song: Song },
    EndOfSong,
    ListQueue,
    Play,
    Pause,
    Prev,
    Next,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlaybackState {
    played_songs: Vec<Song>,
    queued_songs: Vec<Song>,

    /// Whether the client has had a song loaded or not
    song_loaded: bool,

    /// Whether a song is currently being played by the client
    is_playing: bool,

    /// Whether we should start playing if queue empty and a new song is
    /// enqueued
    should_play: bool,
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState {
            played_songs: vec![],
            queued_songs: vec![],
            song_loaded: false,
            is_playing: false,
            should_play: true,
        }
    }
}

impl PlaybackState {
    async fn read_or_default() -> Self {
        let res = tokio::fs::read("state.json").await;

        match res {
            Ok(res) => serde_json::from_slice(&res).unwrap_or_default(),
            Err(e) => {
                eprintln!("Error while reading playback state: {:?}", e);
                eprintln!("Falling back to default state.");
                PlaybackState::default()
            }
        }
    }

    fn persist(&self) {
        let json = serde_json::to_string(&self);

        if let Ok(json) = json {
            tokio::spawn(async move {
                let res = tokio::fs::write("state.json", json).await;

                if let Err(e) = res {
                    eprintln!("Error while writing state to disk: {:?}", e);
                }
            });
        }
    }
}

#[derive(Clone)]
pub struct Playback {
    bus: EventBus,
    state: PlaybackState,
}

impl Playback {
    pub async fn new(bus: EventBus) -> Playback {
        let state = PlaybackState::read_or_default().await;

        println!("Initial playback state:\n{:#?}", state);

        Playback { bus, state }
    }

    /// Convenience method for sending irc messages
    fn irc_say(&self, msg: &str) {
        self.bus
            .send(Event::Irc(IrcAction::SendMsg(msg.to_string())))
            .unwrap();
    }

    fn enqueue(&mut self, song: Song) {
        let queue_was_empty = self.state.queued_songs.is_empty();

        self.state.queued_songs.push(song.clone());

        let msg = format!("Added {} to the queue.", song.video_id);
        self.irc_say(&msg);

        if !self.state.is_playing && self.state.should_play && queue_was_empty {
            self.play_song(song)
        }

        self.state.persist()
    }

    fn list_queue(&self) {
        let fmt_song = |song: Option<&Song>| {
            song.map(|song| format!("{} (queued by {})", song.url, song.queued_by))
                .unwrap_or_else(|| "(nothing)".to_string())
        };

        let is_empty = self.state.queued_songs.is_empty();
        let np = fmt_song(self.state.queued_songs.get(0));
        let next = fmt_song(self.state.queued_songs.get(1));

        let msg = if is_empty {
            "Queue is empty!".to_string()
        } else {
            format!("Now playing: {}, next up: {}", np, next)
        };

        self.irc_say(&msg);
    }

    fn play_song(&mut self, song: Song) {
        self.state.is_playing = true;
        self.state.song_loaded = true;

        self.bus
            .send(Event::Symphonia(SymphoniaAction::PlayFile {
                file_path: song.url,
            }))
            .unwrap();

        self.list_queue();
        self.state.persist();
    }

    fn end_of_queue(&mut self) {
        self.state.is_playing = false;

        // dispatch_app_action(
        //     &self.bus,
        //     AppAction::ControlClient(WsMessageFromServer::Pause),
        // );

        self.irc_say("Playback queue ended.");
        self.state.persist()
    }

    fn next(&mut self) {
        if !self.state.queued_songs.is_empty() {
            // Move now playing song to played_songs
            let song = self.state.queued_songs.remove(0);
            self.state.played_songs.push(song);
        }

        if self.state.queued_songs.is_empty() {
            self.end_of_queue();
        } else {
            // Play next song if it exists
            let song = self.state.queued_songs.get(0).cloned();
            if let Some(song) = song {
                self.play_song(song);
            }
        }
        self.state.persist()
    }

    fn prev(&mut self) {
        let song = self.state.played_songs.pop();

        if let Some(song) = song {
            self.state.queued_songs.insert(0, song.clone());
            self.play_song(song);
        } else {
            self.end_of_queue()
        }
        self.state.persist()
    }
}

pub async fn init(bus: &EventBus) {
    let playback = Arc::new(RwLock::new(Playback::new(bus.clone()).await));

    handle_incoming_event_loop(bus.clone(), playback);
}

fn handle_incoming_event_loop(bus: EventBus, playback: Arc<RwLock<Playback>>) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await.unwrap();

            if let Event::Playback(action) = event {
                let playback = playback.clone();
                tokio::spawn(async move {
                    handle_incoming_event(action, playback).await;
                });
            }
        }
    });
}

async fn handle_incoming_event(action: PlaybackAction, playback: Arc<RwLock<Playback>>) {
    let mut playback = playback.write().await;
    match action {
        PlaybackAction::Enqueue { song } => playback.enqueue(song),
        PlaybackAction::ListQueue => {
            playback.list_queue();
        }
        PlaybackAction::Play => {
            playback.state.is_playing = true;
            playback.state.should_play = true;

            if !playback.state.song_loaded {
                // Play next song if it exists
                let song = playback.state.queued_songs.get(0).cloned();
                if let Some(song) = song {
                    playback.play_song(song);
                }
            } else {
                playback
                    .bus
                    .send(Event::Symphonia(SymphoniaAction::Resume))
                    .unwrap();
            }

            playback.state.persist();
        }
        PlaybackAction::Pause => {
            playback.state.is_playing = false;
            playback.state.should_play = false;

            playback
                .bus
                .send(Event::Symphonia(SymphoniaAction::Pause))
                .unwrap();

            playback.state.persist();
        }
        PlaybackAction::EndOfSong => {
            playback.state.is_playing = false;
            playback.state.song_loaded = false;
            playback.next();
        }
        PlaybackAction::Next => {
            playback.next();
        }
        PlaybackAction::Prev => {
            playback.prev();
        }
    }
}
