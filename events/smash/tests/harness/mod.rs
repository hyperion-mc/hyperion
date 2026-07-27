//! A whole game in a test, with no Minecraft server anywhere near it.

// Each test binary compiles its own copy and uses a different part of it.
#![allow(dead_code)]

use std::sync::Arc;

use flecs_ecs::prelude::*;
use glam::Vec3;
use smash::{
    SmashModule,
    module::player::{Facing, OnGround, Player, Position},
    server::{PlayerId, ServerHandle, mock::MockServer},
};

pub struct Game {
    pub world: World,
    pub server: Arc<MockServer>,
    next_id: u64,
}

impl Game {
    pub fn new() -> Self {
        let world = World::new();
        let server = Arc::new(MockServer::new());
        // Import before set: the workspace enables flecs_manual_registration,
        // so ServerHandle must be registered -- which the module does -- before
        // anything uses it.
        world.import::<SmashModule>();
        world.set(ServerHandle(server.clone()));
        Self {
            world,
            server,
            next_id: 1,
        }
    }

    /// A player standing at `at`, facing +X, on the ground.
    pub fn player(&mut self, name: &str, at: Vec3) -> Entity {
        let id = PlayerId(self.next_id);
        self.next_id += 1;
        self.world
            .entity_named(name)
            .set(id)
            .add(Player::id())
            .set(Position(at))
            .set(Facing(Vec3::X))
            .set(OnGround(true))
            .id()
    }

    /// Advance the simulation by `seconds`, in `steps` equal ticks.
    pub fn advance(&self, seconds: f32, steps: u32) {
        let dt = seconds / steps as f32;
        for _ in 0..steps {
            self.world.progress_time(dt);
        }
    }
}
