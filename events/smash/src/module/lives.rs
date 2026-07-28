//! Four lives, then you spectate.
//!
//! Mineplex's `MAX_LIVES = 4` is the number the wiki and the in-game copy both
//! quote as "four lives"; the mode description's "three respawns" is the same
//! number counted differently. Losing one puts you in a four-second spectate
//! and then back on a platform; losing the last one is permanent.

use flecs_ecs::prelude::*;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        arena::Arena,
        damage::{KILL_CREDIT_WINDOW, LastHitAt, LastHitBy, MatchClock},
        kit,
        player::{self, Health, JumpsLeft, Player, Position},
        sound::{self, PlaysOnDeath},
    },
    server::{Channel, Cue, NamedColor, PlayerId, ServerHandle, Sound, SoundCategory, Text},
};

/// Mineplex's `MAX_LIVES`.
pub const MAX_LIVES: u8 = 4;

/// Seconds spent watching before you come back. Mineplex's
/// `DeathSpectateSecs`.
pub const DEATH_SPECTATE_SECS: f32 = 4.0;

/// Seconds of immunity after respawning. Mineplex's `RESPAWN_INVUL`, and it is
/// cancelled early the moment you use an item so you cannot camp under it.
pub const RESPAWN_INVULNERABLE_SECS: f32 = 1.5;

/// How loud an elimination is, against 1.0 for a sound at natural volume.
///
/// Volume is range, so this is really "how far away should somebody be and
/// still be told the match just got smaller". Everyone in the arena.
pub const ELIMINATION_VOLUME: f32 = 2.0;

#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Lives(pub u8);

impl Default for Lives {
    fn default() -> Self {
        Self(MAX_LIVES)
    }
}

/// Relationship: `(ShownAs, tier)` on a player, where the target is one of the
/// [`LifeTier`] entities this module creates.
///
/// A relation rather than a colour copied onto the player. The band a player
/// is in is a thing other subsystems want to name: `(ShownAs, last_life)` is
/// one query term away from "everybody about to go out", and an observer on
/// the edge is where a last-life effect would hang. A `Colour(NamedColor)`
/// component would answer neither question, and would have to be kept in step
/// with [`Lives`] by whoever remembered to.
///
/// Exclusive, so you are in exactly one band: re-tiering is one `add` that
/// replaces the previous edge, and the state where a player is both healthy
/// and on their last life cannot be written down.
#[derive(Component, Debug)]
pub struct ShownAs;

/// A band of remaining lives, and how a name in that band is drawn.
///
/// One entity per band rather than a match arm, so [`Tint`] hangs off
/// something a relation can point at.
#[derive(Component, Debug)]
pub struct LifeTier;

/// The colour a name in this tier is drawn in.
///
/// A plain component and not a relation, deliberately: a colour is a value,
/// not an entity in this world. Relating a tier to one of sixteen colour
/// entities would add a hop and a lookup and let nothing new be asked.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct Tint(pub NamedColor);

/// The most lives a player can have and still be in this tier.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct AtMost(pub u8);

/// The bands, narrowest first. Mineplex's own: four or more green, falling
/// through yellow and gold to red on your last, and gray once you are out.
///
/// `MAX_LIVES` is the widest band's bound rather than a literal, so raising the
/// life count cannot leave a player above every tier.
const TIERS: [(&str, u8, NamedColor); 5] = [
    ("Out", 0, NamedColor::Gray),
    ("LastLife", 1, NamedColor::Red),
    ("Low", 2, NamedColor::Gold),
    ("Wounded", 3, NamedColor::Yellow),
    ("Healthy", MAX_LIVES, NamedColor::Green),
];

/// The tier table as the world holds it, narrowest band first.
///
/// One read of the [`LifeTier`] entities, shared by the system that writes
/// `(ShownAs, tier)` and by the sidebar that needs a colour now. Both go
/// through [`band_index`], so there is one implementation of which band a
/// count falls in however many callers there are.
///
/// The sidebar reads this and not the relation, and that is not a preference.
/// `player.add((ShownAs, tier))` inside a system is a deferred command: it
/// lands at the next merge, so a same-tick reader sees the previous tick's
/// edge, and on the tick a player is created it sees no edge at all. A sidebar
/// built on that drops a player who has just joined out of the panel and out
/// of the viewer list, which is a redraw they never get. The table is the
/// fact, and the edge is the queryable cache of it.
#[derive(Debug, Default)]
pub struct Bands {
    /// Upper bounds, ascending. Sorted so [`band_index`] can take the first
    /// one wide enough.
    bounds: Vec<u8>,
    /// The tier entity and its colour, in the same order as `bounds`.
    tiers: Vec<(Entity, NamedColor)>,
}

impl Bands {
    /// Read the bands out of `world`.
    #[must_use]
    pub fn of(world: &WorldRef<'_>) -> Self {
        let mut bands = Vec::new();
        world
            .query::<(&AtMost, &Tint)>()
            .with(LifeTier::id())
            .build()
            .each_entity(|tier, (at_most, tint)| bands.push((at_most.0, tier.id(), tint.0)));
        bands.sort_unstable_by_key(|(at_most, ..)| *at_most);
        Self {
            bounds: bands.iter().map(|(at_most, ..)| *at_most).collect(),
            tiers: bands
                .into_iter()
                .map(|(_, tier, tint)| (tier, tint))
                .collect(),
        }
    }

    fn at(&self, remaining: u8) -> Option<(Entity, NamedColor)> {
        band_index(&self.bounds, remaining).and_then(|at| self.tiers.get(at).copied())
    }

    /// The tier entity `remaining` lives puts a player in.
    #[must_use]
    pub fn tier(&self, remaining: u8) -> Option<Entity> {
        self.at(remaining).map(|(tier, _)| tier)
    }

    /// The colour a name is drawn in at `remaining` lives.
    ///
    /// `None` only when the table is empty, which means [`LivesModule`] was
    /// never imported. Every module that asks declares `Lives` as a
    /// requirement and `tests/contract.rs` proves it, so a caller that has run
    /// at all may treat this as total.
    #[must_use]
    pub fn tint(&self, remaining: u8) -> Option<NamedColor> {
        self.at(remaining).map(|(_, tint)| tint)
    }
}

/// The colour `player`'s name is drawn in, read off the `(ShownAs, tier)` edge.
///
/// This is the cached answer, one tick behind whatever wrote it, and `None`
/// until the tier system has run for this player. Reach for it when the
/// question is "which band is this player in" as a fact about the world, and
/// for [`Bands::tint`] when the question is "what colour do I draw right now".
#[must_use]
pub fn tint_of(player: EntityView<'_>) -> Option<NamedColor> {
    player
        .find_target(ShownAs, |_| true)?
        .try_get::<&Tint>(|tint| tint.0)
}

/// Lives remaining as the sidebar counts them: elimination is zero however
/// many the component still says.
#[must_use]
pub fn remaining(player: EntityView<'_>, lives: Lives) -> u8 {
    if player.has(Eliminated::id()) {
        0
    } else {
        lives.0
    }
}

/// Which band `remaining` lives falls in, given the bands' upper bounds sorted
/// narrowest first. The widest band when no bound is high enough.
///
/// Split out of the system so the whole `u8` range is checkable without a
/// world. The failure worth guarding against is a band going missing and a
/// count falling through to the next colour, and no spot check notices that.
#[must_use]
pub fn band_index(bounds: &[u8], remaining: u8) -> Option<usize> {
    if bounds.is_empty() {
        return None;
    }
    Some(
        bounds
            .iter()
            .position(|at_most| remaining <= *at_most)
            .unwrap_or(bounds.len() - 1),
    )
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

/// Whether `player` is still inside their respawn immunity.
///
/// Read by the damage pipeline and by the arena's kill plane. The kill plane is
/// the one that matters most: a respawn writes the new position into the mirror
/// and asks the host to teleport, but the mirror is refilled from the host every
/// tick and the host does not move the player until the client acknowledges the
/// teleport, so for the tick or two in between the mirror still says the player
/// is under the map. Without this the respawn kills them again immediately, and
/// a player loses all four lives to one fall.
#[must_use]
pub fn is_invulnerable(player: EntityView<'_>, now: f32) -> bool {
    player.try_get::<&InvulnerableUntil>(|until| now < until.0) == Some(true)
}

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
        world.component::<LifeTier>();
        world.component::<Tint>();
        world.component::<AtMost>();
        // Exclusive is what makes "in exactly one band" a property of the
        // world rather than of the code that writes the edge: adding the new
        // tier removes the old one, with no window in between where a player
        // is in two.
        world.component::<ShownAs>().add_trait::<flecs::Exclusive>();
        for (name, at_most, tint) in TIERS {
            world
                .entity_named(name)
                .add(LifeTier::id())
                .set(AtMost(at_most))
                .set(Tint(tint));
        }
        world.component::<Eliminated>();
        world.component::<Placement>();
        world.component::<RespawnAt>();
        world.component::<InvulnerableUntil>();
        world.component::<Died>();
        world.component::<EliminatedEvent>();

        world
            .system_named::<()>("smash::assign_life_tier")
            .run(|mut it| {
                while it.next() {
                    let world = it.world();
                    let bands = Bands::of(&world);

                    world
                        .query::<&Lives>()
                        .with(Player::id())
                        .build()
                        .each_entity(|player, lives| {
                            let Some(tier) = bands.tier(remaining(player, *lives)) else {
                                return;
                            };
                            // Exclusive, so this is the whole update. Skipping
                            // the write when it would not change anything keeps
                            // flecs from moving the player between archetypes
                            // twenty times a second.
                            if player.target(ShownAs, 0).map(|t| t.id()) != Some(tier) {
                                player.add((ShownAs, tier));
                            }
                        });
                }
            });

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
                let at = sound::position_of(victim);
                // The kit's last word, where it fell. Reached through the
                // player's own `(Playing, kit)` edge, so this module still does
                // not know that kits have names.
                sound::play_kit_voice(world, victim, PlaysOnDeath, at);

                world.get::<&ServerHandle>(|server| {
                    server.cue(at, Cue::Death);
                    match killer {
                        Some(killer) => {
                            let killer_name = world.entity_from_id(killer).name();
                            server.broadcast(
                                Channel::Chat,
                                Text::text(format!("{name} was smashed by {killer_name}!")),
                            );
                        }
                        None => {
                            server.broadcast(
                                Channel::Chat,
                                Text::text(format!("{name} fell out of bounds!")),
                            );
                        }
                    }
                    server.set_spectating(*player, true);

                    if lives.0 == 0 {
                        server.send_message(
                            *player,
                            Channel::Title,
                            Text::text("GAME OVER: you ran out of lives!").color(NamedColor::Red),
                        );
                    } else {
                        server.send_message(
                            *player,
                            Channel::Title,
                            Text::text(format!("{} lives left!", lives.0)),
                        );
                    }
                });

                if lives.0 == 0 {
                    let placement = u32::try_from(remaining_alive(world)).unwrap_or(0);
                    victim.add(Eliminated::id());
                    victim.set(Placement(placement));
                    // Louder than the death that preceded it, because losing a
                    // life and losing the match are the same animation and only
                    // the sound tells the arena which one it just watched.
                    sound::play_at(
                        world,
                        at,
                        Sound::new(sound::ELIMINATION, SoundCategory::Ui)
                            .volume(ELIMINATION_VOLUME),
                    );
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

                let at = arena.spawn(*player.id());
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

/// How many players are still in the match.
#[must_use]
pub fn remaining_alive(world: WorldRef<'_>) -> usize {
    let count = world
        .query::<&Lives>()
        .without(Eliminated::id())
        .build()
        .count();
    usize::try_from(count).unwrap_or(0)
}
