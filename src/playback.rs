use crate::{
    event::{Event, EventBus},
    message::{MessageAction, NowPlayingInfo, RichContent},
    sources::symphonia::SymphoniaAction,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::RwLock;

const PLAYBACK_STATE_FILE: &str = "playback_state.json";
const PLAYBACK_STATE_FILE_TMP: &str = "playback_state.json.tmp";
pub const MAX_SONG_DURATION: Duration = Duration::from_secs(10 * 60);

/// Vote information for a song
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SongVotes {
    /// Users who upvoted (moves song up in queue)
    pub upvotes: Vec<String>,
    /// Users who downvoted (moves song down in queue)
    pub downvotes: Vec<String>,
}

impl SongVotes {
    /// Net vote score (positive = up, negative = down)
    pub fn score(&self) -> i32 {
        self.upvotes.len() as i32 - self.downvotes.len() as i32
    }
}

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

    /// Upvote a song (moves it up in queue priority)
    Upvote { song_id: String, user: String },

    /// Downvote a song (moves it down in queue priority)
    Downvote { song_id: String, user: String },

    /// Remove upvote from a song
    RemoveUpvote { song_id: String, user: String },

    /// Remove downvote from a song
    RemoveDownvote { song_id: String, user: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PlaybackState {
    pub played_songs: Vec<Song>,
    pub queued_songs: Vec<Song>,

    /// Votes for songs in the queue (keyed by song ID)
    #[serde(default)]
    pub song_votes: HashMap<String, SongVotes>,

    #[serde(skip_deserializing)]
    /// Whether the client has had a song loaded or not
    pub song_loaded: bool,

    #[serde(skip_deserializing)]
    /// Whether a song is currently being played by the client
    pub is_playing: bool,

    /// Whether we should start playing if queue empty and a new song is
    /// enqueued
    pub should_play: bool,

    /// Progress of the current song in seconds
    pub playback_progress: u64,
}

impl Default for PlaybackState {
    fn default() -> Self {
        PlaybackState {
            played_songs: vec![],
            queued_songs: vec![],
            song_votes: HashMap::new(),
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

    /// Persists state to disk using atomic write (write to temp file, then rename).
    /// This prevents corruption if the process crashes during write.
    fn persist(&self) {
        let json = match serde_json::to_string_pretty(&self) {
            Ok(json) => json,
            Err(e) => {
                error!("Error while serializing playback state: {:?}", e);
                return;
            }
        };

        // Spawn a task to perform the atomic write
        tokio::spawn(async move {
            // Write to temp file first
            if let Err(e) = tokio::fs::write(PLAYBACK_STATE_FILE_TMP, &json).await {
                error!("Error while writing playback state to temp file: {:?}", e);
                return;
            }

            // Atomically rename temp file to actual file
            // This ensures we never have a partially written state file
            if let Err(e) = tokio::fs::rename(PLAYBACK_STATE_FILE_TMP, PLAYBACK_STATE_FILE).await {
                // NotFound is expected on first run when no state exists yet
                if e.kind() != std::io::ErrorKind::NotFound {
                    error!("Error while renaming playback state file: {:?}", e);
                }
            }
        });
    }
}

#[derive(Clone)]
pub struct Playback {
    bus: EventBus,
    pub state: PlaybackState,
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

    /// Convenience method for sending messages to all platforms
    fn say(&self, msg: &str) {
        self.bus.say(msg);
    }

    /// Convenience method for sending rich messages to all platforms
    fn say_rich(&self, text: &str, rich: RichContent) {
        self.bus.send_message(MessageAction::rich(text, rich));
    }

    fn queue_len(&self) -> usize {
        self.state.queued_songs.len().saturating_sub(1)
    }

    fn queue_duration_mins(&self) -> u64 {
        let total_secs: u64 = self
            .state
            .queued_songs
            .iter()
            .skip(1) // Skip current song
            .map(|song| song.duration)
            .sum();
        total_secs / 60
    }

    /// Get votes for a song
    pub fn get_votes(&self, song_id: &str) -> SongVotes {
        self.state
            .song_votes
            .get(song_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get upcoming songs sorted by effective position (considering votes)
    /// Returns tuples of (song, original_position, effective_position)
    /// Position 0 is the currently playing song (not affected by votes)
    pub fn get_sorted_upcoming(&self) -> Vec<(Song, usize, usize)> {
        if self.state.queued_songs.is_empty() {
            return vec![];
        }

        // First song (now playing) is always first
        let mut result = vec![];
        if let Some(first) = self.state.queued_songs.first() {
            result.push((first.clone(), 0, 0));
        }

        // For the rest, sort by vote score (higher score = earlier position)
        let mut upcoming: Vec<(Song, usize)> = self
            .state
            .queued_songs
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, song)| (song.clone(), i))
            .collect();

        upcoming.sort_by(|(song_a, pos_a), (song_b, pos_b)| {
            let score_a = self.get_votes(&song_a.id).score();
            let score_b = self.get_votes(&song_b.id).score();
            // Sort by score descending, then by original position ascending for ties
            (score_b, pos_a).cmp(&(score_a, pos_b))
        });

        // Add effective positions
        for (effective_pos, (song, original_pos)) in upcoming.into_iter().enumerate() {
            result.push((song, original_pos, effective_pos + 1));
        }

        result
    }

    fn upvote(&mut self, song_id: &str, user: &str) {
        // Prevent users from upvoting their own songs
        let is_own_song = self
            .state
            .queued_songs
            .iter()
            .any(|s| s.id == song_id && s.queued_by == user);
        if is_own_song {
            info!("User {user} tried to upvote their own song {song_id}, ignoring");
            return;
        }

        let votes = self
            .state
            .song_votes
            .entry(song_id.to_string())
            .or_default();
        // Remove from downvotes if present
        votes.downvotes.retain(|u| u != user);
        // Add to upvotes if not already there
        if !votes.upvotes.contains(&user.to_string()) {
            votes.upvotes.push(user.to_string());
            info!("User {user} upvoted song {song_id}");
        }
        self.state.persist();
    }

    fn downvote(&mut self, song_id: &str, user: &str) {
        let votes = self
            .state
            .song_votes
            .entry(song_id.to_string())
            .or_default();
        // Remove from upvotes if present
        votes.upvotes.retain(|u| u != user);
        // Add to downvotes if not already there
        if !votes.downvotes.contains(&user.to_string()) {
            votes.downvotes.push(user.to_string());
            info!("User {user} downvoted song {song_id}");
        }
        self.state.persist();
    }

    fn remove_upvote(&mut self, song_id: &str, user: &str) {
        if let Some(votes) = self.state.song_votes.get_mut(song_id) {
            votes.upvotes.retain(|u| u != user);
            info!("User {user} removed upvote from song {song_id}");
            self.state.persist();
        }
    }

    fn remove_downvote(&mut self, song_id: &str, user: &str) {
        if let Some(votes) = self.state.song_votes.get_mut(song_id) {
            votes.downvotes.retain(|u| u != user);
            info!("User {user} removed downvote from song {song_id}");
            self.state.persist();
        }
    }

    /// Clean up votes for songs that are no longer in the queue
    fn cleanup_votes(&mut self) {
        let song_ids: std::collections::HashSet<_> =
            self.state.queued_songs.iter().map(|s| &s.id).collect();
        self.state.song_votes.retain(|id, _| song_ids.contains(id));
    }

    fn enqueue(&mut self, song: Song) {
        if self.state.queued_songs.contains(&song) {
            info!("Rejecting duplicate song: {} ({})", song.title, song.id);
            self.say("Song already in queue!");
        } else {
            let queue_was_empty = self.state.queued_songs.is_empty();
            let time_until_playback = self.queue_duration_mins();
            self.state.queued_songs.push(song.clone());

            info!(
                "Enqueued song: {} ({}) by {}, queue size: {}, time until playback: {} min",
                song.title,
                song.id,
                song.queued_by,
                self.queue_len(),
                time_until_playback
            );

            let msg = format!(
                "Added {} ({}) to the queue (queued by {}). Time until playback: {} min",
                song.title, song.url, song.queued_by, time_until_playback
            );
            self.say_rich(
                &msg,
                RichContent::SongEnqueued {
                    song: song.clone(),
                    time_until_playback_mins: time_until_playback,
                },
            );

            if !self.state.is_playing && self.state.should_play && queue_was_empty {
                self.play_song(song)
            }

            self.state.persist()
        }
    }

    fn list_queue(&self, offset: Option<usize>, is_now_playing_update: bool) {
        let fmt_song = |song: Option<&Song>| {
            song.map(|song| {
                format!(
                    "{} ({}, queued by {})",
                    song.title, song.url, song.queued_by
                )
            })
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

        let now_playing = np.cloned().map(|song| NowPlayingInfo {
            song,
            progress_secs: self.state.playback_progress,
        });

        self.say_rich(
            &msg,
            RichContent::QueueStatus {
                now_playing,
                queue_length: len,
                queue_duration_mins: duration_min,
                is_playing: self.state.is_playing,
                is_now_playing_update,
            },
        );
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
            Some(song) => self.say_rich(
                &format!("Removed song {} from the queue", song.title),
                RichContent::SongRemoved { title: song.title },
            ),
            None => self.say(&format!("No song at position {pos} in the queue")),
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
            Some(song) => self.say_rich(
                &format!("Removed song {} from the queue", song.title),
                RichContent::SongRemoved { title: song.title },
            ),
            None => self.say(&format!("No songs queued by {nick}")),
        }
    }

    fn play_song(&mut self, song: Song) {
        info!(
            "Playing song: {} ({}) - duration: {}:{:02}",
            song.title,
            song.id,
            song.duration / 60,
            song.duration % 60
        );

        self.state.is_playing = true;
        self.state.song_loaded = true;
        self.state.playback_progress = 0;

        self.bus.send(Event::Symphonia(SymphoniaAction::PlayYtUrl {
            url: song.url,
        }));

        self.list_queue(None, true);
        self.state.persist();
    }

    fn end_of_queue(&mut self) {
        info!(
            "Playback queue ended, {} songs played",
            self.state.played_songs.len()
        );

        self.state.is_playing = false;

        self.bus.send(Event::Symphonia(SymphoniaAction::Stop));

        self.say("Playback queue ended.");
        self.state.persist()
    }

    fn next(&mut self, remove_current: bool) {
        if !self.state.queued_songs.is_empty() {
            // Move now playing song to played_songs
            let song = self.state.queued_songs.remove(0);

            if !remove_current {
                info!("Finished song: {} ({})", song.title, song.id);
                self.state.played_songs.push(song.clone());
            } else {
                info!("Skipped song: {} ({})", song.title, song.id);
            }

            // Clean up votes for the removed song
            self.state.song_votes.remove(&song.id);
        }

        self.cleanup_votes();

        if self.state.queued_songs.is_empty() {
            self.end_of_queue();
        } else {
            // Sort remaining songs by votes, then take the first one
            let mut songs_by_votes: Vec<_> = self
                .state
                .queued_songs
                .iter()
                .enumerate()
                .map(|(i, song)| (song.clone(), i))
                .collect();

            songs_by_votes.sort_by(|(song_a, pos_a), (song_b, pos_b)| {
                let score_a = self.get_votes(&song_a.id).score();
                let score_b = self.get_votes(&song_b.id).score();
                (score_b, pos_a).cmp(&(score_a, pos_b))
            });

            if let Some((_, orig_pos)) = songs_by_votes.first() {
                // Move the selected song to the front of the queue if needed
                if *orig_pos > 0 {
                    let song = self.state.queued_songs.remove(*orig_pos);
                    self.state.queued_songs.insert(0, song.clone());
                    info!(
                        "Promoted song {} from position {} to play next (due to votes)",
                        song.title, orig_pos
                    );
                }
                let song = self.state.queued_songs.first().cloned();
                if let Some(song) = song {
                    self.play_song(song);
                }
            }
        }
        self.state.persist()
    }

    fn prev(&mut self) {
        let song = self.state.played_songs.pop();

        if let Some(song) = song {
            info!("Going back to previous song: {} ({})", song.title, song.id);
            self.state.queued_songs.insert(0, song.clone());
            self.play_song(song);
        } else {
            info!("No previous song available");
            self.end_of_queue()
        }
        self.state.persist()
    }
}

/// Type alias for shared playback state
pub type SharedPlayback = Arc<RwLock<Playback>>;

pub async fn init(bus: &EventBus) -> SharedPlayback {
    let playback = Arc::new(RwLock::new(Playback::create(bus.clone()).await));

    handle_incoming_event_loop(bus.clone(), playback.clone());

    playback
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
            playback.list_queue(offset, false);
        }
        PlaybackAction::RmSongByPos { pos } => playback.rm_song_at_pos(pos),
        PlaybackAction::RmSongByNick { nick } => playback.rm_latest_song_by_nick(nick),
        PlaybackAction::Play => {
            info!("Playback resumed");
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
            info!("Playback paused");
            playback.state.is_playing = false;
            playback.state.should_play = false;

            playback.bus.send(Event::Symphonia(SymphoniaAction::Pause));

            playback.state.persist();
        }
        PlaybackAction::EndOfSong => {
            info!("End of song signal received, advancing to next");
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
        PlaybackAction::Upvote { song_id, user } => {
            playback.upvote(&song_id, &user);
        }
        PlaybackAction::Downvote { song_id, user } => {
            playback.downvote(&song_id, &user);
        }
        PlaybackAction::RemoveUpvote { song_id, user } => {
            playback.remove_upvote(&song_id, &user);
        }
        PlaybackAction::RemoveDownvote { song_id, user } => {
            playback.remove_downvote(&song_id, &user);
        }
    }
}
