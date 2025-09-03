//! See [`MojangClient`].

use std::{sync::Arc, time::Duration};

use anyhow::{Context, bail};
use bevy::prelude::*;
use serde_json::Value;
use tokio::{
    sync::Semaphore,
    time::{MissedTickBehavior, interval},
};
use tracing::warn;
use uuid::Uuid;

use crate::AsyncRuntime;

/// The API provider to use for Minecraft profile lookups
#[derive(Clone, Copy)]
pub struct ApiProvider {
    username_base_url: &'static str,
    uuid_base_url: &'static str,
    max_requests: usize,
    interval: Duration,
}

impl ApiProvider {
    /// The matdoes.dev API mirror provider with higher rate limits
    pub const MAT_DOES_DEV: Self = Self {
        username_base_url: "https://mowojang.matdoes.dev/users/profiles/minecraft",
        uuid_base_url: "https://mowojang.matdoes.dev/session/minecraft/profile",
        max_requests: 10_000,
        interval: Duration::from_secs(1),
    };
    /// The official Mojang API provider
    pub const MOJANG: Self = Self {
        username_base_url: "https://api.mojang.com/users/profiles/minecraft",
        uuid_base_url: "https://sessionserver.mojang.com/session/minecraft/profile",
        max_requests: 600,
        interval: Duration::from_secs(10 * 60),
    };

    fn username_url(&self, username: &str) -> String {
        format!("{}/{username}", self.username_base_url)
    }

    fn uuid_url(&self, uuid: &Uuid) -> String {
        format!("{}/{uuid}?unsigned=false", self.uuid_base_url)
    }

    const fn max_requests(&self) -> usize {
        self.max_requests
    }

    const fn interval(&self) -> Duration {
        self.interval
    }
}

/// A client to interface with the Minecraft profile API.
///
/// Can use either the official Mojang API or [matdoes/mowojang](https://matdoes.dev/minecraft-uuids) as a data source.
/// This does not include caching, this should be done separately probably using [`crate::storage::LocalDb`].
#[derive(Resource, Clone)]
pub struct MojangClient {
    req: reqwest::Client,
    rate_limit: Arc<Semaphore>,
    provider: ApiProvider,
}

impl MojangClient {
    #[must_use]
    pub fn new(runtime: &AsyncRuntime, provider: ApiProvider) -> Self {
        let rate_limit = Arc::new(Semaphore::new(provider.max_requests()));
        let interval_duration = provider.interval();

        runtime.spawn({
            let rate_limit = Arc::downgrade(&rate_limit);
            let max_requests = provider.max_requests();
            async move {
                let mut interval = interval(interval_duration);
                interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

                loop {
                    interval.tick().await;

                    let Some(rate_limit) = rate_limit.upgrade() else {
                        return;
                    };

                    let available = rate_limit.available_permits();
                    rate_limit.add_permits(max_requests - available);
                }
            }
        });

        Self {
            req: reqwest::Client::new(),
            rate_limit,
            provider,
        }
    }

    /// Gets a player's UUID from their username.
    pub async fn get_uuid(&self, username: &str) -> anyhow::Result<Uuid> {
        let url = self.provider.username_url(username);
        let json_object = self.response_raw(&url).await?;

        let id = json_object
            .get("id")
            .context("no id in json")?
            .as_str()
            .context("id is not a string")?;

        Uuid::parse_str(id).map_err(Into::into)
    }

    /// Gets a player's username from their UUID.
    pub async fn get_username(&self, uuid: Uuid) -> anyhow::Result<String> {
        let url = self.provider.uuid_url(&uuid);
        let json_object = self.response_raw(&url).await?;

        json_object
            .get("name")
            .context("no name in json")?
            .as_str()
            .map(String::from)
            .context("Username not found")
    }

    /// Gets player data from their UUID.
    pub async fn data_from_uuid(&self, uuid: &Uuid) -> anyhow::Result<Value> {
        let url = self.provider.uuid_url(uuid);
        self.response_raw(&url).await
    }

    /// Gets player data from their username.
    pub async fn data_from_username(&self, username: &str) -> anyhow::Result<Value> {
        let url = self.provider.username_url(username);
        self.response_raw(&url).await
    }

    async fn response_raw(&self, url: &str) -> anyhow::Result<Value> {
        self.rate_limit
            .acquire()
            .await
            .expect("semaphore is never closed")
            .forget();

        if self.rate_limit.available_permits() == 0 {
            warn!(
                "rate limiting will be applied: {} requests have been sent in the past {:?} \
                 interval",
                self.provider.max_requests(),
                self.provider.interval()
            );
        }

        let response = self.req.get(url).send().await?;

        if response.status().is_success() {
            let body = response.text().await?;
            let json_object = serde_json::from_str::<Value>(&body)
                .with_context(|| format!("failed to parse json from response: {body:?}"))?;

            if let Some(error) = json_object.get("error") {
                bail!("API Error: {}", error.as_str().unwrap_or("Unknown error"));
            }
            Ok(json_object)
        } else {
            bail!("Failed to retrieve data from API");
        }
    }
}

#[cfg(test)]
#[expect(clippy::unwrap_used, clippy::print_stdout, reason = "these are tests")]
mod tests {
    use std::str::FromStr;

    use crate::{
        runtime::AsyncRuntime,
        util::mojang::{ApiProvider, MojangClient},
    };

    #[test]
    fn test_get_uuid() {
        let tasks = AsyncRuntime::new();
        let mojang = MojangClient::new(&tasks, ApiProvider::MAT_DOES_DEV);

        let uuid = tasks.block_on(mojang.get_uuid("Emerald_Explorer")).unwrap();
        let expected = uuid::Uuid::from_str("86271406-1188-44a5-8496-7af10c906204").unwrap();
        assert_eq!(uuid, expected);
    }

    #[test]
    fn test_get_username() {
        let tasks = AsyncRuntime::new();
        let mojang = MojangClient::new(&tasks, ApiProvider::MAT_DOES_DEV);

        let username = tasks
            .block_on(mojang.get_username(
                uuid::Uuid::from_str("86271406-1188-44a5-8496-7af10c906204").unwrap(),
            ))
            .unwrap();
        assert_eq!(username, "Emerald_Explorer");
    }

    #[test]
    fn test_retrieve_username() {
        let tasks = AsyncRuntime::new();
        let mojang = MojangClient::new(&tasks, ApiProvider::MAT_DOES_DEV);

        let res = tasks
            .block_on(mojang.data_from_uuid(
                &uuid::Uuid::from_str("86271406-1188-44a5-8496-7af10c906204").unwrap(),
            ))
            .unwrap();

        // {
        //   "id": "86271406118844a584967af10c906204",
        //   "name": "Emerald_Explorer",
        //   "profileActions": [],
        //   "properties": [
        //     {
        //       "name": "textures",
        //       "signature": "vSdWxKrUendEP7rapc8Kw2RP6oxWH75CaDrdLXIZlXRmM3+lIYbxaUr8feA0gtZTdiJPTA9GstQHr6mIz1Ap2gm6pd50LVj22yRA1e1qgmAEq8L6EZj7MPnN/kgvWnUj2XFdhP1TsENi3ekvDLHuvRSdeOKgdew3u6/3h6DLAZp/6w2Z89wRJRytWDrSxm3YrPJpGyUA0DjYkoKlCi2n4fd6iTxGzPCnN0gi/y1ewEGbz9rVSsN9EX+tecACl/W4PAOo2wtSEDBziHOMmAEFunmzVReo24XNTTTqQNf6wywAFbXRPaSsRayYrc1vwPXNj4mZwep1LbP8/qQsefjNi3olBmXLxnyxD62Zyx2ZK3NBD1Qbc40PiM6qhpuoQxUgPQHTxL3XazzatH4sQv11rWxLYJhppVsWxUNMy696e5JK7oVtUgSSPbqVjQYdPpn/z22ZzwXh3Y0vkbxfTZ8aZSxEYhJzUtlDNFKcaWEPzuohBsUPELISELLWmL46Rue96gR2lUxdStlUR15L4XZ3cpINTCLj1AQdl2q6mP0T7ooG/Cvri0qKtZ/RuJ3HUZMFfZB6SQ5LGbpwfwPwCWxgYkpwhIUNvLBaEQQNDXELmYgomLE1rd/q6FdM4HaSYCqxBgMyQPzkeOkrZ4k9pBaU16rRWwkCvek4Evdz2L5cpMo=",
        //       "value": "ewogICJ0aW1lc3RhbXAiIDogMTczMDY0Mjc1NjU0OCwKICAicHJvZmlsZUlkIiA6ICI4NjI3MTQwNjExODg0NGE1ODQ5NjdhZjEwYzkwNjIwNCIsCiAgInByb2ZpbGVOYW1lIiA6ICJFbWVyYWxkX0V4cGxvcmVyIiwKICAic2lnbmF0dXJlUmVxdWlyZWQiIDogdHJ1ZSwKICAidGV4dHVyZXMiIDogewogICAgIlNLSU4iIDogewogICAgICAidXJsIiA6ICJodHRwOi8vdGV4dHVyZXMubWluZWNyYWZ0Lm5ldC90ZXh0dXJlLzE1MTBlM2VlM2YwZThkNTJhMGUxZjMzY2UwYmJiZTRhZWE4Yjg4MzhjOWJkYzQ5NjEzNDI2ZWJhYjYxNGE2ODMiCiAgICB9CiAgfQp9"
        //     }
        //   ]
        // }

        // {
        //   "timestamp" : 1730642756548,
        //   "profileId" : "86271406118844a584967af10c906204",
        //   "profileName" : "Emerald_Explorer",
        //   "signatureRequired" : true,
        //   "textures" : {
        //     "SKIN" : {
        //       "url" : "http://textures.minecraft.net/texture/1510e3ee3f0e8d52a0e1f33ce0bbbe4aea8b8838c9bdc49613426ebab614a683"
        //     }
        //   }
        // }⏎

        let pretty = serde_json::to_string_pretty(&res).unwrap();
        println!("{pretty}");
    }
}
