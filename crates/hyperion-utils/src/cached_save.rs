use std::path::PathBuf;

use anyhow::Context;
use bevy::prelude::*;
use futures_util::stream::StreamExt;
use sha2::Digest;
use tar::Archive;
use tokio_util::io::{StreamReader, SyncIoBridge};
use tracing::info;

use crate::AppId;

pub fn cached_save<U: reqwest::IntoUrl + 'static>(
    world: &World,
    url: U,
) -> impl Future<Output = anyhow::Result<PathBuf>> + 'static {
    let project_dirs = world.resource::<AppId>();

    let cache = project_dirs.cache_dir();

    let mut hasher = sha2::Sha256::new();
    let url_str = url.as_str().to_string();
    Digest::update(&mut hasher, url_str.as_bytes());
    // Get the final hash result
    let url_sha = hasher.finalize();
    let url_sha = hex::encode(url_sha);

    let directory = cache.join(url_sha);

    async move {
        if directory.exists() {
            info!("using cached NewYork load");
        } else {
            // download
            let response = reqwest::get(url)
                .await
                .with_context(|| format!("failed to get {url_str}"))?;

            let byte_stream = response.bytes_stream();
            // Convert the byte stream into an AsyncRead

            let reader =
                StreamReader::new(byte_stream.map(|result| result.map_err(std::io::Error::other)));

            let directory = directory.clone();
            let handle = tokio::task::spawn_blocking(move || {
                let reader = SyncIoBridge::new(reader);
                let reader = std::io::BufReader::new(reader);
                let reader = flate2::read::GzDecoder::new(reader);

                // Create the archive in the blocking context
                let mut archive = Archive::new(reader);

                archive
                    .unpack(&directory)
                    .context("failed to unpack archive")?;

                anyhow::Ok(())
            });

            handle.await??;
        }

        Ok(directory)
    }
}
