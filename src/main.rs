use irc_sitz_rs::{config, event, mixer, playback, songleader, sources, youtube};

#[cfg(feature = "discord")]
use std::sync::Arc;
#[cfg(feature = "discord")]
use std::sync::Mutex as StdMutex;

#[cfg(feature = "irc")]
use irc_sitz_rs::irc;

#[cfg(feature = "discord")]
use irc_sitz_rs::discord;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install rustls crypto provider before any TLS operations
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    pretty_env_logger::init_timed();

    let config = config::load().await?;
    let bus = event::EventBus::new();

    // Initialize audio sources - they write to shared buffers
    let tts_buffer = sources::espeak::init(&bus);
    let music_buffer = sources::symphonia::init(&bus).await?;

    youtube::init().await?;
    playback::init(&bus).await;

    #[cfg(feature = "irc")]
    if let Some(ref irc_config) = config.irc {
        irc::init(&bus, &config, irc_config).await?;
    }

    songleader::init(&bus, &config).await;
    event::debug(&bus);

    #[cfg(feature = "discord")]
    if let Some(ref discord_config) = config.discord {
        // Create the mixer with both audio sources
        let mixer = Arc::new(StdMutex::new(mixer::Mixer::new(
            tts_buffer.clone(),
            music_buffer.clone(),
        )));

        discord::init(&bus, &config, discord_config, mixer).await?;
    }

    tokio::signal::ctrl_c().await?;

    Ok(())
}
