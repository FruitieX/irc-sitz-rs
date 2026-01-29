use crate::{
    config::Config,
    event::{Event, EventBus},
    message::{CountdownValue, MessageAction, RichContent},
    playback::PlaybackAction,
    songbook::{self, SongbookSong},
    sources::espeak::{Priority, TextToSpeechAction},
};
use anyhow::{anyhow, Result};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Duration,
};
use tokio::{
    sync::RwLock,
    time::{sleep, Instant},
};

const SONGLEADER_STATE_FILE: &str = "songleader_state.json";
const SONGLEADER_STATE_FILE_TMP: &str = "songleader_state.json.tmp";
const NUM_TEMPO_NICKS: usize = 3;
const NUM_BINGO_NICKS: usize = 3;
const ANTI_FLOOD_DELAY: Duration = Duration::from_millis(1200);
const SECOND: Duration = Duration::from_secs(1);
const TEMPO_DEADLINE_REDUCTION: Duration = Duration::from_secs(60);
const TEMPO_DEADLINE: Duration = Duration::from_secs(420);
const HELP_TEXT: &str = r#"
===================================================================
Useful commands:
Add a YouTube URL to the music queue:     !p https://youtu.be/dQw4w9WgXcQ
Remove most recently queued music by you: !rm
Request a song you want to sing:          !request songbook-url
List current requests:                    !ls
To say stuff, use:                        !speak hello world
For help during the evening:              !help
And the most important - to sing a song:  !tempo
==================================================================="#;

#[derive(Clone, Debug)]
pub enum SongleaderAction {
    /// Requests a song to be sung from an URL
    RequestSongUrl { url: String, queued_by: String },

    /// Requests a song to be sung by name
    RequestSong { song: SongbookSong },

    /// Removes a song by ID
    RmSongById { id: String },

    /// Removes latest song queued by nick
    RmSongByNick { nick: String },

    /// Advance to the next song faster
    Tempo { nick: String },

    /// Ready to sing upcoming song
    Bingo { nick: String },

    /// Song is finished
    Skål,

    /// Responds with list of song requests
    ListSongs,

    /// Forces tempo
    ForceTempo,

    /// Forces bingo
    ForceBingo,

    /// Forces singing
    ForceSinging,

    /// Pauses the songleader
    Pause,

    /// Forces end of party
    End,

    /// Start party
    Begin,

    /// Print help text
    Help,
}

#[derive(Default, Debug, Deserialize, Serialize, PartialEq)]
pub enum Mode {
    /// Songleader is inactive. Effectively pauses the songleader.
    #[default]
    Inactive,

    /// Songleader is playing its starting routine
    Starting,

    /// Songleader is waiting to sing next song.  Waits until [NUM_TEMPO_NICKS]
    /// have typed "!tempo" or until [TEMPO_DEADLINE] has passed. Each "!tempo"
    /// reduces the deadline by [TEMPO_DEADLINE_REDUCTION].
    Tempo {
        /// Set of nicknames that have typed "!tempo"
        nicks: HashSet<String>,

        /// Time when [Mode::Tempo] was entered
        #[serde(skip, default = "Instant::now")]
        init_t: Instant,
    },

    /// Songleader is waiting for everyone to be ready to sing next song.  Waits
    /// until [NUM_BINGO_NICKS] have types "!bingo".
    Bingo {
        /// Set of nicknames that have typed "!bingo"
        nicks: HashSet<String>,

        /// Song that is about to be sung
        song: SongbookSong,
    },

    /// Songleader is waiting for song to end by anybody typing "!skål".
    Singing,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct SongleaderState {
    /// List of songs that the songleader will sing first
    pub first_songs: VecDeque<SongbookSong>,

    /// List of all song requests
    pub requests: Vec<SongbookSong>,

    /// List of backup songs in case the requests run out
    pub backup: Vec<SongbookSong>,

    /// Current mode of the songleader
    pub mode: Mode,
}

impl SongleaderState {
    async fn read_or_default() -> Self {
        let res = tokio::fs::read(SONGLEADER_STATE_FILE).await;

        match res {
            Ok(res) => serde_json::from_slice(&res).unwrap_or_default(),
            Err(e) => {
                info!("Error while reading songleader state: {:?}", e);
                info!("Falling back to default state.");
                SongleaderState::default()
            }
        }
    }

    /// Persists state to disk using atomic write (write to temp file, then rename).
    /// This prevents corruption if the process crashes during write.
    fn persist(&self) {
        let json = match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(e) => {
                error!("Error while serializing songleader state: {:?}", e);
                return;
            }
        };

        // Spawn a task to perform the atomic write
        tokio::spawn(async move {
            // Write to temp file first
            if let Err(e) = tokio::fs::write(SONGLEADER_STATE_FILE_TMP, &json).await {
                error!("Error while writing songleader state to temp file: {:?}", e);
                return;
            }

            // Atomically rename temp file to actual file
            // This ensures we never have a partially written state file
            if let Err(e) =
                tokio::fs::rename(SONGLEADER_STATE_FILE_TMP, SONGLEADER_STATE_FILE).await
            {
                error!("Error while renaming songleader state file: {:?}", e);
            }
        });
    }

    pub fn get_songs(&self) -> Vec<SongbookSong> {
        let mut songs = Vec::new();

        songs.extend(self.first_songs.clone());
        songs.extend(self.requests.clone());
        songs.extend(self.backup.clone());

        songs
    }

    pub fn add_request(&mut self, song: SongbookSong) -> Result<SongbookSong> {
        let songs = self.get_songs();

        if songs.contains(&song) {
            if let Some(index) = self.backup.iter().position(|s| s == &song) {
                // If song already in backup queue, remove it and add it to requests
                self.backup.remove(index);
            } else {
                return Err(anyhow!("Song already requested"));
            }
        }

        self.requests.push(song.clone());
        self.persist();

        Ok(song)
    }

    pub fn rm_song_by_id(&mut self, id: String) -> Result<SongbookSong> {
        let index = self
            .requests
            .iter()
            .position(|song| song.id == id)
            .ok_or_else(|| anyhow!("Song not found by id {id}"))?;

        let song = self.requests.remove(index);
        self.persist();

        Ok(song)
    }

    pub fn rm_song_by_nick(&mut self, nick: String) -> Result<SongbookSong> {
        let index = self
            .requests
            .iter()
            .rposition(|song| song.queued_by == Some(nick.clone()))
            .ok_or_else(|| anyhow!("No song requests found by {nick}"))?;

        let song = self.requests.remove(index);
        self.persist();

        Ok(song)
    }

    pub fn pop_next_song(&mut self) -> Option<SongbookSong> {
        if let Some(song) = self.first_songs.pop_front() {
            return Some(song);
        }

        if !self.requests.is_empty() {
            let index = rand::rng().random_range(0..self.requests.len());
            return Some(self.requests.remove(index));
        }

        if !self.backup.is_empty() {
            let index = rand::rng().random_range(0..self.backup.len());
            return Some(self.backup.remove(index));
        }

        None
    }
}

pub struct Songleader {
    /// Current state of the songleader
    pub state: SongleaderState,

    /// Send and receive events to/from the rest of the app
    bus: EventBus,

    config: Config,
}

impl Songleader {
    /// Creates a new [Songleader] struct
    pub async fn create(bus: &EventBus, config: &Config) -> Self {
        let state = SongleaderState::read_or_default().await;

        debug!("Initial songleader state:\n{:#?}", state);

        Self {
            state,
            bus: bus.clone(),
            config: config.clone(),
        }
    }

    /// Creates a new [Songleader] with a specific initial state (for testing)
    pub fn create_with_state(bus: &EventBus, config: &Config, state: SongleaderState) -> Self {
        Self {
            state,
            bus: bus.clone(),
            config: config.clone(),
        }
    }

    /// Changes the [Mode] of the [SongleaderState] and writes new state to
    /// disk.
    fn set_mode(&mut self, mode: Mode) {
        info!("Mode transition: {:?} -> {:?}", self.state.mode, mode);

        self.state.mode = mode;
        self.state.persist();
    }

    /// Convenience method for sending text to speech messages
    fn tts_say(&self, text: &str) {
        self.bus
            .send(Event::TextToSpeech(TextToSpeechAction::Speak {
                text: text.to_string(),
                prio: Priority::High,
            }));
    }

    /// Convenience method for sending messages to all platforms
    fn say(&self, msg: &str) {
        self.bus.say(msg);
    }

    /// Convenience method for sending rich messages to all platforms
    fn say_rich(&self, text: &str, rich: RichContent) {
        self.bus.send_message(MessageAction::rich(text, rich));
    }

    /// Convenience method for (dis)allowing music playback
    fn allow_music_playback(&self, allow: bool) {
        if allow {
            self.bus.send(Event::Playback(PlaybackAction::Play));
        } else {
            self.bus.send(Event::Playback(PlaybackAction::Pause));
        }
    }

    /// Convenience method for (dis)allowing low priority speech messages
    fn allow_low_prio_speech(&self, allow: bool) {
        if allow {
            self.bus
                .send(Event::TextToSpeech(TextToSpeechAction::AllowLowPrio));
        } else {
            self.bus
                .send(Event::TextToSpeech(TextToSpeechAction::DisallowLowPrio));
        }
    }

    /// Convenience method for sending the same message to tts and all chat platforms
    fn tts_and_say(&self, text: &str) {
        self.tts_say(text);
        self.say(text);
    }

    /// Begins the party, must be called from [Mode::Inactive] and sets
    /// [Mode::Starting] while the starting routine is running. After that,
    /// automatically enters [Mode::Singing].
    pub async fn begin(&mut self) {
        if self.state.mode != Mode::Inactive {
            warn!("Cannot call begin() when not in Inactive mode");
            return;
        }

        // NOTE: Intentionally avoid storing Mode::Starting in the state file
        // since that would block the songleader from being able to start again
        // if the program is restarted while in this mode.
        self.state.mode = Mode::Starting;

        self.allow_music_playback(false);
        self.allow_low_prio_speech(false);

        let mk_songbook_song = |title: &str, id: &str, page: usize| {
            let id = format!("tf-sangbok-150-{}", id);
            let songbook_url = &self.config.songbook.songbook_url;

            SongbookSong {
                url: Some(format!("{songbook_url}/{id}")),
                id,
                title: Some(title.to_string()),
                book: Some(format!("TF:s Sångbok 150 – s. {page}")),
                queued_by: None,
            }
        };

        self.state.first_songs = vec![
            mk_songbook_song("Halvankaren", "halvankaren", 39),
            mk_songbook_song("Fjärran han dröjer", "fjarran-han-drojer", 45),
        ]
        .into();

        self.state.requests = vec![];

        self.state.backup = vec![
            mk_songbook_song("Rattataa", "rattataa", 0),
            mk_songbook_song("Nu är det nu", "nu-ar-det-nu", 125),
            mk_songbook_song("Mera brännvin", "mera-brannvin", 83),
            mk_songbook_song("Tycker du som jag", "tycker-du-som-jag", 79),
            mk_songbook_song("Siffervisan", "siffervisan", 115),
            mk_songbook_song("Vad i allsin dar?", "vad-i-allsin-dar", 54),
            mk_songbook_song("Undulaten", "undulaten", 72),
        ];

        self.tts_say("Diii duuuu diii duuuu diii duuu");
        sleep(3 * SECOND).await;

        let welcome_text = format!(
            r#"{} {}
===================================================================
Hi and welcome to this party. I will be your host today.
{}
Have fun, and don't drown in the shower!
==================================================================="#,
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            HELP_TEXT.replace(
                "songbook-url",
                &format!(
                    "{}/tf-sangbok-150-teknologvisan",
                    self.config.songbook.songbook_url
                )
            )
        );

        for line in welcome_text.split('\n') {
            self.say(line);
            sleep(ANTI_FLOOD_DELAY).await;
        }

        sleep(3 * SECOND).await;

        self.say("*sjunger:*");

        self.tts_and_say("En liten fågel satt en gång, och sjöng i furuskog.");
        sleep(4 * SECOND).await;
        self.tts_and_say("Han hade sjungit dagen lång, men dock ej sjungit nog.");
        sleep(4 * SECOND).await;
        self.tts_and_say("Vad sjöng den lilla fågeln då? JO!");
        sleep(3 * SECOND).await;

        self.say("Helan går...");
        self.tts_say("Helan går");

        // NOTE: Call set_mode() directly instead of enter_singing_mode() to
        // avoid having the latter generate irc and tts messages.
        self.set_mode(Mode::Singing);
    }

    /// Enters the [Mode::Inactive] mode
    pub fn enter_inactive_mode(&mut self) {
        self.set_mode(Mode::Inactive);

        self.allow_music_playback(true);
        self.allow_low_prio_speech(true);
    }

    /// Enters the [Mode::Tempo] mode
    pub fn enter_tempo_mode(&mut self) {
        self.set_mode(Mode::Tempo {
            init_t: Instant::now(),
            nicks: HashSet::new(),
        });

        self.allow_music_playback(true);
        self.allow_low_prio_speech(true);
    }

    /// Enters the [Mode::Bingo] mode
    pub fn enter_bingo_mode(&mut self) {
        info!(
            "Entering bingo mode - requests: {}, backup: {}",
            self.state.requests.len(),
            self.state.backup.len()
        );
        let song = self.state.pop_next_song();

        match song {
            Some(song) => {
                info!("Selected next song: {song}");
                self.set_mode(Mode::Bingo {
                    nicks: HashSet::new(),
                    song: song.clone(),
                });

                self.allow_music_playback(false);

                self.tts_say(&format!("Nästa sång kommer nu... {song}"));

                let text = if let Some(url) = &song.url {
                    format!("Next song coming up: {song}. {url}")
                } else {
                    format!("Next song coming up: {song}")
                };

                self.say_rich(
                    &format!("{text}\nType bingo when you have found it!"),
                    RichContent::BingoAnnouncement { song },
                );
            }
            None => {
                info!("No songs available in queue");
                self.say("No songs found :(, add more songs: !request <url>");
                self.enter_tempo_mode();
            }
        }
    }

    /// Enters the [Mode::Singing] mode
    pub async fn enter_singing_mode(&mut self) {
        self.set_mode(Mode::Singing);

        self.allow_low_prio_speech(false);

        self.tts_say("PLING PLONG");
        self.say_rich(
            "Song starts in 3",
            RichContent::Countdown {
                value: CountdownValue::Three,
            },
        );
        sleep(SECOND).await;
        self.say_rich(
            "2",
            RichContent::Countdown {
                value: CountdownValue::Two,
            },
        );
        sleep(SECOND).await;
        self.say_rich(
            "1",
            RichContent::Countdown {
                value: CountdownValue::One,
            },
        );
        sleep(SECOND).await;
        self.say_rich(
            "NOW!",
            RichContent::Countdown {
                value: CountdownValue::Now,
            },
        );
    }

    /// Ends the party
    pub fn end(&mut self) {
        if self.state.mode == Mode::Inactive {
            warn!("Cannot call end() when already in Inactive mode");
            return;
        }

        self.say("Party is over. go drunk, you are home....");
        self.enter_inactive_mode();
    }
}

/// Type alias for shared songleader state
pub type SharedSongleader = Arc<RwLock<Songleader>>;

pub async fn init(bus: &EventBus, config: &Config) -> SharedSongleader {
    let songleader = Arc::new(RwLock::new(Songleader::create(bus, config).await));

    handle_incoming_event_loop(bus.clone(), config.clone(), songleader.clone());
    check_tempo_timeout_loop(songleader.clone());

    songleader
}

/// Polls for tempo timeouts every second
fn check_tempo_timeout_loop(songleader: Arc<RwLock<Songleader>>) {
    tokio::spawn(async move {
        loop {
            sleep(SECOND).await;
            let mut songleader = songleader.write().await;

            if let Mode::Tempo { init_t, nicks } = &mut songleader.state.mode {
                let timeout =
                    *init_t + TEMPO_DEADLINE - TEMPO_DEADLINE_REDUCTION * nicks.len() as u32;

                if Instant::now() > timeout {
                    info!(
                        "Tempo timeout reached after {:?} with {} tempo(s), auto-transitioning to bingo mode",
                        init_t.elapsed(),
                        nicks.len()
                    );
                    songleader.enter_bingo_mode();
                }
            }
        }
    });
}

/// Loop over incoming events on the bus
fn handle_incoming_event_loop(bus: EventBus, config: Config, songleader: Arc<RwLock<Songleader>>) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await;

            if let Event::Songleader(action) = event {
                let songleader = songleader.clone();
                let bus = bus.clone();
                let config = config.clone();

                tokio::spawn(async move {
                    handle_incoming_event(bus, config, songleader, action).await;
                });
            }
        }
    });
}

/// Decide what to do based on the incoming event
pub async fn handle_incoming_event(
    _bus: EventBus,
    config: Config,
    songleader_rwlock: Arc<RwLock<Songleader>>,
    action: SongleaderAction,
) {
    let mut songleader = songleader_rwlock.write().await;

    match action {
        SongleaderAction::RequestSongUrl { url, queued_by } => {
            info!("Processing song request URL from {queued_by}: {url}");

            // Don't hold onto the lock while fetching song info
            drop(songleader);

            let song = songbook::get_song_info(&url, &config, &queued_by).await;

            let mut songleader = songleader_rwlock.write().await;
            let result = song.and_then(|song| songleader.state.add_request(song));

            match result {
                Ok(song) => {
                    info!("Song request added: {song} (queued by {queued_by})");
                    songleader.say_rich(
                        &format!("Added {song} to requests"),
                        RichContent::SongRequestAdded { song },
                    );
                }
                Err(e) => {
                    info!("Failed to add song request: {e:?}");
                    songleader.say(&format!("Error while requesting song: {e:?}"));
                }
            }
        }

        SongleaderAction::RequestSong { song } => {
            info!("Processing direct song request: {song}");
            let result = songleader.state.add_request(song);

            match result {
                Ok(song) => {
                    info!("Song request added: {song}");
                    songleader.say_rich(
                        &format!("Added {song} to requests"),
                        RichContent::SongRequestAdded { song },
                    );
                }
                Err(e) => {
                    info!("Failed to add song request: {e:?}");
                    songleader.say(&format!("Error while requesting song: {e:?}"));
                }
            }
        }

        SongleaderAction::RmSongById { id } => {
            info!("Processing song removal by id: {id}");
            let result = songleader.state.rm_song_by_id(id.clone());

            match result {
                Ok(song) => {
                    info!("Song removed: {song}");
                    let title = song.title.clone().unwrap_or_else(|| song.id.clone());
                    songleader.say_rich(
                        &format!("Removed {song} from requests"),
                        RichContent::SongRemoved { title },
                    );
                }
                Err(e) => {
                    info!("Failed to remove song by id {id}: {e:?}");
                    songleader.say(&format!("Error while removing song: {e:?}"));
                }
            }
        }

        SongleaderAction::RmSongByNick { nick } => {
            info!("Processing song removal by nick: {nick}");
            let result = songleader.state.rm_song_by_nick(nick.clone());

            match result {
                Ok(song) => {
                    info!("Song removed for {nick}: {song}");
                    let title = song.title.clone().unwrap_or_else(|| song.id.clone());
                    songleader.say_rich(
                        &format!("Removed {song} from requests"),
                        RichContent::SongRemoved { title },
                    );
                }
                Err(e) => {
                    info!("Failed to remove song by nick {nick}: {e:?}");
                    songleader.say(&format!("Error while removing song: {e:?}"));
                }
            }
        }

        SongleaderAction::Tempo { nick } => {
            if let Mode::Tempo { nicks, init_t } = &mut songleader.state.mode {
                let is_new = nicks.insert(nick.clone());
                let remaining = NUM_TEMPO_NICKS.saturating_sub(nicks.len());

                if is_new {
                    info!(
                        "Got tempo by {nick}, have {count}/{required} (waiting for {remaining} more)",
                        count = nicks.len(),
                        required = NUM_TEMPO_NICKS
                    );
                } else {
                    info!("Duplicate tempo by {nick}, ignoring");
                }

                let timeout_at =
                    *init_t + TEMPO_DEADLINE - TEMPO_DEADLINE_REDUCTION * nicks.len() as u32;
                let time_remaining = timeout_at.saturating_duration_since(Instant::now());
                info!(
                    "Tempo timeout in {time_remaining:.0?} (reduced by {reduction:.0?} due to {count} tempo(s))",
                    reduction = TEMPO_DEADLINE_REDUCTION * nicks.len() as u32,
                    count = nicks.len()
                );

                if nicks.len() >= NUM_TEMPO_NICKS {
                    info!("Tempo threshold reached, transitioning to bingo mode");
                    songleader.enter_bingo_mode();
                } else {
                    songleader.state.persist();
                }
            } else {
                info!(
                    "Ignoring tempo by {nick} - not in Tempo mode (current: {:?})",
                    songleader.state.mode
                );
            }
        }

        SongleaderAction::Bingo { nick } => {
            if let Mode::Bingo { nicks, song } = &mut songleader.state.mode {
                let is_new = nicks.insert(nick.clone());
                let remaining = NUM_BINGO_NICKS.saturating_sub(nicks.len());

                if is_new {
                    info!(
                        "Got bingo by {nick}, have {count}/{required} (waiting for {remaining} more) for song: {song}",
                        count = nicks.len(),
                        required = NUM_BINGO_NICKS
                    );
                } else {
                    info!("Duplicate bingo by {nick}, ignoring");
                }

                if nicks.len() >= NUM_BINGO_NICKS {
                    info!("Bingo threshold reached, transitioning to singing mode");
                    songleader.enter_singing_mode().await;
                } else {
                    songleader.state.persist();
                }
            } else {
                info!(
                    "Ignoring bingo by {nick} - not in Bingo mode (current: {:?})",
                    songleader.state.mode
                );
            }
        }

        SongleaderAction::Skål => {
            if let Mode::Singing = &mut songleader.state.mode {
                info!("Received skål, song finished - transitioning to tempo mode");
                songleader.enter_tempo_mode();
            } else {
                info!(
                    "Ignoring skål - not in Singing mode (current: {:?})",
                    songleader.state.mode
                );
            }
        }
        SongleaderAction::ListSongs => {
            let songs = songleader.state.get_songs();
            info!("Listing songs: {} total", songs.len());
            let msg = if songs.is_empty() {
                "No requested songs found :(".to_string()
            } else {
                let songs_str: Vec<String> = songs
                    .iter()
                    .map(|song| song.title.clone().unwrap_or_else(|| song.id.clone()))
                    .collect();
                format!("Song requests: {}", songs_str.join(", "))
            };
            songleader.say_rich(&msg, RichContent::SongRequestList { songs });
        }
        SongleaderAction::ForceTempo => {
            info!("Force tempo requested");
            songleader.enter_tempo_mode();
        }
        SongleaderAction::ForceBingo => {
            info!("Force bingo requested");
            songleader.enter_bingo_mode();
        }
        SongleaderAction::ForceSinging => {
            info!("Force singing requested");
            songleader.enter_singing_mode().await;
        }
        SongleaderAction::Pause => {
            info!("Pause requested");
            songleader.enter_inactive_mode();
        }
        SongleaderAction::End => {
            info!("End party requested");
            songleader.end();
        }
        SongleaderAction::Begin => {
            info!("Begin party requested");
            songleader.begin().await;
        }
        SongleaderAction::Help => {
            // Disallow help text outside of these modes
            if !matches!(songleader.state.mode, Mode::Tempo { .. } | Mode::Inactive) {
                return;
            }

            let songbook_url = config.songbook.songbook_url.clone();
            songleader.say_rich(
                &HELP_TEXT.replace(
                    "songbook-url",
                    &format!("{songbook_url}/tf-sangbok-150-teknologvisan"),
                ),
                RichContent::Help { songbook_url },
            );
        }
    }
}
