use std::{fmt::Debug, net::SocketAddr};

use clap::Parser;
use hyperion_proxy::run_proxy;
use serde::Deserialize;
use tokio::net::TcpListener;
#[cfg(unix)]
use tokio::net::UnixListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[derive(Deserialize, Debug, Parser)]
#[clap(version)]
struct Params {
    /// The address for the proxy to listen on. Can be either:
    /// - A TCP address like "127.0.0.1:25565"
    /// - A Unix domain socket path like "/tmp/minecraft.sock" (Unix only)
    #[serde(default = "default_proxy_addr")]
    proxy_addr: String,

    /// The address of the target Minecraft game server to proxy from/to
    #[clap(short, long, default_value = "127.0.0.1:35565")]
    #[serde(default = "default_server")]
    server: String,
}

fn default_proxy_addr() -> String {
    "0.0.0.0:25565".to_string()
}

fn default_server() -> String {
    "127.0.0.1:35565".to_string()
}

#[derive(Debug)]
enum ProxyAddress {
    Tcp(SocketAddr),
    #[cfg(unix)]
    Unix(PathBuf),
}

use std::{fmt::Display, task::Poll};

use colored::Colorize;
use tokio_util::net::Listener;

impl Display for ProxyAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp(addr) => write!(f, "tcp://{addr}"),
            #[cfg(unix)]
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
        }
    }
}

impl ProxyAddress {
    fn parse(addr: &str) -> Result<Self, Box<dyn std::error::Error>> {
        if addr.contains(':') {
            Ok(Self::Tcp(addr.parse()?))
        } else {
            #[cfg(unix)]
            {
                Ok(Self::Unix(PathBuf::from(addr)))
            }
            #[cfg(not(unix))]
            {
                Err("Unix sockets are not supported on this platform".into())
            }
        }
    }
}

fn setup_logging() {
    // Build a custom subscriber
    tracing_subscriber::fmt()
        .with_ansi(true)
        .with_file(false)
        .with_line_number(false)
        .with_target(false)
        .with_env_filter(EnvFilter::from_default_env())
        .with_max_level(tracing::Level::INFO)
        .init();
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Load .env file if available
    dotenvy::dotenv().ok();

    setup_logging();

    // Try to load params from environment variables
    let params = match envy::prefixed("HYPERION_PROXY_").from_env::<Params>() {
        Ok(params) => {
            info!("Loaded configuration from environment variables");
            params
        }
        Err(e) => {
            info!(
                "Failed to load from environment: {}, falling back to command line arguments",
                e
            );
            Params::parse()
        }
    };

    let proxy_addr = ProxyAddress::parse(&params.proxy_addr)?;

    let server_addr: SocketAddr = tokio::net::lookup_host(&params.server)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve hostname: {}", params.server))?;

    let login_help = "~ The address to connect to".dimmed();

    info!("Starting Hyperion Proxy");
    info!("📡 Public proxy address: {proxy_addr} {login_help}",);

    let server_help = "~ The event server internal address".dimmed();
    info!("👾 Internal server address: tcp://{server_addr} {server_help}");

    let handle = tokio::spawn(async move {
        match &proxy_addr {
            ProxyAddress::Tcp(addr) => {
                let listener = TcpListener::bind(addr).await.unwrap();
                let socket = NoDelayTcpListener { listener };
                run_proxy(socket, server_addr).await.unwrap();
            }
            #[cfg(unix)]
            ProxyAddress::Unix(path) => {
                // remove file if already exists
                let _unused = tokio::fs::remove_file(path).await;
                let listener = UnixListener::bind(path).unwrap();
                run_proxy(listener, server_addr).await.unwrap();
            }
        }
    });

    if let Err(e) = handle.await {
        error!("Proxy task failed: {:?}", e);
    } else {
        info!("Proxy task completed successfully");
    }
    Ok(())
}

struct NoDelayTcpListener {
    listener: TcpListener,
}

impl Listener for NoDelayTcpListener {
    type Addr = <TcpListener as Listener>::Addr;
    type Io = <TcpListener as Listener>::Io;

    fn poll_accept(
        &mut self,
        cx: &mut core::task::Context<'_>,
    ) -> Poll<std::io::Result<(Self::Io, Self::Addr)>> {
        let Poll::Ready(result) = self.listener.poll_accept(cx) else {
            return Poll::Pending;
        };

        let Ok((socket, addr)) = result else {
            return Poll::Ready(result);
        };

        match socket.set_nodelay(true) {
            Ok(..) => Poll::Ready(Ok((socket, addr))),
            Err(e) => Poll::Ready(Err(e)),
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        self.listener.local_addr()
    }
}
