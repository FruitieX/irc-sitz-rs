use anyhow::Result;

mod mixer;
mod net;
mod sine;

#[tokio::main]
async fn main() -> Result<()> {
    let sine_source = sine::start().await;

    let mixer_channel = mixer::start().await?;
    net::start(mixer_channel).await?;

    tokio::signal::ctrl_c().await?;

    Ok(())
}
