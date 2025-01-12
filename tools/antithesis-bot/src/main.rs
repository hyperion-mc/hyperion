use antithesis_bot::LaunchArguments;
use tracing::trace;

#[tokio::main]
async fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    if let Err(e) = dotenvy::dotenv() {
        trace!("Failed to load .env file: {}", e);
    }

    // Deserialize environment variables into the struct
    let args: LaunchArguments = envy::from_env()?;

    antithesis_bot::start(args).await?;

    Ok(())
}
