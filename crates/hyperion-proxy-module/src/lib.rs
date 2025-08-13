use std::{net::SocketAddr, path::Path};

use bevy::prelude::*;
use hyperion::runtime::AsyncRuntime;
use tokio::net::TcpListener;

pub struct HyperionProxyPlugin;

#[derive(Event)]
pub struct SetProxyAddress {
    pub proxy: String,
    pub server: String,
}

impl Default for SetProxyAddress {
    fn default() -> Self {
        Self {
            proxy: "0.0.0.0:25565".to_string(),
            server: "127.0.0.1:35565".to_string(),
        }
    }
}

impl Plugin for HyperionProxyPlugin {
    fn build(&self, app: &mut App) {
        app.add_event::<SetProxyAddress>();
        app.add_observer(update_proxy_address);
    }
}

fn update_proxy_address(trigger: Trigger<'_, SetProxyAddress>, runtime: Res<'_, AsyncRuntime>) {
    let proxy = trigger.proxy.clone();
    let server = trigger.server.clone();

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
            Path::new("root_ca.crt"),
            Path::new("proxy.crt"),
            Path::new("proxy_private_key.pem"),
        )
        .await
        .unwrap();
    });
}
