//! Finding an entity again, by whatever key the caller happens to hold.
//!
//! Three of these map an external identifier (a stream id, a UUID, an in-game
//! name) onto an [`Entity`]. The fourth, [`DeferredMap`], is the mechanism the
//! others are built on: systems run in parallel and cannot mutate a shared
//! map, so writes queue in a thread-local and land in [`DeferredMap::update`]
//! at a point in the schedule where nothing is reading.

use std::{borrow::Borrow, collections::HashMap, hash::Hash, sync::Arc};

use derive_more::{Deref, DerefMut, From};
use flecs_ecs::prelude::*;
use rustc_hash::FxHashMap;

use crate::{net::ConnectionId, simulation::Uuid, storage::ThreadLocalVec};

/// Every player entity, by the connection it arrived on.
///
/// Keyed on the whole [`ConnectionId`] rather than the bare stream id, because each proxy numbers
/// its own players from 1 and knows nothing of the others. On the bare id, proxy-0's first player
/// and proxy-1's first player are one entry: the second connect overwrites the first, the first
/// disconnect destructs the wrong entity, and the second finds nothing and panics. The proxy id is
/// the half that makes the key unique.
#[derive(Component, Default, Debug, Deref, DerefMut)]
pub struct StreamLookup {
    inner: FxHashMap<ConnectionId, Entity>,
}

#[derive(Component, Default, Debug, Deref, DerefMut)]
pub struct PlayerUuidLookup {
    /// The UUID of all players
    inner: HashMap<Uuid, Entity>,
}

#[derive(Debug)]
pub struct DeferredMap<K, V> {
    to_add: ThreadLocalVec<(K, V)>,
    to_remove: ThreadLocalVec<K>,
    map: FxHashMap<K, V>,
}

impl<K, V> Default for DeferredMap<K, V> {
    fn default() -> Self {
        Self {
            to_add: ThreadLocalVec::default(),
            to_remove: ThreadLocalVec::default(),
            map: HashMap::default(),
        }
    }
}

impl<K: Eq + Hash, V> DeferredMap<K, V> {
    pub fn insert(&self, key: K, value: V, world: &World) {
        self.to_add.push((key, value), world);
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        self.map.get(key)
    }

    pub fn remove(&self, key: K, world: &World) {
        self.to_remove.push(key, world);
    }
}

impl<K: Eq + Hash, V> DeferredMap<K, V> {
    pub fn update(&mut self) {
        for (key, value) in self.to_add.drain() {
            self.map.insert(key, value);
        }

        for key in self.to_remove.drain() {
            self.map.remove(&key);
        }
    }
}

#[derive(Component, Deref, DerefMut, From, Debug, Default)]
pub struct IgnMap(DeferredMap<Arc<str>, Entity>);
