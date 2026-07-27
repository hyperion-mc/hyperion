use flecs_ecs::macros::Component;
use heed::{Database, Env, byteorder::NativeEndian, types};
use hyperion::storage::LocalDb;
use num_traits::{FromPrimitive, ToPrimitive};

use crate::Group;

#[derive(Component)]
pub struct PermissionStorage {
    env: Env,
    perms: Database<types::U128<NativeEndian>, types::U8>,
}

impl PermissionStorage {
    pub fn new(db: &LocalDb) -> anyhow::Result<Self> {
        // We open the default unnamed database
        let perms = {
            let mut wtxn = db.write_txn()?;
            let db = db.create_database(&mut wtxn, Some("uuid-to-perms"))?;
            wtxn.commit()?;
            db
        };

        Ok(Self {
            env: (**db).clone(),
            perms,
        })
    }

    pub fn get(&self, uuid: uuid::Uuid) -> Group {
        let uuid = uuid.as_u128();
        let rtxn = self.env.read_txn().unwrap();
        let Some(perms) = self.perms.get(&rtxn, &uuid).unwrap() else {
            return Group::default();
        };

        let Some(group) = Group::from_u8(perms) else {
            tracing::error!("invalid group {perms:?}");
            return Group::default();
        };

        group
    }

    pub fn set(&self, uuid: uuid::Uuid, group: Group) -> anyhow::Result<()> {
        let uuid = uuid.as_u128();
        let mut wtxn = self.env.write_txn()?;
        self.perms.put(&mut wtxn, &uuid, &group.to_u8().unwrap())?;
        wtxn.commit()?;
        Ok(())
    }
}
