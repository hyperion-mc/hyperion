//! Constructs for connecting and working with a `Heed` database.

use std::path::Path;

use byteorder::NativeEndian;
use derive_more::Deref;
use flecs_ecs::macros::Component;
use heed::{Database, Env, EnvOpenOptions, types};
use uuid::Uuid;

use crate::simulation::skin::{ArchivedPlayerSkin, PlayerSkin};

/// A wrapper around a `Heed` database
#[derive(Component, Debug, Clone, Deref)]
pub struct LocalDb {
    env: Env,
}

impl LocalDb {
    /// Creates a new [`LocalDb`]
    pub fn new() -> anyhow::Result<Self> {
        let path = Path::new("db").join("heed.mdb");

        std::fs::create_dir_all(&path)?;

        let env = unsafe {
            EnvOpenOptions::new()
                .map_size(10 * 1024 * 1024) // 10MB
                .max_dbs(8) // todo: why is this needed/configurable? ideally would be infinite...
                .open(&path)?
        };

        Ok(Self { env })
    }
}

/// A handler for player skin operations
#[derive(Component, Debug, Clone)]
pub struct SkinHandler {
    env: Env,
    skins: Database<types::U128<NativeEndian>, types::Bytes>,
}

impl SkinHandler {
    /// Creates a new [`SkinHandler`] from a given [`LocalDb`].
    pub fn new(db: &LocalDb) -> anyhow::Result<Self> {
        // We open the default unnamed database
        let skins = {
            let mut wtxn = db.write_txn()?;
            let db = db.create_database(&mut wtxn, Some("uuid-to-skins"))?;
            wtxn.commit()?;
            db
        };

        Ok(Self {
            env: db.env.clone(),
            skins,
        })
    }

    /// Finds a [`PlayerSkin`] by its UUID.
    pub fn find(&self, uuid: Uuid) -> anyhow::Result<Option<PlayerSkin>> {
        // We open a read transaction to check if those values are now available

        let uuid = uuid.as_u128();

        let rtxn = self.env.read_txn()?;
        let skin = self.skins.get(&rtxn, &uuid);

        let Some(skin) = skin? else {
            return Ok(None);
        };

        let skin = unsafe { rkyv::access_unchecked::<ArchivedPlayerSkin>(skin) };
        let skin = rkyv::deserialize::<_, rkyv::rancor::Error>(skin).unwrap();
        Ok(Some(skin))
    }

    /// Inserts a [`PlayerSkin`] into the database.
    pub fn insert(&self, uuid: Uuid, skin: &PlayerSkin) -> anyhow::Result<()> {
        let uuid = uuid.as_u128();

        let mut wtxn = self.env.write_txn()?;

        let skin = rkyv::to_bytes::<rkyv::rancor::Error>(skin).unwrap();

        self.skins.put(&mut wtxn, &uuid, &skin)?;
        wtxn.commit()?;

        Ok(())
    }
}
