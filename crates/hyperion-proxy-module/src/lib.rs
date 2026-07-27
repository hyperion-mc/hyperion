use std::{net::SocketAddr, path::PathBuf};

use flecs_ecs::{core::World, prelude::*};
use hyperion::runtime::AsyncRuntime;
use hyperion_proxy::ProxyIdentity;
use tokio::net::TcpListener;

#[derive(Component)]
pub struct HyperionProxyModule;

/// A proxy hosted inside the game server process, for running both halves on
/// one machine without a second binary.
///
/// Setting this component starts the proxy. There is deliberately no `Default`:
/// the certificate paths belong to whoever launched the process, and guessing
/// them is how the proxy used to fail with a bare "No such file or directory".
#[derive(Component, Debug, Clone)]
pub struct EmbeddedProxy {
    /// Address players connect to.
    pub listen: SocketAddr,
    /// Address of the game server this proxy dials. Its host part must match a
    /// subject alternative name on the game server's certificate.
    pub server: String,
    /// The private certificate authority both ends are signed by.
    pub root_ca_cert: PathBuf,
    /// This proxy's certificate.
    pub cert: PathBuf,
    /// This proxy's private key.
    pub private_key: PathBuf,
}

impl EmbeddedProxy {
    /// Reads this proxy's mTLS material from the configured paths.
    pub fn identity(&self) -> anyhow::Result<ProxyIdentity> {
        ProxyIdentity::from_pem_files(&self.root_ca_cert, &self.cert, &self.private_key)
    }
}

impl Module for HyperionProxyModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world
            .component::<EmbeddedProxy>()
            .add_trait::<flecs::Singleton>();

        embedded_proxy_observer(world);
    }
}

fn embedded_proxy_observer(world: &World) {
    let mut observer = world.observer_named::<flecs::OnSet, (
        &EmbeddedProxy, // (0)
        &AsyncRuntime,  // (1)
    )>("embedded_proxy");

    observer.term_at(1).filter();

    observer.each(|(config, runtime)| {
        // Both the certificates and the address are resolved before the task is
        // spawned, so a bad path or an unresolvable host aborts startup with the
        // offending value instead of panicking inside the runtime.
        let identity = match config.identity() {
            Ok(identity) => identity,
            Err(error) => {
                panic!("embedded proxy has no usable certificates: {error:?}");
            }
        };

        let config = config.clone();

        runtime.spawn(async move {
            let listener = TcpListener::bind(config.listen)
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "embedded proxy failed to bind {listen}: {error}",
                        listen = config.listen
                    )
                });
            tracing::info!("Listening on {listen}", listen = config.listen);

            let server_addr: SocketAddr = tokio::net::lookup_host(&config.server)
                .await
                .ok()
                .and_then(|mut addrs| addrs.next())
                .unwrap_or_else(|| {
                    panic!(
                        "embedded proxy could not resolve game server address {server}",
                        server = config.server
                    )
                });

            hyperion_proxy::run_proxy(listener, server_addr, config.server, identity)
                .await
                .expect("embedded proxy exited with an error");
        });
    });
}
