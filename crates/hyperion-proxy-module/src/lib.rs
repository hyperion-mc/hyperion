use std::net::SocketAddr;

use flecs_ecs::{core::World, prelude::*};
use hyperion::runtime::AsyncRuntime;
use tokio::net::TcpListener;

#[derive(Component)]
pub struct HyperionProxyModule;

#[derive(Component)]
pub struct ProxyAddress {
    pub proxy: String,
    pub server: String,
}

impl Default for ProxyAddress {
    fn default() -> Self {
        Self {
            proxy: "0.0.0.0:25565".to_string(),
            server: "127.0.0.1:35565".to_string(),
        }
    }
}

impl Module for HyperionProxyModule {
    fn module(world: &World) {
        world.import::<hyperion::HyperionCore>();
        world.component::<ProxyAddress>();

        proxy_address_observer(world);
    }
}

fn proxy_address_observer(world: &World) {
    let mut query = world.observer_named::<flecs::OnSet, (
        &ProxyAddress, // (0)
        &AsyncRuntime, // (1)
    )>("proxy_address");

    #[rustfmt::skip]
    query
        .term_at(0).singleton()
        .term_at(1).filter().singleton();

    query.each(|(addresses, runtime)| {
        let proxy = addresses.proxy.clone();
        let server = addresses.server.clone();

        runtime.spawn(async move {
            let listener = TcpListener::bind(&proxy).await.unwrap();
            tracing::info!("Listening on {proxy}");

            let server: SocketAddr = tokio::net::lookup_host(&server)
                .await
                .unwrap()
                .next()
                .unwrap();

            hyperion_proxy::run_proxy(listener, server).await.unwrap();
        });
    });
}
