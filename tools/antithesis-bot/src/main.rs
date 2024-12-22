use eyre::Result;
use serde::Deserialize;
use tracing::{info, warn};

#[derive(Deserialize)]
struct Args {
    ip: String,
}

fn main() -> Result<()> {
    // Load .env file first
    if let Err(e) = dotenvy::dotenv() {
        warn!("Failed to load .env file: {}", e);
    }
    
    // Deserialize environment variables into the struct
    let args: Args = envy::from_env()?;
    info!(?args.ip, "Using IP address");

    Ok(())
}