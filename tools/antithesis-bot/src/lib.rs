use antithesis::random::AntithesisRng;
use rand::Rng;
use serde::Deserialize;

mod bot;

#[derive(Deserialize, Debug)]
pub struct LaunchArguments {
    ip: String,
    bot_count: u32,
}

pub fn bootstrap(args: LaunchArguments) {
    const UNUSUALLY_HIGH_BOT_THRESHOLD: u32 = 1_000;

    tracing::info!("args = {args:?}");

    let LaunchArguments { ip, bot_count } = args;

    if bot_count > UNUSUALLY_HIGH_BOT_THRESHOLD {
        tracing::warn!("bot_count {bot_count} is unusually high. This may cause issues.");
    }

    let sender: char = AntithesisRng.random();
    let mut bot = antithesis::Bot::new(ip, bot_count);
}
