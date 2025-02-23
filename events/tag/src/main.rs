use std::net::SocketAddr;

use clap::Parser;
use tag::init_game;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};
// use tracing_tracy::TracyLayer;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// The arguments to run the server
#[derive(Parser)]
struct Args {
    /// The IP address the server should listen on. Defaults to 0.0.0.0
    #[clap(short, long, default_value = "0.0.0.0")]
    ip: String,
    /// The port the server should listen on. Defaults to 25565
    #[clap(short, long, default_value = "35565")]
    port: u16,
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

    let Args { ip, port } = Args::parse();

    let address = format!("{ip}:{port}");
    let address = address.parse::<SocketAddr>().unwrap();

    init_game(address).unwrap();
}
