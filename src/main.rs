use anyhow::Result;

mod constants;
mod bus;
mod mixer;
mod net;
mod sources;
mod stdin;

#[tokio::main]
async fn main() -> Result<()> {
    let bus = bus::start();

    let _sine_source1 = sources::sine::start(440.0);
    let _sine_source2 = sources::sine::start(640.0);
    let espeak_source = sources::espeak::start(&bus);
    let symphonia_source = sources::symphonia::start("./rickroll.m4a");

    let mixer_output = mixer::start(
        &bus,
        vec![
            espeak_source,
            symphonia_source,
            // sine_source1,
            // sine_source2
        ],
    )?;

    net::start(mixer_output);

    stdin::start(bus);

    tokio::signal::ctrl_c().await?;

    Ok(())
}
