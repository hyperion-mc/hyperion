use antithesis_bot::LaunchArguments;
use eyre::Result;
use serde::Deserialize;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    // Load .env file first
    if let Err(e) = dotenvy::dotenv() {
        warn!("Failed to load .env file: {}", e);
    }

    // Deserialize environment variables into the struct
    let args: LaunchArguments = envy::from_env()?;

    antithesis_bot::start(args).await?;

    Ok(())
}
