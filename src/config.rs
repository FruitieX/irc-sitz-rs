use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;

#[cfg(feature = "irc")]
#[derive(Clone, Deserialize, Serialize)]
pub struct IrcConfig {
    /// IRC nickname for the bot
    pub irc_nickname: String,

    /// IRC server hostname
    pub irc_server: String,

    /// IRC channel to join
    pub irc_channel: String,

    /// Whether to use TLS for the IRC connection
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
