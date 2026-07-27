use flecs_ecs::{
    core::World,
    macros::Component,
    prelude::{Entity, Module},
};

mod cached_save;
mod lifetime;
pub use cached_save::cached_save;
pub use lifetime::*;

pub trait EntityExt {
    fn minecraft_id(&self) -> i32;

    fn from_minecraft_id(id: i32) -> Self;
}

impl EntityExt for Entity {
    fn minecraft_id(&self) -> i32 {
        let raw = self.0;
        // Convert entity id into two u32s
        let most_significant = (raw >> 32) as u32;

        #[expect(
            clippy::cast_possible_truncation,
            reason = "we are getting the least significant bits, we expect truncation"
        )]
        let least_significant = raw as u32;

        // Ensure most_significant >> 4 does not overlap with least_significant
        // and that least_significant AND most_significant is 0
        // this is the "thread" space which allows for 2^6 = 64 threads
        debug_assert_eq!(
            most_significant >> 6,
            0,
            "Entity ID is too large for Minecraft"
        );

        debug_assert!(
            least_significant < (1 << 26),
            "Entity ID is too large for Minecraft (must fit in 2^26)"
        );

        // Combine them into a single i32
        let result = (most_significant << 26) | least_significant;

        #[expect(
            clippy::cast_possible_wrap,
            reason = "we do not care about sign changes, we expect wrap"
        )]
        let result = result as i32;

        result
    }

    fn from_minecraft_id(id: i32) -> Self {
        #[expect(clippy::cast_sign_loss, reason = "we do not care about sign changes.")]
        let id = id as u32;

        let least_significant = id & ((1 << 26) - 1);
        let most_significant = (id >> 26) & 0x3F;

        let raw = (u64::from(most_significant) << 32) | u64::from(least_significant);
        Self(raw)
    }
}

/// Represents application identification information used for caching and other system-level operations
#[derive(Component)]
pub struct AppId {
    /// The qualifier/category of the application (e.g. "com", "org", "hyperion")
    pub qualifier: String,
    /// The organization that created the application (e.g. "andrewgazelka")
    pub organization: String,
    /// The specific application name (e.g. "proof-of-concept")
    pub application: String,
}

#[derive(Component)]
pub struct HyperionUtilsModule;

impl Module for HyperionUtilsModule {
    fn module(world: &World) {
        world.component::<AppId>();

        world.set(AppId {
            qualifier: "github".to_string(),
            organization: "hyperion-mc".to_string(),
            application: "generic".to_string(),
        });
    }
}
