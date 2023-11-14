use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::fs::read_to_string;

#[derive(Clone, Deserialize, Serialize)]
pub struct IrcConfig {
    pub nickname: String,
    pub server: String,
    pub channel: String,
    pub use_tls: Option<bool>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct SongbookConfig {
    pub songbook_url: String,

    #[serde(with = "serde_regex")]
    pub songbook_re: Regex,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    #[serde(flatten)]
    pub irc: IrcConfig,

    #[serde(flatten)]
    pub songbook: SongbookConfig,
}

pub async fn load() -> Result<Config> {
    let config = read_to_string("Config.toml").await?;
    let config: Config = toml::from_str(&config)?;

    Ok(config)
}
