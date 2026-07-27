#![allow(clippy::similar_names, reason = "todo: fix")]

use std::{
    net::ToSocketAddrs,
    path::PathBuf,
    sync::{Arc, atomic::AtomicU32},
};

use clap::Parser;
use rust_mc_bot::{Address, BotManager};

const UDS_PREFIX: &str = "unix://";

/// Both settings are positional as well as environment-backed, so
/// `rust-mc-bot 127.0.0.1:25565 100` and `BOT_BOT_COUNT=100 rust-mc-bot` mean
/// the same thing.
///
/// This used to read the environment through `envy`, whose prefix handling is
/// `trim_start_matches("BOT_")` — it strips the prefix repeatedly, so
/// `BOT_BOT_COUNT` arrived as `count`, never matched `bot_count`, and every run
/// silently used the default of 500 per thread. clap matches variable names
/// exactly.
#[derive(Parser, Debug)]
#[clap(version)]
struct Config {
    /// Server address, as `hostname:port` or `unix://path`
    #[clap(env = "BOT_SERVER", default_value = "127.0.0.1:25565")]
    server: String,

    /// Total number of bots to spawn, across all threads
    #[clap(env = "BOT_BOT_COUNT", default_value_t = 500)]
    bot_count: u32,

    /// Number of threads to spread the bots across. Defaults to the core count.
    #[clap(short, long, env = "BOT_THREADS")]
    threads: Option<usize>,
}

fn main() {
    tracing_subscriber::fmt::init();

    let config = Config::parse();
    let threads = config
        .threads
        .unwrap_or_else(|| 1_usize.max(num_cpus::get()));

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

    let bot_on = Arc::new(AtomicU32::new(0));

    if config.bot_count > 0 {
        // Every manager is given the whole count, and `bot_on` — which they
        // share — is what caps the total. Handing each thread a slice of the
        // count instead makes them all stop at one slice's worth.
        let thread_count =
            threads.clamp(1, usize::try_from(config.bot_count).unwrap_or(usize::MAX));

        tracing::info!(
            "connecting {count} bots to {server} across {thread_count} threads",
            count = config.bot_count,
            server = config.server
        );

        let threads: Vec<_> = (0..thread_count)
            .map(|_| {
                let count = config.bot_count;
                let addrs = addrs.clone();
                let bot_on = bot_on.clone();
                std::thread::spawn(move || {
                    let mut manager = BotManager::create(count, addrs, bot_on).unwrap();
                    manager.game_loop();
                })
            })
            .collect();

        for thread in threads {
            let _unused = thread.join();
        }
    }
}
