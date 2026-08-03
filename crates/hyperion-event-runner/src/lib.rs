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
    io::IsTerminal,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, bail};
use clap::Parser;
use hyperion::Crypto;
use hyperion_web_console::Config as ConsoleConfig;
use serde::Deserialize;
use tracing_subscriber::{EnvFilter, Registry, filter::LevelFilter, layer::SubscriberExt};

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

    /// The reloadable rules dylib: loaded once at startup, and again on every
    /// reload request.
    ///
    /// A stable path outside the store, so that deploying a new build of the
    /// rules does not change this string and therefore does not change the
    /// unit's `[Service]` section -- which is the condition under which systemd
    /// reloads rather than restarts. See `nix/modules/game-server.nix`.
    #[clap(long, requires_all = ["reload_socket", "build_stamp"])]
    #[serde(default)]
    pub rules: Option<PathBuf>,

    /// Where to listen for reload requests. `ExecReload` runs a client that
    /// connects here; see `hyperion-reload-client`.
    #[clap(long, requires_all = ["rules", "build_stamp"])]
    #[serde(default)]
    pub reload_socket: Option<PathBuf>,

    /// Directory holding the build stamp files the deploy writes.
    ///
    /// A directory read at runtime rather than three variables in the
    /// environment, because a process's environment is fixed at `exec` and a
    /// hot reload's entire point is that there is no new `exec` to fix it. A
    /// server that reloaded would otherwise report the build it started as,
    /// forever.
    #[clap(long, requires_all = ["rules", "reload_socket"])]
    #[serde(default)]
    pub build_stamp: Option<PathBuf>,

    /// Where to serve the operator console, or nowhere.
    ///
    /// Absent means no console at all: no socket is opened and nothing is
    /// spent. An address here is an admin surface, so the value an operator
    /// should reach for is a loopback or an internal one; see
    /// [`hyperion_web_console`] for what it exposes.
    ///
    /// `requires` runs this way round and not the other because only one
    /// direction is dangerous. A token file with no bind address is a console
    /// that is simply off; a bind address with no token file is an open admin
    /// port that looks configured.
    #[clap(long, requires = "console_token_file")]
    #[serde(default)]
    pub console_bind: Option<SocketAddr>,

    /// The file holding the console's bearer token.
    ///
    /// A file rather than an argument or an environment variable, because both
    /// of those are readable by anything that can list processes. A systemd
    /// `LoadCredential` puts one here.
    #[clap(long)]
    #[serde(default)]
    pub console_token_file: Option<PathBuf>,
}

/// The paths a packaged deployment hands the server.
///
/// All three or none of them. They are separate options rather than one
/// directory because the deployment, not the event, decides where each lives,
/// and an event that derived `<dir>/<its own name>-rules.so` would be a second
/// place that has to agree with the NixOS module about a filename.
#[derive(Debug, Clone)]
pub struct Deployment {
    /// The rules dylib to load, and to reload.
    pub rules: PathBuf,
    /// The socket a reload is asked for on.
    pub reload_socket: PathBuf,
    /// The directory the build stamp files are in.
    pub build_stamp: PathBuf,
}

impl Args {
    /// Where the server listens.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        SocketAddr::new(self.ip, self.port)
    }

    /// The deployment paths, when this process was started by one.
    ///
    /// Clap enforces all-or-nothing on the command line; this is the same rule
    /// for the environment, which `envy` deserializes without consulting clap
    /// at all. Refusing a partial set rather than degrading to "no reload" is
    /// deliberate: a deploy that passed two of the three and silently lost the
    /// ability to reload would look exactly like one that never asked for it.
    ///
    /// # Errors
    /// If some of the three are set and some are not, naming which are missing.
    pub fn deployment(&self) -> anyhow::Result<Option<Deployment>> {
        match (&self.rules, &self.reload_socket, &self.build_stamp) {
            (None, None, None) => Ok(None),
            (Some(rules), Some(reload_socket), Some(build_stamp)) => Ok(Some(Deployment {
                rules: rules.clone(),
                reload_socket: reload_socket.clone(),
                build_stamp: build_stamp.clone(),
            })),
            (rules, socket, stamp) => {
                let missing: Vec<&str> = [
                    ("--rules", rules.is_none()),
                    ("--reload-socket", socket.is_none()),
                    ("--build-stamp", stamp.is_none()),
                ]
                .into_iter()
                .filter_map(|(name, absent)| absent.then_some(name))
                .collect();
                bail!("a partial deployment: {} not set", missing.join(", "))
            }
        }
    }

    /// How to run the operator console, or `None` when it was not asked for.
    ///
    /// # Errors
    /// Fails when a bind address was given without a token file, when the file
    /// cannot be read, or when it is empty once trimmed. All three are refused
    /// at startup rather than warned about: every one of them ends in a console
    /// that either is not there or has no password, and an operator finding
    /// that out later finds it out the wrong way.
    pub fn console(&self) -> anyhow::Result<Option<ConsoleConfig>> {
        let Some(address) = self.console_bind else {
            return Ok(None);
        };

        // `clap` already refuses this on the command line; the environment path
        // does not go through clap at all, so this is the check that covers
        // `SMASH_CONSOLE_BIND` set without its token. Same shape, and same
        // reason, as `deployment` above.
        let Some(path) = self.console_token_file.as_ref() else {
            bail!(
                "the console was asked for at {address} with no --console-token-file, which would \
                 be an admin port with no password"
            );
        };

        let token = std::fs::read_to_string(path)
            .with_context(|| format!("reading the console token from {}", path.display()))?;
        // Trimmed because a token file written by a person, or by systemd's
        // credential machinery, ends in a newline that is not part of the
        // secret. A token compared with the newline still on it fails every
        // request and looks like a wrong password.
        let token = token.trim().to_owned();
        anyhow::ensure!(
            !token.is_empty(),
            "the console token file {} is empty",
            path.display()
        );

        Ok(Some(ConsoleConfig { address, token }))
    }
}

const fn default_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

const fn default_port() -> u16 {
    35565
}

/// Logging, at a level and in a form that survives the trip to a journal.
///
/// # `info` by default, rather than whatever `RUST_LOG` happens to say
///
/// `EnvFilter::from_default_env()` with `RUST_LOG` unset builds a filter with no directives
/// at all, which passes nothing. A deployed unit sets no `RUST_LOG`, so the server was
/// silent: not quiet, silent -- no startup line, no map built, and no `hot reload accepted`
/// either, which is the one line an operator needs after a deploy that is supposed to be
/// invisible. Measured on dev-compute-6: the same server, same unit, with and without
/// `RUST_LOG`, is zero journal lines versus every line below.
///
/// `RUST_LOG` still wins when it is set, so narrowing or widening is a variable away.
///
/// # No ANSI unless something is there to render it
///
/// `fmt::layer()` colours its output whatever it is writing to, so a service wrote
/// `\x1b[32m INFO\x1b[0m` into the journal and every `grep 'INFO'` over it matched
/// nothing. The escapes are dropped when stdout is not a terminal, which is exactly the
/// case where nobody can see them and something is probably grepping.
fn setup_logging() {
    let filter = EnvFilter::builder()
        .with_default_directive(LevelFilter::INFO.into())
        .from_env_lossy();

    tracing::subscriber::set_global_default(
        Registry::default().with(filter).with(
            tracing_subscriber::fmt::layer()
                .with_ansi(std::io::stdout().is_terminal())
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
    init_game: impl FnOnce(&Args, Crypto) -> anyhow::Result<()>,
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

    init_game(&args, crypto)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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

    /// Nothing set is the developer's server, and it is not a misconfiguration.
    #[test]
    fn a_server_with_no_deployment_paths_has_no_deployment() {
        assert!(
            args_from("::")
                .deployment()
                .expect("all three absent is legal")
                .is_none()
        );
    }

    /// Two of three has to fail, and has to say which one is missing.
    ///
    /// Clap refuses this on the command line -- `requires_all` -- so the way it
    /// reaches a running server is `envy`, which reads the environment and
    /// knows nothing about clap's argument groups. That is the path the
    /// deployed server takes.
    #[test]
    fn a_partial_deployment_is_refused_by_name() {
        let mut args = args_from("::");
        args.rules = Some(PathBuf::from("/etc/hyperion/smash-rules.so"));
        args.reload_socket = Some(PathBuf::from("/run/hyperion/reload.sock"));

        let error = args
            .deployment()
            .expect_err("two of three must not read as a working deployment")
            .to_string();
        assert!(error.contains("--build-stamp"), "{error}");
        assert!(!error.contains("--rules"), "{error}");
    }

    #[test]
    fn all_three_together_are_a_deployment() {
        let mut args = args_from("::");
        args.rules = Some(PathBuf::from("/etc/hyperion/smash-rules.so"));
        args.reload_socket = Some(PathBuf::from("/run/hyperion/reload.sock"));
        args.build_stamp = Some(PathBuf::from("/etc/hyperion"));

        let deployment = args
            .deployment()
            .expect("all three set")
            .expect("all three set");
        assert_eq!(
            deployment.rules,
            PathBuf::from("/etc/hyperion/smash-rules.so")
        );
        assert_eq!(deployment.build_stamp, PathBuf::from("/etc/hyperion"));
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
