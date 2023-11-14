use crate::{
    config::Config,
    event::{Event, EventBus},
    irc::IrcAction,
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
const NUM_TEMPO_NICKS: usize = 4;
const NUM_BINGO_NICKS: usize = 4;
const ANTI_FLOOD_DELAY: Duration = Duration::from_millis(1200);
const SECOND: Duration = Duration::from_secs(1);
const TEMPO_DEADLINE_REDUCTION: Duration = Duration::from_secs(60);
const TEMPO_DEADLINE: Duration = Duration::from_secs(300);
const HELP_TEXT: &str = r#"
===================================================================
Useful commands:
Add a song you want to sing:              !request <url>
List current requests:                    !ls
And to say stuff, use:                    !speak <text>
Add a YouTube url to the music queue:     !p <url>
For help during the evening:              !help
And the most important - to sing a song:  !tempo
==================================================================="#;

#[derive(Clone, Debug)]
pub enum SongleaderAction {
    /// Requests a song to be sung
    RequestSong { url: String },

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
    first_songs: VecDeque<SongbookSong>,

    /// List of all song requests
    requests: Vec<SongbookSong>,

    /// List of backup songs in case the requests run out
    backup: Vec<SongbookSong>,

    /// Current mode of the songleader
    mode: Mode,
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

    fn persist(&self) {
        let json = serde_json::to_string_pretty(self);
        match json {
            Ok(json) => {
                tokio::spawn(async move {
                    let res = tokio::fs::write(SONGLEADER_STATE_FILE, json).await;

                    if let Err(e) = res {
                        error!("Error while writing songleader state: {:?}", e);
                    }
                });
            }
            Err(e) => {
                error!("Error while serializing songleader state: {:?}", e)
            }
        }
    }

    pub fn get_songs(&self) -> Vec<SongbookSong> {
        let mut songs = Vec::new();

        songs.extend(self.first_songs.clone());
        songs.extend(self.requests.clone());
        songs.extend(self.backup.clone());

        songs
    }

    fn add_request(&mut self, song: SongbookSong) -> Result<SongbookSong> {
        let songs = self.get_songs();

        if songs.contains(&song) {
            return Err(anyhow!("Song already requested"));
        }

        self.requests.push(song.clone());
        self.persist();

        Ok(song)
    }

    pub fn pop_next_song(&mut self) -> Option<SongbookSong> {
        if let Some(song) = self.first_songs.pop_front() {
            return Some(song);
        }

        if !self.requests.is_empty() {
            let index = rand::thread_rng().gen_range(0..self.requests.len());
            return Some(self.requests.remove(index));
        }

        if !self.backup.is_empty() {
            let index = rand::thread_rng().gen_range(0..self.backup.len());
            return Some(self.backup.remove(index));
        }

        None
    }
}

pub struct Songleader {
    /// Current state of the songleader
    state: SongleaderState,

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

    /// Changes the [Mode] of the [SongleaderState] and writes new state to
    /// disk.
    fn set_mode(&mut self, mode: Mode) {
        debug!("Transitioning to mode: {:?}", mode);

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

    /// Convenience method for sending irc messages
    fn irc_say(&self, msg: &str) {
        self.bus
            .send(Event::Irc(IrcAction::SendMsg(msg.to_string())));
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

    /// Convenience method for sending the same message to tts and irc
    fn tts_and_irc_say(&self, text: &str) {
        self.tts_say(text);
        self.irc_say(text);
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

        let mk_songbook_song = |title: &str, id: &str, page: usize| SongbookSong {
            url: format!("{}/{id}", self.config.songbook.songbook_url),
            id: id.to_string(),
            title: Some(title.to_string()),
            book: Some(format!("TF:s Sångbok 150 – s. {page}")),
        };

        self.state.first_songs = vec![
            mk_songbook_song("Halvankaren", "tf-sangbok-150-halvankaren", 39),
            mk_songbook_song(
                "Fjärran han dröjer",
                "tf-sangbok-150-fjarran-han-drojer",
                45,
            ),
        ]
        .into();

        self.state.requests = vec![];
        self.state.backup = vec![
            mk_songbook_song("Rattataa", "tf-sangbok-150-rattataa", 0),
            mk_songbook_song("Nu är det nu", "tf-sangbok-150-nu-ar-det-nu", 125),
            mk_songbook_song("Mera brännvin", "tf-sangbok-150-mera-brannvin", 83),
            mk_songbook_song("Tycker du som jag", "tf-sangbok-150-tycker-du-som-jag", 79),
            mk_songbook_song("Siffervisan", "tf-sangbok-150-siffervisan", 115),
            mk_songbook_song("Vad i allsin dar?", "tf-sangbok-150-vad-i-allsin-dar", 54),
            mk_songbook_song("Undulaten", "tf-sangbok-150-undulaten", 72),
        ];

        self.tts_say("Diii duuuu diii duuuu diii duuu");
        sleep(3 * SECOND).await;

        let welcome_text = format!(
            r#"===================================================================
Hi and welcome to this party. I will be your host today.
{HELP_TEXT}
Have fun, and don't drown in the shower!
==================================================================="#
        );

        for line in welcome_text.split('\n') {
            self.irc_say(line);
            sleep(ANTI_FLOOD_DELAY).await;
        }

        sleep(3 * SECOND).await;

        self.irc_say("*sjunger:*");

        self.tts_and_irc_say("En liten fågel satt en gång, och sjöng i furuskog.");
        sleep(4 * SECOND).await;
        self.tts_and_irc_say("Han hade sjungit dagen lång, men dock ej sjungit nog.");
        sleep(4 * SECOND).await;
        self.tts_and_irc_say("Vad sjöng den lilla fågeln då? JO!");
        sleep(3 * SECOND).await;

        self.irc_say("Helan går...");
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
        let song = self.state.pop_next_song();

        match song {
            Some(song) => {
                self.set_mode(Mode::Bingo {
                    nicks: HashSet::new(),
                    song: song.clone(),
                });

                self.allow_music_playback(false);

                self.tts_say(&format!("Nästa sång kommer nu... {song}"));
                self.irc_say(&format!("Next song coming up: {song}. {}", song.url));
                self.irc_say("Type !bingo when you have found it!")
            }
            None => {
                self.irc_say("No songs found :(, add more songs: !request <url>");
                self.enter_tempo_mode();
            }
        }
    }

    /// Enters the [Mode::Singing] mode
    pub async fn enter_singing_mode(&mut self) {
        self.set_mode(Mode::Singing);

        self.allow_low_prio_speech(false);

        self.tts_say("PLING PLONG");
        self.irc_say("Song starts in 3");
        sleep(SECOND).await;
        self.irc_say("2");
        sleep(SECOND).await;
        self.irc_say("1");
        sleep(SECOND).await;
        self.irc_say("NOW!");
    }

    /// Ends the party
    pub fn end(&mut self) {
        if self.state.mode == Mode::Inactive {
            warn!("Cannot call end() when already in Inactive mode");
            return;
        }

        self.irc_say("Party is over. go drunk, you are home....");
        self.enter_inactive_mode();
    }
}

pub async fn init(bus: &EventBus, config: &Config) {
    let songleader = Arc::new(RwLock::new(Songleader::create(bus, config).await));

    handle_incoming_event_loop(bus.clone(), config.clone(), songleader.clone());
    check_tempo_timeout_loop(songleader.clone());
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
async fn handle_incoming_event(
    bus: EventBus,
    config: Config,
    songleader_rwlock: Arc<RwLock<Songleader>>,
    action: SongleaderAction,
) {
    let mut songleader = songleader_rwlock.write().await;

    match action {
        SongleaderAction::RequestSong { url } => {
            // Don't hold onto the lock while fetching song info
            drop(songleader);

            let song = songbook::get_song_info(&url, &config).await;

            let mut songleader = songleader_rwlock.write().await;
            let result = song.and_then(|song| songleader.state.add_request(song));

            match result {
                Ok(song) => songleader.irc_say(&format!("Added {song} to requests")),
                Err(e) => songleader.irc_say(&format!("Error while requesting song: {:?}", e)),
            }
        }

        SongleaderAction::Tempo { nick } => {
            if let Mode::Tempo { nicks, .. } = &mut songleader.state.mode {
                nicks.insert(nick);

                if nicks.len() > NUM_TEMPO_NICKS {
                    songleader.enter_bingo_mode();
                }
            }
        }

        SongleaderAction::Bingo { nick } => {
            if let Mode::Bingo { nicks, .. } = &mut songleader.state.mode {
                nicks.insert(nick);

                if nicks.len() > NUM_BINGO_NICKS {
                    songleader.enter_singing_mode().await;
                }
            }
        }

        SongleaderAction::Skål => {
            if let Mode::Singing = &mut songleader.state.mode {
                songleader.enter_tempo_mode();
            }
        }
        SongleaderAction::ListSongs => {
            let songs = songleader.state.get_songs();
            let msg = if songs.is_empty() {
                "No requested songs found :(".to_string()
            } else {
                let songs_str: Vec<String> = songs.iter().map(|song| song.to_string()).collect();
                format!("Song requests: {}", songs_str.join(", "))
            };
            songleader.irc_say(&msg);
        }
        SongleaderAction::ForceTempo => songleader.enter_tempo_mode(),
        SongleaderAction::ForceBingo => songleader.enter_bingo_mode(),
        SongleaderAction::ForceSinging => songleader.enter_singing_mode().await,
        SongleaderAction::Pause => songleader.enter_inactive_mode(),
        SongleaderAction::End => songleader.end(),
        SongleaderAction::Begin => songleader.begin().await,
        SongleaderAction::Help => {
            // Disallow help text outside of these modes
            if !matches!(songleader.state.mode, Mode::Tempo { .. } | Mode::Inactive) {
                return;
            }

            // Avoid blocking current task by spawning a new one to
            // flood the help text
            let bus = bus.clone();
            tokio::spawn(async move {
                for line in HELP_TEXT.split('\n') {
                    bus.send(Event::Irc(IrcAction::SendMsg(line.to_string())));
                    sleep(ANTI_FLOOD_DELAY).await;
                }
            });
        }
    }
}
