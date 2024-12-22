use antithesis::random::AntithesisRng;
use rand::Rng;
use serde::Deserialize;

mod bot;

#[derive(Deserialize, Debug)]
pub struct LaunchArguments {
    ip: String,

    #[serde(default = "default_bot_count")]
    bot_count: u32,
}

const fn default_bot_count() -> u32 {
    1
}

pub async fn start(args: LaunchArguments) -> eyre::Result<()> {
    const UNUSUALLY_HIGH_BOT_THRESHOLD: u32 = 1_000;

    tracing::info!("args = {args:?}");

    let LaunchArguments { ip, bot_count } = args;

    if bot_count > UNUSUALLY_HIGH_BOT_THRESHOLD {
        tracing::warn!("bot_count {bot_count} is unusually high. This may cause issues.");
    }

    for _ in 0..bot_count {
        bot::launch(&ip).await?;
    }

    Ok(())
}
