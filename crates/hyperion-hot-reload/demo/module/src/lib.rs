//! A minimal game module used to exercise the reload gate end to end.
//!
//! Cargo features stand in for editing the source between builds: `tuned` changes only a
//! system body, `health-f32` changes `Health`'s layout, and `migration` adds the
//! migration that makes that change acceptable.

use flecs_ecs::prelude::*;

/// Survives every reload untouched, so its values prove world state was preserved rather
/// than rebuilt.
#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
pub struct Score {
    pub points: i32,
}

#[cfg(not(feature = "health-f32"))]
mod health {
    /// Whole hit points.
    pub type Hp = u32;
    /// The code-only change: what one tick of regeneration is worth.
    #[cfg(not(feature = "tuned"))]
    pub const REGEN_PER_TICK: Hp = 1;
    #[cfg(feature = "tuned")]
    pub const REGEN_PER_TICK: Hp = 5;
}

#[cfg(feature = "health-f32")]
mod health {
    /// Fractional hit points.
    pub type Hp = f32;
    #[cfg(not(feature = "tuned"))]
    pub const REGEN_PER_TICK: Hp = 1.0;
    #[cfg(feature = "tuned")]
    pub const REGEN_PER_TICK: Hp = 5.0;
}

#[derive(Component, Debug, Default, Clone, Copy)]
#[flecs(meta)]
pub struct Health {
    pub hp: health::Hp,
}

#[derive(Component)]
pub struct ArenaModule;

impl Module for ArenaModule {
    fn module(world: &World) {
        world.component::<Health>().meta();
        world.component::<Score>().meta();

        world
            .system_named::<&mut Health>("regen")
            .each(|h| h.hp += health::REGEN_PER_TICK);
    }
}

#[cfg(feature = "migration")]
fn migrations() -> Vec<hyperion_hot_reload::Migration> {
    vec![hyperion_hot_reload::migration! {
        component: Health,
        from: { hp: u32 },
        with: |old| Health { hp: old.hp as f32 },
    }]
}

#[cfg(not(feature = "migration"))]
const fn migrations() -> Vec<hyperion_hot_reload::Migration> {
    Vec::new()
}

hyperion_hot_reload::export_module! {
    name: "arena",
    register: |world| {
        world.import::<ArenaModule>();
    },
    migrations: migrations(),
}
