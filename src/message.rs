use crate::playback::Song;
use crate::songbook::SongbookSong;

/// Platform from which a message originates or to which it is targeted
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Platform {
    Irc,
    #[cfg(feature = "discord")]
    Discord,
    /// Bot-generated messages (should go to all platforms)
    Bot,
}

/// Rich message content for platforms that support it (e.g., Discord embeds)
/// These fields are only used when the `discord` feature is enabled.
#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub enum RichContent {
    /// Queue status with optional progress information
    QueueStatus {
        now_playing: Option<NowPlayingInfo>,
        next_up: Option<Song>,
        queue_length: usize,
        queue_duration_mins: u64,
        is_playing: bool,
    },

    /// Song added to queue confirmation
    SongEnqueued {
        song: Song,
        time_until_playback_mins: u64,
    },

    /// Bingo mode announcement - users should find the song
    BingoAnnouncement { song: SongbookSong },

    /// Song request list
    SongRequestList { songs: Vec<SongbookSong> },

    /// Help text
    Help { songbook_url: String },

    /// Error message
    Error { message: String },

    /// Song countdown ("3... 2... 1... NOW!")
    Countdown { value: CountdownValue },

    /// Song has been removed
    SongRemoved { title: String },

    /// Song request added
    SongRequestAdded { song: SongbookSong },
}

#[derive(Clone, Debug)]
#[cfg_attr(not(feature = "discord"), allow(dead_code))]
pub struct NowPlayingInfo {
    pub song: Song,
    pub progress_secs: u64,
}

#[derive(Clone, Debug)]
pub enum CountdownValue {
    Three,
    Two,
    One,
    Now,
}

/// Platform-agnostic message action
#[derive(Clone, Debug)]
pub enum MessageAction {
    /// Send a message to all platforms
    Send {
        /// Plain text fallback (used for IRC and TTS)
        text: String,
        /// Optional rich content for platforms that support it
        #[cfg_attr(not(feature = "discord"), allow(dead_code))]
        rich: Option<RichContent>,
        /// Source platform (for mirroring - don't echo back to source)
        source: Platform,
    },

    /// A user message to be mirrored to other platforms
    Mirror {
        /// Username/nickname of the sender
        username: String,
        /// Message content
        text: String,
        /// Source platform
        source: Platform,
    },

    /// Store message ID for reaction tracking (Discord bingo)
    #[cfg(feature = "discord")]
    StoreBingoMessageId { message_id: u64 },
}

impl MessageAction {
    /// Create a simple text message from the bot
    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    pub fn bot_say(text: impl Into<String>) -> Self {
        MessageAction::Send {
            text: text.into(),
            rich: None,
            source: Platform::Bot,
        }
    }

    /// Create a message with rich content
    pub fn rich(text: impl Into<String>, rich: RichContent) -> Self {
        MessageAction::Send {
            text: text.into(),
            rich: Some(rich),
            source: Platform::Bot,
        }
    }

    /// Create an error message
    #[cfg_attr(not(feature = "discord"), allow(dead_code))]
    pub fn error(message: impl Into<String>) -> Self {
        let msg = message.into();
        MessageAction::Send {
            text: msg.clone(),
            rich: Some(RichContent::Error { message: msg }),
            source: Platform::Bot,
        }
    }
}
