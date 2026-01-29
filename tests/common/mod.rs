//! Test infrastructure for irc-sitz-rs integration tests.
//!
//! Provides mocking utilities, test harnesses, and helper functions
//! for testing the songleader bot without external dependencies.

use regex::Regex;
use std::time::Duration;
use tokio::sync::broadcast::error::TryRecvError;

// Re-export key types from the main crate
#[cfg(feature = "irc")]
pub use irc_sitz_rs::config::IrcConfig;
pub use irc_sitz_rs::config::{Config, SongbookConfig};
pub use irc_sitz_rs::event::{Event, EventBus, Subscriber};
pub use irc_sitz_rs::message::{MessageAction, Platform, RichContent};
pub use irc_sitz_rs::playback::{PlaybackAction, Song};
pub use irc_sitz_rs::songbook::SongbookSong;
pub use irc_sitz_rs::songleader::{Mode, SongleaderAction, SongleaderState};
pub use irc_sitz_rs::sources::espeak::TextToSpeechAction;
pub use irc_sitz_rs::sources::symphonia::SymphoniaAction;

/// Creates a test configuration with localhost defaults.
pub fn test_config() -> Config {
    Config {
        #[cfg(feature = "irc")]
        irc: Some(IrcConfig {
            irc_nickname: "testbot".to_string(),
            irc_server: "localhost".to_string(),
            irc_channel: "#test".to_string(),
            irc_use_tls: None,
        }),
        songbook: SongbookConfig {
            songbook_url: "https://example-songbook.com".to_string(),
            songbook_re: Regex::new(r"(https?://)?example-songbook\.com/(.+)").unwrap(),
        },
        #[cfg(feature = "discord")]
        discord: None,
    }
}

/// Creates a mock Song for testing.
pub fn mock_song(id: &str, title: &str, queued_by: &str) -> Song {
    Song {
        id: id.to_string(),
        url: format!("https://youtu.be/{id}"),
        title: title.to_string(),
        channel: "Test Channel".to_string(),
        duration: 180, // 3 minutes
        queued_by: queued_by.to_string(),
    }
}

/// Creates a mock Song with custom duration for testing.
pub fn mock_song_with_duration(id: &str, title: &str, queued_by: &str, duration_secs: u64) -> Song {
    Song {
        id: id.to_string(),
        url: format!("https://youtu.be/{id}"),
        title: title.to_string(),
        channel: "Test Channel".to_string(),
        duration: duration_secs,
        queued_by: queued_by.to_string(),
    }
}

/// Creates a mock SongbookSong for testing.
pub fn mock_songbook_song(id: &str, title: &str, queued_by: Option<&str>) -> SongbookSong {
    SongbookSong {
        id: id.to_string(),
        url: Some(format!("https://example-songbook.com/{id}")),
        title: Some(title.to_string()),
        book: Some("Test Songbook".to_string()),
        queued_by: queued_by.map(|s| s.to_string()),
    }
}

/// Test harness that wraps EventBus and provides test utilities.
pub struct TestHarness {
    pub bus: EventBus,
    pub config: Config,
    subscribers: Vec<Subscriber>,
}

impl TestHarness {
    /// Creates a new test harness with default configuration.
    pub fn new() -> Self {
        Self {
            bus: EventBus::new(),
            config: test_config(),
            subscribers: Vec::new(),
        }
    }

    /// Creates a new test harness with custom configuration.
    pub fn with_config(config: Config) -> Self {
        Self {
            bus: EventBus::new(),
            config,
            subscribers: Vec::new(),
        }
    }

    /// Returns a reference to the EventBus.
    pub fn bus(&self) -> &EventBus {
        &self.bus
    }

    /// Returns a reference to the Config.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Creates a new subscriber for receiving events.
    pub fn subscribe(&mut self) -> Subscriber {
        self.bus.subscribe()
    }

    /// Sends an event to the bus.
    pub fn send(&self, event: Event) {
        self.bus.send(event);
    }

    /// Sends a songleader action.
    pub fn send_songleader(&self, action: SongleaderAction) {
        self.bus.send(Event::Songleader(action));
    }

    /// Sends a playback action.
    pub fn send_playback(&self, action: PlaybackAction) {
        self.bus.send(Event::Playback(action));
    }

    /// Simulates a tempo action from a nick.
    pub fn tempo(&self, nick: &str) {
        self.send_songleader(SongleaderAction::Tempo {
            nick: nick.to_string(),
        });
    }

    /// Simulates a bingo action from a nick.
    pub fn bingo(&self, nick: &str) {
        self.send_songleader(SongleaderAction::Bingo {
            nick: nick.to_string(),
        });
    }

    /// Simulates a skål action.
    pub fn skål(&self) {
        self.send_songleader(SongleaderAction::Skål);
    }

    /// Simulates beginning the party.
    pub fn begin(&self) {
        self.send_songleader(SongleaderAction::Begin);
    }

    /// Simulates ending the party.
    pub fn end(&self) {
        self.send_songleader(SongleaderAction::End);
    }

    /// Enqueues a song.
    pub fn enqueue(&self, song: Song) {
        self.send_playback(PlaybackAction::Enqueue { song });
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

/// Collects all events from a subscriber within a timeout period.
/// Returns events in the order they were received.
pub async fn collect_events(subscriber: &mut Subscriber, timeout: Duration) -> Vec<Event> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        match subscriber.try_recv() {
            Ok(event) => events.push(event),
            Err(TryRecvError::Empty) => {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(TryRecvError::Lagged(n)) => {
                eprintln!("Warning: subscriber lagged, missed {n} events");
            }
            Err(TryRecvError::Closed) => break,
        }
    }

    events
}

/// Collects events until a predicate is satisfied or timeout is reached.
pub async fn collect_events_until<F>(
    subscriber: &mut Subscriber,
    timeout: Duration,
    predicate: F,
) -> Vec<Event>
where
    F: Fn(&Event) -> bool,
{
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        match subscriber.try_recv() {
            Ok(event) => {
                let should_stop = predicate(&event);
                events.push(event);
                if should_stop {
                    break;
                }
            }
            Err(TryRecvError::Empty) => {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(TryRecvError::Lagged(n)) => {
                eprintln!("Warning: subscriber lagged, missed {n} events");
            }
            Err(TryRecvError::Closed) => break,
        }
    }

    events
}

/// Waits for a specific type of event within a timeout.
pub async fn wait_for_event<F>(
    subscriber: &mut Subscriber,
    timeout: Duration,
    matches: F,
) -> Option<Event>
where
    F: Fn(&Event) -> bool,
{
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        match subscriber.try_recv() {
            Ok(event) if matches(&event) => return Some(event),
            Ok(_) => continue,
            Err(TryRecvError::Empty) => {
                if tokio::time::Instant::now() >= deadline {
                    return None;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(TryRecvError::Lagged(_)) => continue,
            Err(TryRecvError::Closed) => return None,
        }
    }
}

/// Filters events by type.
pub fn filter_message_events(events: &[Event]) -> Vec<&MessageAction> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Message(action) => Some(action),
            _ => None,
        })
        .collect()
}

/// Filters playback events.
pub fn filter_playback_events(events: &[Event]) -> Vec<&PlaybackAction> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Playback(action) => Some(action),
            _ => None,
        })
        .collect()
}

/// Filters TTS events.
pub fn filter_tts_events(events: &[Event]) -> Vec<&TextToSpeechAction> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::TextToSpeech(action) => Some(action),
            _ => None,
        })
        .collect()
}

/// Filters symphonia (audio playback) events.
pub fn filter_symphonia_events(events: &[Event]) -> Vec<&SymphoniaAction> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Symphonia(action) => Some(action),
            _ => None,
        })
        .collect()
}

/// Filters songleader events.
pub fn filter_songleader_events(events: &[Event]) -> Vec<&SongleaderAction> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Songleader(action) => Some(action),
            _ => None,
        })
        .collect()
}

/// Checks if an event is a Message::Send with text containing a substring.
pub fn is_message_containing(event: &Event, substring: &str) -> bool {
    matches!(event, Event::Message(MessageAction::Send { text, .. }) if text.contains(substring))
}

/// Checks if any event in the list contains the given substring in its message.
pub fn has_message_containing(events: &[Event], substring: &str) -> bool {
    events.iter().any(|e| is_message_containing(e, substring))
}

/// Extracts the text from Message::Send events.
pub fn extract_message_texts(events: &[Event]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|e| match e {
            Event::Message(MessageAction::Send { text, .. }) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

/// Mock state file manager for testing state persistence.
pub struct MockStateFiles {
    pub dir: tempfile::TempDir,
}

impl MockStateFiles {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            dir: tempfile::TempDir::new()?,
        })
    }

    pub fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// Writes a songleader state file.
    pub async fn write_songleader_state(&self, state: &SongleaderState) -> std::io::Result<()> {
        let path = self.dir.path().join("songleader_state.json");
        let json = serde_json::to_string_pretty(state).unwrap();
        tokio::fs::write(path, json).await
    }

    /// Reads the songleader state file.
    pub async fn read_songleader_state(&self) -> std::io::Result<SongleaderState> {
        let path = self.dir.path().join("songleader_state.json");
        let json = tokio::fs::read_to_string(path).await?;
        Ok(serde_json::from_str(&json).unwrap())
    }
}

impl Default for MockStateFiles {
    fn default() -> Self {
        Self::new().expect("Failed to create temp directory")
    }
}

/// Asserts that a specific event type was received.
#[macro_export]
macro_rules! assert_event_received {
    ($events:expr, $pattern:pat) => {
        assert!(
            $events.iter().any(|e| matches!(e, $pattern)),
            "Expected event matching {} not found in {:?}",
            stringify!($pattern),
            $events
        );
    };
}

/// Asserts that a specific event type was NOT received.
#[macro_export]
macro_rules! assert_event_not_received {
    ($events:expr, $pattern:pat) => {
        assert!(
            !$events.iter().any(|e| matches!(e, $pattern)),
            "Unexpected event matching {} found in {:?}",
            stringify!($pattern),
            $events
        );
    };
}
