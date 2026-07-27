//! Four lives, then you spectate.
//!
//! Mineplex's `MAX_LIVES = 4` is the number the wiki and the in-game copy both
//! quote as "four lives"; the mode description's "three respawns" is the same
//! number counted differently. Losing one puts you in a four-second spectate
//! and then back on a platform; losing the last one is permanent.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        arena::Arena,
        damage::{KILL_CREDIT_WINDOW, LastHitAt, LastHitBy, MatchClock},
        kit,
        player::{self, Health, JumpsLeft, Player, Position},
    },
    server::{Channel, Cue, PlayerId, ServerHandle},
};

/// Mineplex's `MAX_LIVES`.
pub const MAX_LIVES: u8 = 4;

/// Seconds spent watching before you come back. Mineplex's
/// `DeathSpectateSecs`.
pub const DEATH_SPECTATE_SECS: f32 = 4.0;

/// Seconds of immunity after respawning. Mineplex's `RESPAWN_INVUL`, and it is
/// cancelled early the moment you use an item so you cannot camp under it.
pub const RESPAWN_INVULNERABLE_SECS: f32 = 1.5;

#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Lives(pub u8);

impl Default for Lives {
    fn default() -> Self {
        Self(MAX_LIVES)
    }
}

impl Lives {
    /// The colour Mineplex showed remaining lives in. Four or more green,
    /// falling through yellow and gold to red on your last.
    #[must_use]
    pub const fn colour(self) -> &'static str {
        match self.0 {
            0 => "gray",
            1 => "red",
            2 => "gold",
            3 => "yellow",
            _ => "green",
        }
    }
}

/// Out of lives. Permanent for the rest of the match.
#[derive(Component, Debug)]
pub struct Eliminated;

/// Finishing position, assigned in reverse elimination order.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Placement(pub u32);

/// Match clock time at which a dead player comes back.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct RespawnAt(pub f32);

/// Match clock time until which a respawned player cannot be hurt.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct InvulnerableUntil(pub f32);

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DeathCause {
    Void,
    Damage,
}

/// Kill a player. Named so callers do not have to know which component the
/// death observers query on.
pub fn kill(victim: EntityView<'_>, cause: DeathCause) {
    player::notify(victim, &Died { cause });
}

/// Emitted at a player who has just died. Prefer [`kill`].
#[derive(Component, Debug, Copy, Clone)]
pub struct Died {
    pub cause: DeathCause,
}

/// Emitted at a player who has just lost their last life.
#[derive(Component, Debug, Copy, Clone)]
pub struct EliminatedEvent {
    pub placement: u32,
}

/// Who, if anyone, should be credited for a death.
///
/// Void deaths are attributed to the game, so the credit has to come from the
/// combat log instead: whoever hit the victim last, if they did it recently
/// enough. Mineplex kept a full combat log with assists; this keeps only the
/// last hit, which is the part that decides the kill.
#[must_use]
pub fn killer_of(victim: EntityView<'_>, now: f32) -> Option<Entity> {
    let at = victim.try_get::<&LastHitAt>(|a| a.0)?;
    if now - at > KILL_CREDIT_WINDOW {
        return None;
    }
    victim.target(LastHitBy, 0).map(|e| e.id())
}

#[derive(Component)]
pub struct LivesModule;

impl Module for LivesModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Lives");

        world.component::<Lives>();
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Lives)>();
        world.component::<Eliminated>();
        world.component::<Placement>();
        world.component::<RespawnAt>();
        world.component::<InvulnerableUntil>();
        world.component::<Died>();
        world.component::<EliminatedEvent>();

        world
            .observer_named::<Died, (&mut Lives, &PlayerId, &mut Health)>("smash::on_death")
            .with(Player::id())
            .each_iter(|it, index, (lives, player, health)| {
                let victim = it.entity(index);
                if victim.has(Eliminated::id()) {
                    return;
                }
                let world = it.world();
                let clock = world.cloned::<&MatchClock>().0;

                lives.0 = lives.0.saturating_sub(1);
                // Zero the health so the void system does not re-fire on the
                // same corpse before the respawn lands.
                health.current = 0.0;

                let name = victim.name();
                let killer = killer_of(victim, clock);

                world.get::<&ServerHandle>(|server| {
                    server.cue(Vec3::ZERO, Cue::Death);
                    match killer {
                        Some(killer) => {
                            let killer_name = world.entity_from_id(killer).name();
                            server.broadcast(
                                Channel::Chat,
                                &format!("{name} was smashed by {killer_name}!"),
                            );
                        }
                        None => {
                            server.broadcast(Channel::Chat, &format!("{name} fell out of bounds!"));
                        }
                    }
                    server.set_spectating(*player, true);

                    if lives.0 == 0 {
                        server.send_message(
                            *player,
                            Channel::Title,
                            "GAME OVER — you ran out of lives!",
                        );
                    } else {
                        server.send_message(
                            *player,
                            Channel::Title,
                            &format!("{} lives left!", lives.0),
                        );
                    }
                });

                if lives.0 == 0 {
                    let placement = remaining_alive(world) as u32;
                    victim.add(Eliminated::id());
                    victim.set(Placement(placement));
                    player::notify(victim, &EliminatedEvent { placement });
                } else {
                    victim.set(RespawnAt(clock + DEATH_SPECTATE_SECS));
                }
            });

        world
            .system_named::<(&RespawnAt, &mut Health, &mut Position, &PlayerId, &Arena)>(
                "smash::respawn",
            )
            .each_entity(|player, (respawn, health, position, id, arena)| {
                let world = player.world();
                let clock = world.cloned::<&MatchClock>().0;
                if clock < respawn.0 {
                    return;
                }

                let at = arena.spawn(*player.id() as usize);
                health.current = health.max;
                position.0 = at;
                player.remove(RespawnAt::id());
                player.set(InvulnerableUntil(clock + RESPAWN_INVULNERABLE_SECS));
                player.set(JumpsLeft(1));

                let hotbar = kit::hotbar(player);
                world.get::<&ServerHandle>(|server| {
                    server.teleport(*id, at);
                    server.set_health(*id, health.current, health.max);
                    server.set_spectating(*id, false);
                    server.set_hotbar(*id, &hotbar);
                });
            });
    }
}

use glam::Vec3;

/// How many players are still in the match.
#[must_use]
pub fn remaining_alive(world: WorldRef<'_>) -> usize {
    world
        .query::<&Lives>()
        .without(Eliminated::id())
        .build()
        .count() as usize
}
