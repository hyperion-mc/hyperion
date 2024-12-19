#![allow(clippy::similar_names, reason = "todo: fix")]

#[cfg(unix)]
use std::path::PathBuf;
use std::{
    env,
    net::ToSocketAddrs,
    sync::{Arc, atomic::AtomicU32},
};

use rust_mc_bot::{Address, BotManager};

#[cfg(unix)]
const UDS_PREFIX: &str = "unix://";

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        let name = args.first().unwrap();
        #[cfg(unix)]
        tracing::error!("usage: {name} <ip:port or path> <count> [threads]");
        #[cfg(not(unix))]
        tracing::error!("usage: {} <ip:port> <count> [threads]", name);
        tracing::error!("example: {name} localhost:25565 500");
        #[cfg(unix)]
        tracing::error!("example: {name} unix:///path/to/socket 500");
        return;
    }

    let arg1 = args.get(1).unwrap();
    let arg2 = args.get(2).unwrap();
    let arg3 = args.get(3);

    let mut addrs = arg1
        .strip_prefix(UDS_PREFIX)
        .map(|path| Address::UNIX(PathBuf::from(path)));

    if addrs.is_none() {
        let mut parts = arg1.split(':');
        let ip = parts.next().expect("no ip provided");
        let port = parts.next().map_or(25565u16, |port_string| {
            port_string.parse().expect("invalid port")
        });

        let server = (ip, port)
            .to_socket_addrs()
            .expect("Not a socket address")
            .next()
            .expect("No socket address found");

        addrs = Some(Address::TCP(server));
    }

    // Cant be none because it would have panicked earlier
    let addrs = addrs.unwrap();

    let count: u32 = arg2
        .parse()
        .unwrap_or_else(|_| panic!("{arg2} is not a number"));

    let mut cpus = 1.max(num_cpus::get());

    if let Some(str) = arg3 {
        cpus = str
            .parse()
            .unwrap_or_else(|_| panic!("{arg2} is not a number"));
    }

    tracing::info!("cpus: {cpus}");

    let bot_on = Arc::new(AtomicU32::new(0));

    if count > 0 {
        let mut threads = Vec::new();
        for _ in 0..cpus {
            let addrs = addrs.clone();
            let bot_on = bot_on.clone();
            threads.push(std::thread::spawn(move || {
                let mut manager = BotManager::create(count, addrs, bot_on).unwrap();
                manager.game_loop();
            }));
        }

        for thread in threads {
            let _unused = thread.join();
        }
    }
}
