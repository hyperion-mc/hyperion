//! Applying damage, and remembering who to credit for it.
//!
//! Damage and knockback are split across two events on purpose. [`Damaged`]
//! lowers health; only once that has happened is [`Smashed`] emitted, so the
//! knockback of a hit is computed against the health that hit left behind. If
//! both lived on one event the two modules would have to agree on observer
//! registration order, which is exactly the kind of implicit coupling that
//! makes a module list stop being reorderable.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        knockback::{Knockback, Smashed},
        player::{self, Health, Player, Position},
    },
    server::{Cue, PlayerId, ServerHandle},
};

/// Where a hit came from. Several kits' passives key off this — Creeper's
/// Lightning Shield only arms against non-melee, Guardian's Thorns only reduces
/// projectiles — so it travels with every hit rather than being reconstructed.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DamageKind {
    Melee,
    Projectile,
    Ability,
    /// Hunger, lava, the map itself. Ignores armour, as Mineplex's hunger
    /// damage did.
    Environment,
}

impl DamageKind {
    /// Whether armour applies. Mineplex made hunger true damage precisely so
    /// that high-armour kits could not outlast everyone in a stalled game.
    #[must_use]
    pub const fn is_reduced_by_armor(self) -> bool {
        !matches!(self, Self::Environment)
    }
}

/// Armour in vanilla armour points.
///
/// Reduction is `points * 4%`, capped at 80%, which is vanilla Minecraft's
/// formula and reproduces the wiki's own pairings exactly: Skeleton's 12 points
/// are listed as 48% reduction, Iron Golem's 16 as 64%.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Armor(pub f32);

impl Default for Armor {
    fn default() -> Self {
        Self(0.0)
    }
}

impl Armor {
    const MAX_REDUCTION: f32 = 0.8;
    const PER_POINT: f32 = 0.04;

    #[must_use]
    pub fn reduction(self) -> f32 {
        (self.0 * Self::PER_POINT).clamp(0.0, Self::MAX_REDUCTION)
    }

    #[must_use]
    pub fn apply(self, damage: f32) -> f32 {
        damage * (1.0 - self.reduction())
    }
}

/// Relationship: `(LastHitBy, attacker)` on the victim.
///
/// Exclusive, because "who gets the kill" has exactly one answer. Storing it as
/// a relationship rather than an `Option<Entity>` field means flecs cleans the
/// edge up when the attacker entity is destroyed, so a disconnect mid-fight
/// cannot leave a dangling attacker id behind.
#[derive(Component, Debug)]
pub struct LastHitBy;

/// When the last hit landed, in game seconds. Kill credit expires.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct LastHitAt(pub f32);

/// Seconds after which a void death stops being someone's kill.
pub const KILL_CREDIT_WINDOW: f32 = 10.0;

/// Mineplex made every kit starve at the same rate by making hunger true
/// damage: half a heart, ignoring armour, once the food bar empties.
pub const STARVE_DAMAGE: f32 = 1.0;

/// Hurt someone. The only supported way to deal damage.
///
/// A function rather than a bare `emit` because a flecs payload event has to
/// name the component its observers query on, and no caller should have to know
/// that [`Health`] is that component.
pub fn hurt(victim: EntityView<'_>, event: Damaged) {
    player::notify(victim, &event);
}

/// Emitted at a victim to hurt them. Prefer [`hurt`].
#[derive(Component, Debug, Copy, Clone)]
pub struct Damaged {
    pub attacker: Option<Entity>,
    /// Before armour.
    pub amount: f32,
    pub knockback: Knockback,
    pub kind: DamageKind,
}

/// Wall-clock of the match, ticked by the lobby. Damage needs it only to stamp
/// kill credit.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct MatchClock(pub f32);

#[derive(Component)]
pub struct DamageModule;

impl Module for DamageModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Damage");

        world.component::<Armor>();
        world.component::<Damaged>();
        // This module is what makes armour mean anything, so this module is
        // what says every player has some.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Armor)>();
        world.component::<LastHitAt>();
        world.component::<LastHitBy>().add(flecs::Exclusive);
        world
            .component::<MatchClock>()
            .add_trait::<flecs::Singleton>();
        world.set(MatchClock::default());

        // Health is written here and read by the knockback observer this one
        // emits into. flecs tracks that at runtime and panics on the overlap,
        // so the write is scoped to a block and `Smashed` goes out only once
        // the borrow is gone. The query therefore does not name `Health` at
        // all.
        world
            .observer_named::<Damaged, (&Armor, &Position, &PlayerId)>("smash::apply_damage")
            .with(Player::id())
            .each_iter(|it, index, (armor, position, player)| {
                let event = *it.param();
                let victim = it.entity(index);
                let world = it.world();

                let applied = if event.kind.is_reduced_by_armor() {
                    armor.apply(event.amount)
                } else {
                    event.amount
                };

                let (current, max) = victim.get::<&mut Health>(|health| {
                    health.damage(applied);
                    (health.current, health.max)
                });

                if let Some(attacker) = event.attacker
                    && attacker != victim.id()
                {
                    let clock = world.cloned::<&MatchClock>();
                    victim.add((LastHitBy, attacker)).set(LastHitAt(clock.0));
                }

                world.get::<&ServerHandle>(|server| {
                    server.set_health(*player, current, max);
                    server.cue(position.0, Cue::Hurt);
                });

                player::notify(victim, &Smashed {
                    attacker: event.attacker,
                    knockback: event.knockback,
                    damage: applied,
                });
            });
    }
}
