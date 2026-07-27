use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
};

use bedwars::init_game;
use clap::Parser;
use hyperion::Crypto;
use hyperion_proxy_module::EmbeddedProxy;
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

    /// The file path to the root certificate authority's certificate
    #[clap(long)]
    root_ca_cert: PathBuf,

    /// The file path to the game server's certificate
    #[clap(long)]
    cert: PathBuf,

    /// The file path to the game server's private key
    #[clap(long)]
    private_key: PathBuf,

    /// Host a proxy in this process on the given address, so a single-machine
    /// setup needs one binary. Omit it to run the game server alone and connect
    /// a separate `hyperion-proxy`.
    #[clap(long, requires_all = ["proxy_cert", "proxy_private_key"])]
    #[serde(default)]
    proxy_addr: Option<SocketAddr>,

    /// The file path to the embedded proxy's certificate
    #[clap(long)]
    #[serde(default)]
    proxy_cert: Option<PathBuf>,

    /// The file path to the embedded proxy's private key
    #[clap(long)]
    #[serde(default)]
    proxy_private_key: Option<PathBuf>,
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
    let crypto = Crypto::new(&args.root_ca_cert, &args.cert, &args.private_key).unwrap();

    let embedded_proxy = args.proxy_addr.map(|listen| EmbeddedProxy {
        listen,
        server: SocketAddr::new(dial_address(address.ip()), address.port()).to_string(),
        root_ca_cert: args.root_ca_cert.clone(),
        // clap's `requires_all` guarantees both are present whenever
        // `--proxy-addr` is.
        cert: args.proxy_cert.clone().expect("--proxy-cert is required"),
        private_key: args
            .proxy_private_key
            .clone()
            .expect("--proxy-private-key is required"),
    });

    init_game(address, crypto, embedded_proxy).unwrap();
}

/// The address the embedded proxy dials the game server on.
///
/// A wildcard bind address is not a destination, and it is not a name any
/// certificate can carry a SAN for, so an unspecified listen address becomes
/// loopback.
const fn dial_address(listen: IpAddr) -> IpAddr {
    if listen.is_unspecified() {
        match listen {
            IpAddr::V4(..) => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
            IpAddr::V6(..) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        }
    } else {
        listen
    }
}
