#![allow(clippy::similar_names, reason = "todo: fix")]

use std::{
    net::ToSocketAddrs,
    sync::{Arc, atomic::AtomicU32},
};

use rust_mc_bot::{Address, BotManager};
use serde::Deserialize;

const UDS_PREFIX: &str = "unix://";

#[derive(Deserialize, Debug)]
#[allow(clippy::doc_markdown)]
struct Config {
    /// Server address (hostname:port or unix://path)
    #[serde(default = "default_server")]
    server: String,

    /// Number of bots to spawn
    #[serde(default = "default_bot_count")]
    bot_count: u32,

    /// Number of threads to use
    #[serde(default = "default_threads")]
    threads: usize,
}

fn default_server() -> String {
    "hyperion-proxy:25565".to_string()
}

const fn default_bot_count() -> u32 {
    500
}

fn default_threads() -> usize {
    1_usize.max(num_cpus::get())
}

fn main() {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt::init();

    // Load config from environment variables
    let config = match envy::prefixed("BOT_").from_env::<Config>() {
        Ok(config) => {
            tracing::info!("Loaded configuration from environment variables");
            config
        }
        Err(e) => {
            tracing::error!(
                "Failed to load configuration from environment variables: {}",
                e
            );
            tracing::info!(
                "Configure using BOT_SERVER, BOT_BOT_COUNT, and BOT_THREADS environment variables"
            );
            tracing::info!(
                "Default values: BOT_SERVER={}, BOT_BOT_COUNT={}, BOT_THREADS={}",
                default_server(),
                default_bot_count(),
                default_threads()
            );
            return;
        }
    };

    let addrs: Address = if config.server.starts_with(UDS_PREFIX) {
        #[cfg(unix)]
        {
            Address::UNIX(PathBuf::from(
                config.server.strip_prefix(UDS_PREFIX).unwrap(),
            ))
        }
        #[cfg(not(unix))]
        {
            tracing::error!("unix sockets are not supported on your platform");
            return;
        }
    } else {
        let mut parts = config.server.split(':');
        let ip = parts.next().expect("no ip provided");
        let port = parts.next().map_or(25565u16, |port_string| {
            port_string.parse().expect("invalid port")
        });

        let server = (ip, port)
            .to_socket_addrs()
            .expect("Not a socket address")
            .next()
            .expect("No socket address found");

        Address::TCP(server)
    };

    tracing::info!("cpus: {}", config.threads);

    let bot_on = Arc::new(AtomicU32::new(0));

    if config.bot_count > 0 {
        let mut threads = Vec::new();
        for _ in 0..config.threads {
            let addrs = addrs.clone();
            let bot_on = bot_on.clone();
            threads.push(std::thread::spawn(move || {
                let mut manager = BotManager::create(config.bot_count, addrs, bot_on).unwrap();
                manager.game_loop();
            }));
        }

        for thread in threads {
            let _unused = thread.join();
        }
    }
}
