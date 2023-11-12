use anyhow::Result;

mod buffer;
mod bus;
mod constants;
mod irc;
mod mixer;
mod net;
mod playback;
mod songleader;
mod sources;
mod stdin;

#[tokio::main]
async fn main() -> Result<()> {
    let bus = bus::init();

    let _sine_source1 = sources::sine::init(440.0);
    let _sine_source2 = sources::sine::init(640.0);
    let espeak_source = sources::espeak::init(&bus);
    let symphonia_source = sources::symphonia::init(&bus);

    let mixer_output = mixer::init(
        &bus,
        vec![
            espeak_source,
            symphonia_source,
            // sine_source1,
            // sine_source2
        ],
    )?;

    playback::init(&bus).await;
    irc::init(&bus).await?;
    songleader::init(&bus).await;
    net::init(mixer_output);
    // stdin::init(&bus);
    bus::debug(&bus);

    tokio::signal::ctrl_c().await?;

    Ok(())
}
