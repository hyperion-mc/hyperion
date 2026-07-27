//! The command line every event binary shares.
//!
//! An event is a world plus an `init_game`. Everything around that call is the
//! same for all of them: the same five arguments, the same environment
//! override, the same logging. It used to be the same ninety lines copied into
//! each event's `main.rs`, and the copies had drifted: the listen address was
//! built by joining the IP and the port with a colon and parsing the result, so
//! the documented IPv6 default `::` became `::35565` and every IPv6 launch
//! panicked on `AddrParseError`. Both copies. One of them is the default this
//! server ships, so that default had never started.
//!
//! Here the address is a [`SocketAddr`] built from an [`IpAddr`] and a `u16`,
//! which cannot be spelled wrong.

use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use clap::Parser;
use hyperion::Crypto;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

/// What every event binary takes.
#[derive(Parser, Deserialize, Debug)]
pub struct Args {
    /// The address the server listens on.
    #[clap(short, long, default_value = "0.0.0.0")]
    #[serde(default = "default_ip")]
    pub ip: IpAddr,

    /// The port the server listens on.
    #[clap(short, long, default_value = "35565")]
    #[serde(default = "default_port")]
    pub port: u16,

    /// The file path to the root certificate authority's certificate.
    #[clap(long)]
    pub root_ca_cert: PathBuf,

    /// The file path to the game server's certificate.
    #[clap(long)]
    pub cert: PathBuf,

    /// The file path to the game server's private key.
    #[clap(long)]
    pub private_key: PathBuf,
}

impl Args {
    /// Where the server listens.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }
}

const fn default_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
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

/// Read the arguments, set up logging, and hand the event its world.
///
/// `env_prefix` is the prefix of the environment variables that stand in for
/// the arguments, so `bedwars` reads `BEDWARS_PORT` and smash reads
/// `SMASH_PORT`. The environment wins when it carries a complete set, and the
/// command line is used otherwise.
///
/// # Errors
/// Returns whatever `init_game` returns, or an error reading the TLS material.
pub fn run(
    env_prefix: &str,
    init_game: impl FnOnce(SocketAddr, Crypto) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    setup_logging();

    let args = match envy::prefixed(env_prefix).from_env::<Args>() {
        Ok(args) => {
            tracing::info!("loaded configuration from environment variables");
            args
        }
        Err(error) => {
            tracing::info!("reading the command line instead of the environment: {error}");
            Args::parse()
        }
    };

    let crypto = Crypto::new(&args.root_ca_cert, &args.cert, &args.private_key)?;

    init_game(args.address(), crypto)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    fn args_from(ip: &str) -> Args {
        Args::parse_from([
            "event",
            "--ip",
            ip,
            "--root-ca-cert",
            "/dev/null",
            "--cert",
            "/dev/null",
            "--private-key",
            "/dev/null",
        ])
    }

    /// The bug this crate exists to make unspellable. Both events used to join
    /// the IP and the port with a colon and parse the result, so `::` became
    /// `::35565` and every IPv6 launch died on `AddrParseError`.
    #[test]
    fn an_ipv6_listen_address_survives_reaching_the_socket() {
        assert_eq!(args_from("::").address().to_string(), "[::]:35565");
    }

    #[test]
    fn the_default_is_every_ipv4_address() {
        assert_eq!(
            Args::parse_from([
                "event",
                "--root-ca-cert",
                "/dev/null",
                "--cert",
                "/dev/null",
                "--private-key",
                "/dev/null",
            ])
            .address()
            .to_string(),
            "0.0.0.0:35565"
        );
    }
}
