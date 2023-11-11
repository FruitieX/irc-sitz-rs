use crate::{
    bus::{Event, EventBus},
    irc::IrcAction,
    playback::PlaybackAction,
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
Add a song you want to sing:              !request <songname, page>
List current requests:                    !ls
And to say stuff, use:                    !speak <text>
Add a YouTube url to the music queue:     !p <url>
For help during the evening:              !help
And the most important - to sing a song:  !tempo
==================================================================="#;

#[derive(Clone, Debug)]
pub enum SongleaderAction {
    /// Requests a song to be sung
    RequestSong { song: Song },

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

pub type Song = String;

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
        song: Song,
    },

    /// Songleader is waiting for song to end by anybody typing "!skål".
    Singing,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct SongleaderState {
    /// List of songs that the songleader will sing first
    first_songs: VecDeque<Song>,

    /// List of all song requests
    requests: Vec<Song>,

    /// List of backup songs in case the requests run out
    backup: Vec<Song>,

    /// Current mode of the songleader
    mode: Mode,
}

impl SongleaderState {
    async fn read_or_default() -> Self {
        let res = tokio::fs::read(SONGLEADER_STATE_FILE).await;

        match res {
            Ok(res) => serde_json::from_slice(&res).unwrap_or_default(),
            Err(e) => {
                eprintln!("Error while reading songleader state: {:?}", e);
                eprintln!("Falling back to default state.");
                SongleaderState::default()
            }
        }
    }

    fn write_state(&self) {
        let json = serde_json::to_vec(self);
        match json {
            Ok(json) => {
                tokio::spawn(async move {
                    let res = tokio::fs::write(SONGLEADER_STATE_FILE, json).await;

                    if let Err(e) = res {
                        eprintln!("Error while writing songleader state: {:?}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Error while serializing songleader state: {:?}", e)
            }
        }
    }

    pub fn get_songs(&self) -> Vec<Song> {
        let mut songs = Vec::new();

        songs.extend(self.first_songs.clone());
        songs.extend(self.requests.clone());
        songs.extend(self.backup.clone());

        songs
    }

    fn add_request(&mut self, song: &Song) -> Result<()> {
        let songs = self.get_songs();

        if songs.contains(song) {
            return Err(anyhow!("Song already requested"));
        }

        self.requests.push(song.clone());
        self.write_state();

        Ok(())
    }

    pub fn pop_next_song(&mut self) -> Option<Song> {
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
}

impl Songleader {
    /// Creates a new [Songleader] struct
    pub async fn create(bus: &EventBus) -> Self {
        let state = SongleaderState::read_or_default().await;

        println!("Initial state:\n{:#?}", state);

        Self {
            state,
            bus: bus.clone(),
        }
    }

    /// Changes the [Mode] of the [SongleaderState] and writes new state to
    /// disk.
    fn set_mode(&mut self, mode: Mode) {
        println!("Transitioning to mode: {:?}", mode);

        self.state.mode = mode;
        self.state.write_state();
    }

    /// Convenience method for sending text to speech messages
    fn tts_say(&self, text: &str) {
        self.bus
            .send(Event::TextToSpeech(TextToSpeechAction::Speak {
                text: text.to_string(),
                prio: Priority::High,
            }))
            .unwrap();
    }

    /// Convenience method for sending irc messages
    fn irc_say(&self, msg: &str) {
        self.bus
            .send(Event::Irc(IrcAction::SendMsg(msg.to_string())))
            .unwrap();
    }

    /// Convenience method for (dis)allowing music playback
    fn allow_music_playback(&self, allow: bool) {
        if allow {
            self.bus
                .send(Event::Playback(PlaybackAction::Play))
                .unwrap();
        } else {
            self.bus
                .send(Event::Playback(PlaybackAction::Pause))
                .unwrap();
        }
    }

    /// Convenience method for (dis)allowing low priority speech messages
    fn allow_low_prio_speech(&self, allow: bool) {
        if allow {
            self.bus
                .send(Event::TextToSpeech(TextToSpeechAction::AllowLowPrio))
                .unwrap();
        } else {
            self.bus
                .send(Event::TextToSpeech(TextToSpeechAction::DisallowLowPrio))
                .unwrap();
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
            println!("Cannot call begin() when not in Inactive mode");
            return;
        }

        // NOTE: Intentionally avoid storing Mode::Starting in the state file
        // since that would block the songleader from being able to start again
        // if the program is restarted while in this mode.
        self.state.mode = Mode::Starting;

        self.allow_music_playback(false);
        self.allow_low_prio_speech(false);

        self.state.first_songs = vec![
            "Halvankaren - s, 32".to_string(),
            "Fjärran han dröjer - s, 35".to_string(),
        ]
        .into();

        self.state.requests = vec![];
        self.state.backup = vec![
            "Rattataa - s, 43".to_string(),
            "Nu är det nu - s, 91".to_string(),
            "Mera brännvin(Internationalen) - s. 62".to_string(),
            "Tycker du som jag - s, 63".to_string(),
            "Siffervisan - s, 114".to_string(),
            "Vad i allsindar - s, 115".to_string(),
            "Undulaten - s, 45".to_string(),
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
                self.irc_say(&format!(
                    "Next song coming up: {song}. Type !bingo when you have found it!"
                ));
            }
            None => {
                self.irc_say("No songs found :(, add more songs: !request songname, page number");
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
            println!("Cannot call end() when already in Inactive mode");
            return;
        }

        self.irc_say("Party is over. go drunk, you are home....");
        self.enter_inactive_mode();
    }
}

pub async fn start(bus: &EventBus) {
    let songleader = Arc::new(RwLock::new(Songleader::create(bus).await));

    handle_incoming_event_loop(bus.clone(), songleader.clone());
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
fn handle_incoming_event_loop(bus: EventBus, songleader: Arc<RwLock<Songleader>>) {
    tokio::spawn(async move {
        let mut bus_rx = bus.subscribe();

        loop {
            let event = bus_rx.recv().await.unwrap();

            let mut songleader = songleader.write().await;

            if let Event::Songleader(action) = event {
                match action {
                    SongleaderAction::RequestSong { song } => {
                        let result = songleader.state.add_request(&song);
                        match result {
                            Ok(_) => songleader.irc_say(&format!("Added {song} to requests")),
                            Err(e) => songleader.irc_say(&format!("Error: {:?}", e)),
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
                        let msg = songs.join(", ");
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
                            continue;
                        }

                        // Avoid blocking current task by spawning a new one to
                        // flood the help text
                        let bus = bus.clone();
                        tokio::spawn(async move {
                            for line in HELP_TEXT.split('\n') {
                                bus.send(Event::Irc(IrcAction::SendMsg(line.to_string())))
                                    .unwrap();
                                sleep(ANTI_FLOOD_DELAY).await;
                            }
                        });
                    }
                }
            }
        }
    });
}
