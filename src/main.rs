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
    let bus = bus::start();

    let _sine_source1 = sources::sine::start(440.0);
    let _sine_source2 = sources::sine::start(640.0);
    let espeak_source = sources::espeak::start(&bus);
    let symphonia_source = sources::symphonia::start(&bus);

    let mixer_output = mixer::start(
        &bus,
        vec![
            espeak_source,
            symphonia_source,
            // sine_source1,
            // sine_source2
        ],
    )?;

    irc::start(&bus).await?;
    songleader::start(&bus).await;
    net::start(mixer_output);
    // stdin::start(&bus);
    bus::debug(&bus);

    tokio::signal::ctrl_c().await?;

    Ok(())
}
