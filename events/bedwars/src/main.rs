use std::net::SocketAddr;

use bedwars::init_game;
use clap::Parser;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};
// use tracing_tracy::TracyLayer;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The arguments to run the server
#[derive(Parser, Deserialize, Debug)]
struct Args {
    /// The IP address the server should listen on. Defaults to 0.0.0.0
    #[clap(short, long, default_value = "0.0.0.0")]
    #[serde(default = "default_ip")]
    ip: String,

    /// The port the server should listen on. Defaults to 25565
    #[clap(short, long, default_value = "35565")]
    #[serde(default = "default_port")]
    port: u16,
}

fn default_ip() -> String {
    "0.0.0.0".to_string()
}

const fn default_port() -> u16 {
    35565
}

fn setup_logging() {
    tracing::subscriber::set_global_default(
        Registry::default()
            .with(EnvFilter::from_default_env())
            // .with(TracyLayer::default())
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_file(true)
                    .with_line_number(true),
            ),
    )
    .expect("setup tracing subscribers");
}

fn main() {
    dotenvy::dotenv().ok();

    setup_logging();

    // Try to load config from environment variables
    let args = match envy::prefixed("BEDWARS_").from_env::<Args>() {
        Ok(args) => {
            tracing::info!("Loaded configuration from environment variables");
            args
        }
        Err(e) => {
            tracing::info!(
                "Failed to load from environment: {}, falling back to command line arguments",
                e
            );
            Args::parse()
        }
    };

    let address = format!("{ip}:{port}", ip = args.ip, port = args.port);
    let address = address.parse::<SocketAddr>().unwrap();

    init_game(address).unwrap();
}
