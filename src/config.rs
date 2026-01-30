use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::fs::read_to_string;
use tokio::sync::RwLock;

const PARAMS_STATE_FILE: &str = "params_state.json";
const PARAMS_STATE_FILE_TMP: &str = "params_state.json.tmp";

/// Runtime-adjustable parameters (can be modified via admin commands)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuntimeParams {
    /// Normal volume for music during playback (0.0 - 1.0)
    pub music_volume: f64,
    /// Ducked volume for music when TTS is playing (0.0 - 1.0)
    pub music_volume_ducked: f64,
    /// Volume multiplier for TTS (speech)
    pub tts_volume: f64,
    /// Number of votes required to skip a song
    pub skip_votes_required: usize,
    /// Number of tempo votes to advance to bingo
    pub num_tempo_nicks: usize,
    /// Number of bingo votes to start singing
    pub num_bingo_nicks: usize,
}

impl Default for RuntimeParams {
    fn default() -> Self {
        Self {
            music_volume: 0.75,
            music_volume_ducked: 0.2,
            tts_volume: 1.25,
            skip_votes_required: 3,
            num_tempo_nicks: 3,
            num_bingo_nicks: 3,
        }
    }
}

impl RuntimeParams {
    async fn read_or_default() -> Self {
        match tokio::fs::read(PARAMS_STATE_FILE).await {
            Ok(data) => serde_json::from_slice(&data).unwrap_or_default(),
            Err(e) => {
                info!("Error while reading params state: {:?}", e);
                info!("Falling back to default params.");
                RuntimeParams::default()
            }
        }
    }

    /// Persists params to disk using atomic write (write to temp file, then rename).
    pub fn persist(&self) {
        let json = match serde_json::to_string_pretty(self) {
            Ok(json) => json,
            Err(e) => {
                error!("Error while serializing params state: {:?}", e);
                return;
            }
        };

        tokio::spawn(async move {
            if let Err(e) = tokio::fs::write(PARAMS_STATE_FILE_TMP, &json).await {
                error!("Error while writing params state to temp file: {:?}", e);
                return;
            }

            if let Err(e) = tokio::fs::rename(PARAMS_STATE_FILE_TMP, PARAMS_STATE_FILE).await {
                error!("Error while renaming params state file: {:?}", e);
            }
        });
    }
}

/// Shared runtime parameters accessible from multiple components
pub type SharedRuntimeParams = Arc<RwLock<RuntimeParams>>;

/// Create a new shared runtime parameters instance, loading from file if available
pub async fn create_runtime_params() -> SharedRuntimeParams {
    Arc::new(RwLock::new(RuntimeParams::read_or_default().await))
}

#[cfg(feature = "irc")]
#[derive(Clone, Deserialize, Serialize)]
pub struct IrcConfig {
    /// IRC nickname for the bot
    pub irc_nickname: String,

    /// IRC server hostname
    pub irc_server: String,

    /// IRC server port (default: 6667 for plain, 6697 for TLS)
    pub irc_port: Option<u16>,

    /// IRC channel to join
    pub irc_channel: String,

    /// Whether to use TLS for the IRC connection (default: false)
    pub irc_use_tls: Option<bool>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SongbookConfig {
    pub songbook_url: String,

    #[serde(with = "serde_regex")]
    pub songbook_re: Regex,
}

#[cfg(feature = "discord")]
#[derive(Clone, Deserialize, Serialize)]
pub struct DiscordConfig {
    /// Discord bot token
    pub discord_token: String,

    /// Channel ID for the bot to operate in
    pub discord_channel_id: u64,

    /// Guild (server) ID for registering slash commands
    pub discord_guild_id: u64,

    /// Voice channel ID for streaming audio (optional, auto-joins if set)
    pub discord_voice_channel_id: Option<u64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    #[cfg(feature = "irc")]
    #[serde(flatten)]
    pub irc: Option<IrcConfig>,

    #[serde(flatten)]
    pub songbook: SongbookConfig,

    #[cfg(feature = "discord")]
    #[serde(flatten)]
    pub discord: Option<DiscordConfig>,
}

pub async fn load() -> Result<Config> {
    let config = read_to_string("Config.toml").await?;
    let config: Config = toml::from_str(&config)?;

    Ok(config)
}
