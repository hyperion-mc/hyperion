use std::{fmt::Write, sync::TryLockError};

use bevy::prelude::*;
use hyperion::{
    ingress,
    net::{Compose, agnostic},
    simulation::packet::play,
};
use tracing::{debug, warn};

use crate::component::CommandRegistry;

/// Executes commands sent by the client.
///
/// This system is the reason that [`CommandRegistry`] must be locked by a mutex. Storing
/// [`bevy::ecs::system::SystemState`] as command state is common. However, to use it, a `&World`
/// is needed. However, if this system accepted `&World`, using a [`ResMut`] to
/// access the [`CommandRegistry`] would conflict with each other. The performance costs to using a
/// mutex should be negligible as the mutex should not be contested, so it should only cost a
/// an atomic `compare_exchange`.
#[expect(
    clippy::significant_drop_tightening,
    reason = "the mutex should not be contended and the lock guard lifetime cannot be tightened"
)]
fn execute_commands(
    mut packets: EventReader<'_, '_, play::CommandExecution>,
    registry: Res<'_, CommandRegistry>,
    compose: Res<'_, Compose>,
    world: &World,
) {
    let mut registry = match registry.try_lock() {
        Ok(registry) => registry,
        Err(TryLockError::WouldBlock) => {
            warn!(
                "execute_commands: CommandRegistry lock is contested - this should ideally not \
                 occur"
            );
            registry.lock().unwrap()
        }
        Err(poison) => {
            panic!("command registry lock is poisoned: {poison}");
        }
    };

    for packet in packets.read() {
        let Some(first_word) = packet.command.split_whitespace().next() else {
            warn!("command is empty");
            continue;
        };

        let Some(command) = registry.commands.get_mut(first_word) else {
            debug!("command {first_word} not found");

            let mut msg = String::new();
            write!(&mut msg, "§cAvailable commands: §r[").unwrap();

            for w in registry
                .get_permitted(world, packet.sender())
                .intersperse(", ")
            {
                write!(&mut msg, "{w}").unwrap();
            }

            write!(&mut msg, "]").unwrap();

            let chat = agnostic::chat(msg);

            compose.unicast(&chat, packet.connection_id()).unwrap();

            continue;
        };

        debug!("executing command {first_word}");

        command.executable.execute(world, packet);
    }
}

fn apply_deferred_changes(world: &mut World) {
    let mut registry = world.resource_mut::<CommandRegistry>();

    // TODO: There should be some sort of error if the apply callback tries to access the
    // CommandRegistry
    let mut commands = std::mem::take(&mut registry.get_mut().unwrap().commands);

    for (_, command) in &mut commands {
        command.executable.apply(world);
    }

    world
        .resource_mut::<CommandRegistry>()
        .get_mut()
        .unwrap()
        .commands = commands;
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the mutex should not be contended and the lock guard lifetime cannot be tightened"
)]
fn complete_commands(
    mut packets: EventReader<'_, '_, play::RequestCommandCompletions>,
    registry: Res<'_, CommandRegistry>,
    world: &World,
) {
    // TODO: This lock could be removed by separating the tab_complete callback from the execute
    // callback
    let registry = match registry.try_lock() {
        Ok(registry) => registry,
        Err(TryLockError::WouldBlock) => {
            warn!(
                "complete_commands: CommandRegistry lock is contested - this should ideally not \
                 occur"
            );
            registry.lock().unwrap()
        }
        Err(poison) => {
            panic!("command registry lock is poisoned: {poison}");
        }
    };

    for packet in packets.read() {
        // should be in form "/{command}"
        let command = packet
            .text
            .strip_prefix("/")
            .unwrap_or(&packet.text)
            .split_whitespace()
            .next()
            .unwrap_or("");

        if command.is_empty() {
            continue;
        }

        let Some(cmd) = registry.commands.get(command) else {
            continue;
        };

        (cmd.tab_complete)(world, packet);
    }
}

pub struct CommandSystemPlugin;

impl Plugin for CommandSystemPlugin {
    fn build(&self, app: &mut App) {
        // The ordering constraint between execute_command and complete_commands isn't necessary,
        // but they avoid lock contention on the CommandRegistry.
        app.add_systems(
            FixedUpdate,
            (execute_commands, apply_deferred_changes, complete_commands)
                .chain()
                .after(ingress::decode::play),
        );
    }
}
