use antithesis_bot::LaunchArguments;
use tracing::warn;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    if let Err(e) = dotenvy::dotenv() {
        warn!("Failed to load .env file: {}", e);
    }

    // Deserialize environment variables into the struct
    let args: LaunchArguments = envy::from_env()?;

    antithesis_bot::start(args).await?;

    Ok(())
}
