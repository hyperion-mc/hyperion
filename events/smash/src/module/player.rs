//! Player state that every other module reads.
//!
//! Position, rotation and ground state are *mirrors*: the adapter writes them
//! from the host server once per tick and nothing in the game writes them back.
//! Keeping them as components rather than trait calls is what lets the arena
//! bounds check and the cooldown tick stay pure component iteration.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{flecs_ext::EntityViewExt, server::PlayerId};

/// Tag: this entity is a player taking part in the game.
#[derive(Component, Debug, Default)]
pub struct Player;

/// Mirror of the host's position. Written by the adapter, read by the game.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Position(pub Vec3);

/// Mirror of the host's velocity.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq)]
pub struct Velocity(pub Vec3);

/// Unit vector the player is looking along. Abilities that fire "where you look"
/// read this and nothing else.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Facing(pub Vec3);

impl Default for Facing {
    fn default() -> Self {
        Self(Vec3::NEG_Z)
    }
}

/// Mirror of the host's ground state. Gates double jump and the abilities that
/// Mineplex required you to be grounded for (Fissure, Seismic Slam).
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct OnGround(pub bool);

/// Health in half-hearts out of [`Health::max`].
///
/// This is the quantity that drives knockback: in Mineplex SSM a low health bar
/// means you fly further, which is the inverse of Smash Bros' accumulating
/// damage percentage but produces the same feel.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Health {
    pub current: f32,
    pub max: f32,
}

impl Health {
    #[must_use]
    pub const fn full(max: f32) -> Self {
        Self { current: max, max }
    }

    /// 1.0 at full health, 0.0 at death. The single input the knockback model
    /// takes from the victim's condition.
    #[must_use]
    pub fn fraction(self) -> f32 {
        if self.max <= 0.0 {
            return 0.0;
        }
        (self.current / self.max).clamp(0.0, 1.0)
    }

    #[must_use]
    pub fn is_dead(self) -> bool {
        self.current <= 0.0
    }

    pub fn damage(&mut self, amount: f32) {
        self.current = (self.current - amount).max(0.0);
    }

    pub fn heal(&mut self, amount: f32) {
        self.current = (self.current + amount).min(self.max);
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::full(20.0)
    }
}

/// A charge or ammo pool. Only kits that have one carry it, which is why it is
/// a separate component rather than a field on [`Health`].
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Energy {
    pub current: f32,
    pub max: f32,
    /// Units per second.
    pub regen: f32,
}

impl Energy {
    #[must_use]
    pub const fn full(max: f32, regen: f32) -> Self {
        Self {
            current: max,
            max,
            regen,
        }
    }

    /// Spend `amount` if it is there. Returns whether it was.
    pub fn try_spend(&mut self, amount: f32) -> bool {
        if self.current + f32::EPSILON < amount {
            return false;
        }
        self.current -= amount;
        true
    }
}

/// How many double jumps remain before touching ground again.
///
/// How many a player starts with is the kit's, not a constant: the Chicken has
/// eight and everybody else has one. [`crate::module::jump`] is what spends
/// them and what puts them back.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct JumpsLeft(pub u8);

/// Mirror of the host having this client in flight.
///
/// A vanilla client turns this on by double-tapping the jump key with flight
/// permitted, which sends a serverbound abilities packet, and that is the only
/// mid-air jump input a client without a mod can produce. So
/// [`crate::module::jump`] reads a true here as a *press*.
///
/// Reading a level as an edge is sound only because of what the game does with
/// it. Every press is answered by writing the flight state back across the
/// seam, which clears the host's flying bit in the same tick, so the next
/// mirror read is false and one double tap is one jump. Nothing in this game
/// ever leaves anybody flying, and `smash::Jump::spend_double_jump` refusing a
/// press from somebody standing on the ground is the other half of that.
///
/// An `is_flying && !was_flying_last_tick` edge computed in the mirror is the
/// tempting alternative and it does not work: hyperion decodes packets in
/// `OnUpdate` and copies `is_flying` into `MovementTracking::last_tick_flying`
/// in `PreStore`, both of them after the mirror has run in `OnLoad`, so by the
/// next mirror read the two are already equal and the edge is gone. Watched:
/// a real client's double tap produced no impulse at all.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct Flying(pub bool);

/// What the client was last told about flight, so the arming system sends a
/// packet when the answer changes rather than twenty times a second.
///
/// The game's own memory of a write it made across the seam. It is not a
/// mirror: nothing reads it back off the host, because [`crate::server`]
/// deliberately has no reads. Both halves of that are load-bearing -- a
/// clientbound abilities packet per airborne player per tick is a fifth of the
/// game's whole packet budget spent saying nothing changed.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct MayFly(pub bool);

/// Mirror of which of the nine hotbar slots the player is holding.
///
/// A mirror in the same sense as [`Position`]: the host owns it, the adapter
/// copies it in once a tick, and the game reads it as a plain component. What
/// reads it is the experience bar, which has to answer "how far along is the
/// ability in the slot you are holding" for every player twenty times a
/// second, and a call across the seam per player per tick is exactly the cost
/// the mirrors in this module exist to avoid.
///
/// It is deliberately not what *fires* an ability. A right-click is a packet
/// and the slot it means is the one the host knows at the instant the packet
/// arrives, which is the same tick a client may have changed it in; the input
/// layer therefore reads the host directly and this stays a display input.
#[derive(Component, Debug, Default, Copy, Clone, PartialEq, Eq)]
pub struct SelectedSlot(pub u8);

/// Registers the mirrored components and the systems that maintain them.
#[derive(Component)]
pub struct PlayerModule;

impl Module for PlayerModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Player");

        // Final: a player is a leaf, not a base to inherit from. `is_a(Player)`
        // is now a CONSTRAINT_VIOLATED abort rather than a quiet mistake.
        world.component::<Player>().add_trait::<flecs::Final>();
        world.component::<Position>();
        world.component::<Velocity>();
        world.component::<Facing>();
        world.component::<OnGround>();
        world.component::<Health>();
        world.component::<Energy>();
        world.component::<JumpsLeft>();
        world.component::<Flying>();
        world.component::<MayFly>();
        world.component::<SelectedSlot>();

        // A player is never meaningfully without these, and forgetting one is
        // the kind of bug that shows up as a silently non-matching query three
        // modules away. `With` makes flecs add them for us.
        world
            .component::<Player>()
            .add_trait::<(flecs::With, Position)>()
            .add_trait::<(flecs::With, Velocity)>()
            .add_trait::<(flecs::With, Facing)>()
            .add_trait::<(flecs::With, OnGround)>()
            .add_trait::<(flecs::With, Health)>()
            .add_trait::<(flecs::With, JumpsLeft)>()
            .add_trait::<(flecs::With, Flying)>()
            .add_trait::<(flecs::With, MayFly)>()
            .add_trait::<(flecs::With, SelectedSlot)>();

        world
            .system_named::<&mut Energy>("regen_energy")
            .each_iter(|it, _, energy| {
                energy.current =
                    (energy.regen.mul_add(it.delta_time(), energy.current)).min(energy.max);
            });
    }
}

/// Send a game event to one player.
///
/// Every event in the game is *about a player*, and flecs matches an observer
/// only when the emitted id matches one of the observer's terms — not when the
/// observer's terms merely happen to be present on the entity. Tagging every
/// event with [`Player`] makes that one rule.
///
/// The consequence for anyone writing an observer, in this crate or in a kit
/// module outside it: **name `Player` as a term**.
///
/// ```ignore
/// world
///     .observer_named::<Damaged, &Health>("my_kit::retaliate")
///     .with(Player::id())     // <- without this the observer never fires
///     .each_iter(|it, _, health| { .. });
/// ```
///
/// Leaving it off is silent rather than loud, which is the one sharp edge in
/// this design; `docs/smash-design.md` records why the alternatives were worse.
pub fn notify<E: ComponentId>(player: EntityView<'_>, event: &E) {
    player.emit_about::<Player, _>(event);
}

/// Convenience for tests and for the adapter: make a player entity.
#[must_use]
pub fn spawn_player<'w>(world: &'w World, id: PlayerId, name: &str) -> EntityView<'w> {
    world.entity_named(name).set(id).add(Player::id())
}
