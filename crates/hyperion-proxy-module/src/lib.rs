use std::{net::SocketAddr, path::PathBuf};

use flecs_ecs::{core::World, prelude::*};
use hyperion::runtime::AsyncRuntime;
use tokio::net::TcpListener;

#[derive(Component)]
pub struct HyperionProxyModule;

#[derive(Component)]
pub struct ProxyAddress {
    pub proxy: String,
    pub server: String,
    /// Where the in-process proxy finds its mTLS material.
    ///
    /// These were hardcoded to bare filenames resolved against the working
    /// directory, so the embedded proxy could only ever start for a server
    /// launched from a directory holding files with exactly those names, and
    /// otherwise died in a background task with the failure only in the log.
    pub certs: ProxyCerts,
}

#[derive(Debug, Clone)]
pub struct ProxyCerts {
    pub root_ca_cert: PathBuf,
    pub cert: PathBuf,
    pub private_key: PathBuf,
}

impl Default for ProxyCerts {
    fn default() -> Self {
        Self {
            root_ca_cert: PathBuf::from("root_ca.crt"),
            cert: PathBuf::from("proxy.crt"),
            private_key: PathBuf::from("proxy_private_key.pem"),
        }
    }
}

impl Default for ProxyAddress {
    fn default() -> Self {
        Self {
            proxy: "0.0.0.0:25565".to_string(),
            server: "127.0.0.1:35565".to_string(),
            certs: ProxyCerts::default(),
        }
    }
}

impl Module for HyperionProxyModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world
            .component::<ProxyAddress>()
            .add_trait::<flecs::Singleton>();

        proxy_address_observer(world);
    }
}

fn proxy_address_observer(world: &World) {
    let mut query = world.observer_named::<flecs::OnSet, (
        &ProxyAddress, // (0)
        &AsyncRuntime, // (1)
    )>("proxy_address");

    query.term_at(1).filter();

    query.each(|(addresses, runtime)| {
        let proxy = addresses.proxy.clone();
        let server = addresses.server.clone();
        let certs = addresses.certs.clone();

        runtime.spawn(async move {
            let listener = TcpListener::bind(&proxy).await.unwrap();
            tracing::info!("Listening on {proxy}");

            let addr: SocketAddr = tokio::net::lookup_host(&server)
                .await
                .unwrap()
                .next()
                .unwrap();

            hyperion_proxy::run_proxy(
                listener,
                addr,
                server.clone(),
                &certs.root_ca_cert,
                &certs.cert,
                &certs.private_key,
            )
            .await
            .unwrap();
        });
    });
}
