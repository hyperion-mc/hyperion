use std::{net::SocketAddr, path::PathBuf};

use clap::Parser;
use hyperion::Crypto;
use hyperion_proxy_module::ProxyCerts;
use serde::Deserialize;
use smash::{EmbeddedProxy, init_game};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

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

    /// The port the server should listen on. Defaults to 35565
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

    /// Run a proxy inside this process, listening here. Omit it when a separate
    /// proxy already owns the public port, or the two race for it and the loser
    /// dies in a background task with nothing on stdout to say so.
    #[clap(long)]
    #[serde(default)]
    embedded_proxy: Option<String>,
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

    let args = match envy::prefixed("SMASH_").from_env::<Args>() {
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

    // The embedded proxy is the other end of the same mTLS pair, so on a
    // development box it uses the same certificate as the game server. On a
    // real deployment the proxy is a separate process on a separate machine
    // with its own key, and this flag is simply not passed.
    let embedded_proxy = args.embedded_proxy.map(|listen| EmbeddedProxy {
        listen,
        certs: ProxyCerts {
            root_ca_cert: args.root_ca_cert,
            cert: args.cert,
            private_key: args.private_key,
        },
    });

    init_game(address, embedded_proxy, crypto).unwrap();
}
