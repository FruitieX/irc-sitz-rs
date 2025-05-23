#[macro_use]
extern crate log;

mod buffer;
mod config;
mod constants;
mod event;
mod irc;
mod mixer;
mod net;
mod playback;
mod songbook;
mod songleader;
mod sources;
mod stdin;
mod youtube;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init_timed();

    let config = config::load().await?;
    let bus = event::EventBus::new();

    // let sine_source1 = sources::sine::init(440.0);
    // let sine_source2 = sources::sine::init(640.0);
    let espeak_source = sources::espeak::init(&bus);
    let symphonia_source = sources::symphonia::init(&bus).await?;

    let mixer_output = mixer::init(
        &bus,
        vec![
            espeak_source,
            symphonia_source,
            // sine_source1,
            // sine_source2
        ],
    )?;

    youtube::init().await?;
    playback::init(&bus).await;
    irc::init(&bus, &config).await?;
    songleader::init(&bus, &config).await;
    net::init(mixer_output);
    event::debug(&bus);

    // stdin::init(&bus);
    tokio::signal::ctrl_c().await?;

    Ok(())
}
