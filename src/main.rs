use anyhow::Result;

mod constants;
mod mixer;
mod net;
mod sources;

#[tokio::main]
async fn main() -> Result<()> {
    let _sine_source1 = sources::sine::start(440.0);
    let _sine_source2 = sources::sine::start(640.0);

    let espeak_source = sources::espeak::start();
    let symphonia_source = sources::symphonia::start("./rickroll.m4a");

    let mixer_output = mixer::start(vec![
        espeak_source,
        symphonia_source,
        // sine_source1,
        // sine_source2
    ])?;
    net::start(mixer_output).await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
