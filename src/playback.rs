use crate::{
    event::{Event, EventBus},
    irc::IrcAction,
    sources::symphonia::SymphoniaAction,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};
use tokio::sync::RwLock;

const PLAYBACK_STATE_FILE: &str = "playback_state.json";
pub const MAX_SONG_DURATION: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Song {
    pub id: String,
    pub url: String,
    pub title: String,
    pub channel: String,
    pub duration: u64,
    pub queued_by: String,
}

impl PartialEq for Song {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

#[derive(Clone, Debug)]
pub enum PlaybackAction {
    /// Add song at the end of the queue
    Enqueue { song: Song },

    /// Player reached end of song
    EndOfSong,

    /// List either the first items in a queue or an item at a specific position
    ListQueue { offset: Option<usize> },

    /// Removes song by position
    RmSongByPos { pos: usize },

    /// Removes latest song queued by nick
    RmSongByNick { nick: String },

    /// Resumes playback
    Play,

    /// Pauses playback
    Pause,

    /// Play previous song
    Prev,

    /// Play next song
    Next,

    /// Notification that playback has progressed
    PlaybackProgress { position: u64 },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PlaybackState {
    played_songs: Vec<Song>,
    queued_songs: Vec<Song>,

    #[serde(skip_deserializing)]
    /// Whether the client has had a song loaded or not
    song_loaded: bool,

    #[serde(skip_deserializing)]
    /// Whether a song is currently being played by the client
    is_playing: bool,

    /// Whether we should start playing if queue empty and a new song is
    /// enqueued
    should_play: bool,

    /// Progress of the current song in seconds
    playback_progress: u64,
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState {
            played_songs: vec![],
            queued_songs: vec![],
            song_loaded: false,
            is_playing: false,
            should_play: true,
            playback_progress: 0,
        }
    }
}

impl PlaybackState {
    async fn read_or_default() -> Self {
        let res = tokio::fs::read(PLAYBACK_STATE_FILE).await;

        match res {
            Ok(res) => serde_json::from_slice(&res).unwrap_or_default(),
            Err(e) => {
                info!("Error while reading playback state: {:?}", e);
                info!("Falling back to default state.");
                PlaybackState::default()
            }
        }
    }

    fn persist(&self) {
        let json = serde_json::to_string_pretty(&self);

        match json {
            Ok(json) => {
                tokio::spawn(async move {
                    let res = tokio::fs::write(PLAYBACK_STATE_FILE, json).await;

                    if let Err(e) = res {
                        error!("Error while writing state to disk: {:?}", e);
                    }
                });
            }
            Err(e) => {
                error!("Error while serializing playback state: {:?}", e);
            }
        }
    }
}

#[derive(Clone)]
pub struct Playback {
    bus: EventBus,
    state: PlaybackState,
}

impl Playback {
    pub async fn create(bus: EventBus) -> Playback {
        let state = PlaybackState::read_or_default().await;

        debug!("Initial playback state:\n{:#?}", state);

        // Play next song if it exists
        let first_song = state.queued_songs.first().cloned();
        let should_play = state.should_play;

        let mut playback = Playback { bus, state };

        if should_play {
            if let Some(song) = first_song {
                playback.play_song(song);
            }
        }

        playback
    }

    /// Convenience method for sending irc messages
    fn irc_say(&self, msg: &str) {
        self.bus
            .send(Event::Irc(IrcAction::SendMsg(msg.to_string())));
    }

    fn queue_len(&self) -> usize {
        self.state.queued_songs.len()
    }

    fn queue_duration_mins(&self) -> u64 {
        self.state
            .queued_songs
            .iter()
            .map(|song| song.duration)
            .sum::<u64>()
            / 60
            - self.state.playback_progress / 60
    }

    fn enqueue(&mut self, song: Song) {
        if self.state.queued_songs.contains(&song) {
            self.irc_say("Song already in queue!");
        } else {
            let queue_was_empty = self.state.queued_songs.is_empty();
            let time_until_playback = self.queue_duration_mins();
            self.state.queued_songs.push(song.clone());

            let msg = format!(
                "Added {} {} to the queue. Time until playback: {} min",
                song.title, song.url, time_until_playback
            );
            self.irc_say(&msg);

            if !self.state.is_playing && self.state.should_play && queue_was_empty {
                self.play_song(song)
            }

            self.state.persist()
        }
    }

    fn list_queue(&self, offset: Option<usize>) {
        let fmt_song = |song: Option<&Song>| {
            song.map(|song| format!("{} (queued by {})", song.title, song.queued_by))
                .unwrap_or_else(|| "(nothing)".to_string())
        };

        let is_empty = self.state.queued_songs.is_empty();
        let np = self.state.queued_songs.first();
        let np_formatted = fmt_song(np);
        let next_formatted = fmt_song(self.state.queued_songs.get(1));
        let len = self.queue_len();
        let duration_min = self.queue_duration_mins();

        let msg = if is_empty {
            "Queue is empty!".to_string()
        } else if let Some(offset) = offset {
            let song = fmt_song(self.state.queued_songs.get(offset));
            format!("Song at position {offset}: {song}")
        } else {
            let state = if self.state.is_playing {
                "Now playing"
            } else {
                "Paused"
            };
            let np_duration = np.map(|song| song.duration).unwrap_or_default();
            let progress = format!(
                "{}:{:02}/{}:{:02}",
                self.state.playback_progress / 60,
                self.state.playback_progress % 60,
                np_duration / 60,
                np_duration % 60
            );
            format!("{state} ({progress}): {np_formatted}, next up: {next_formatted}. Queue length: {len} songs ({duration_min} min)")
        };

        self.irc_say(&msg);
    }

    fn rm_song_at_pos(&mut self, pos: usize) {
        let song = if pos == 0 {
            let song = self.state.queued_songs.first().cloned();
            self.next(true);
            song
        } else if pos < self.state.queued_songs.len() {
            Some(self.state.queued_songs.remove(pos))
        } else {
            None
        };

        match song {
            Some(song) => self.irc_say(&format!("Removed song {} from the queue", song.title)),
            None => self.irc_say(&format!("No song at position {pos} in the queue")),
        }
    }

    fn rm_latest_song_by_nick(&mut self, nick: String) {
        let index = self
            .state
            .queued_songs
            .iter()
            .rposition(|song| song.queued_by == nick);

        let song = if index == Some(0) {
            let song = self.state.queued_songs.first().cloned();
            self.next(true);
            song
        } else if let Some(index) = index {
            Some(self.state.queued_songs.remove(index))
        } else {
            None
        };

        match song {
            Some(song) => self.irc_say(&format!("Removed song {} from the queue", song.title)),
            None => self.irc_say(&format!("No songs queued by {nick}")),
        }
    }

    fn play_song(&mut self, song: Song) {
        self.state.is_playing = true;
        self.state.song_loaded = true;
        self.state.playback_progress = 0;

        self.bus.send(Event::Symphonia(SymphoniaAction::PlayYtUrl {
            url: song.url,
        }));

        self.list_queue(None);
        self.state.persist();
    }

    fn end_of_queue(&mut self) {
        self.state.is_playing = false;

        self.bus.send(Event::Symphonia(SymphoniaAction::Stop));

        self.irc_say("Playback queue ended.");
        self.state.persist()
    }

    fn next(&mut self, remove_current: bool) {
        if !self.state.queued_songs.is_empty() {
            // Move now playing song to played_songs
            let song = self.state.queued_songs.remove(0);

            if !remove_current {
                self.state.played_songs.push(song);
            }
        }

        if self.state.queued_songs.is_empty() {
            self.end_of_queue();
        } else {
            // Play next song if it exists
            let song = self.state.queued_songs.first().cloned();
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
    let playback = Arc::new(RwLock::new(Playback::create(bus.clone()).await));

    handle_incoming_event_loop(bus.clone(), playback);
}

fn handle_incoming_event_loop(bus: EventBus, playback: Arc<RwLock<Playback>>) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await;

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
        PlaybackAction::ListQueue { offset } => {
            playback.list_queue(offset);
        }
        PlaybackAction::RmSongByPos { pos } => playback.rm_song_at_pos(pos),
        PlaybackAction::RmSongByNick { nick } => playback.rm_latest_song_by_nick(nick),
        PlaybackAction::Play => {
            playback.state.should_play = true;

            if playback.state.song_loaded {
                playback.state.is_playing = true;
                playback.bus.send(Event::Symphonia(SymphoniaAction::Resume));
            } else {
                // Play next song if it exists
                let song = playback.state.queued_songs.first().cloned();
                if let Some(song) = song {
                    playback.play_song(song);
                }
            }

            playback.state.persist();
        }
        PlaybackAction::Pause => {
            playback.state.is_playing = false;
            playback.state.should_play = false;

            playback.bus.send(Event::Symphonia(SymphoniaAction::Pause));

            playback.state.persist();
        }
        PlaybackAction::EndOfSong => {
            playback.state.is_playing = false;
            playback.state.song_loaded = false;
            playback.next(false);
        }
        PlaybackAction::Next => {
            playback.next(false);
        }
        PlaybackAction::Prev => {
            playback.prev();
        }
        PlaybackAction::PlaybackProgress { position } => {
            playback.state.playback_progress = position;
        }
    }
}
