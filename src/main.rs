use anyhow::Result;

mod constants;
mod mixer;
mod net;
mod sources;

#[tokio::main]
async fn main() -> Result<()> {
    let sine_source1 = sources::sine::start(440.0).await;
    let sine_source2 = sources::sine::start(640.0).await;

    let symphonia_source = sources::symphonia::start("./rickroll.m4a");

    let mixer_output = mixer::start(vec![
        symphonia_source,
        // sine_source1,
        // sine_source2
    ])
    .await?;
    net::start(mixer_output).await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
