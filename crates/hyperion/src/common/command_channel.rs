//! A multiple-producer single-consumer channel of deferred world mutations.
//!
//! Async tasks (notably the proxy reader in [`crate::net::proxy`]) run off the ECS thread and
//! cannot touch the world directly. They push a closure here instead, and [`sync_command_channel`]
//! drains the queue on the main thread at the start of every tick.

use std::sync::{Arc, Mutex};

use flecs_ecs::prelude::*;

/// A deferred mutation of the world.
pub type WorldCommand = Box<dyn FnOnce(&World) + Send>;

/// Queue of [`WorldCommand`]s to be applied on the ECS thread.
///
/// Cloning is cheap and yields another handle onto the same queue.
#[derive(Component, Clone, Default)]
pub struct CommandChannel {
    inner: Arc<Mutex<Vec<WorldCommand>>>,
}

impl CommandChannel {
    /// Queue a command to run on the ECS thread during the next tick.
    pub fn push(&self, command: impl FnOnce(&World) + Send + 'static) {
        self.inner
            .lock()
            .expect("CommandChannel mutex poisoned")
            .push(Box::new(command));
    }

    /// Run every queued command against `world`, clearing the queue.
    ///
    /// Commands pushed while this runs are left for the next call so that a command which queues
    /// another command cannot livelock the tick.
    pub fn apply(&self, world: &World) {
        let commands = {
            let mut inner = self.inner.lock().expect("CommandChannel mutex poisoned");
            std::mem::take(&mut *inner)
        };

        for command in commands {
            command(world);
        }
    }
}

/// Registers [`CommandChannel`] and the system that drains it.
pub fn register(world: &World) {
    world
        .component::<CommandChannel>()
        .add_trait::<flecs::Singleton>();
    world.set(CommandChannel::default());

    system!("sync_command_channel", world, &mut CommandChannel)
        .kind(id::<flecs::pipeline::OnLoad>())
        .each_iter(|it, _, channel| {
            let channel = channel.clone();
            channel.apply(&it.world());
        });
}
