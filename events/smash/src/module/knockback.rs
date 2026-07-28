//! Knockback: the only thing in Super Smash Mobs that actually kills you.
//!
//! Damage never kills in the common case. Damage lowers your health bar, a low
//! health bar makes every subsequent hit launch you further, and eventually a
//! hit launches you off the map. That inversion — health *down* meaning
//! knockback *up* — is Mineplex's restatement of Smash Bros' rising damage
//! percentage, and it is the one formula the whole game balances around.
//!
//! The numbers below are transcribed from Mineplex's own Java, not fitted:
//! `SuperSmash.java` supplies the health term and `DamageManager.java` plus
//! `UtilAction.velocity` supply the rest. See `docs/smash-design.md` for the
//! citations and for the two constants that are genuinely ours.
//!
//! Everything here is in Minecraft's native velocity unit, blocks per tick.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        player::{Health, OnGround, Player, Position},
        sound,
    },
    server::{PlayerId, ServerHandle},
};

/// The constants of Mineplex's knockback pipeline, as data.
///
/// A singleton so it is tunable without recompiling the kits, and so the pure
/// functions below can be driven directly from a test with no world at all.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct KnockbackModel {
    /// `log10` of the damage is the base, floored at this much damage. Below
    /// the floor every hit knocks back the same, which is what stops a
    /// one-damage tick from being a free reposition.
    pub min_damage: f32,
    /// Added to the health multiplier per point of missing health. Mineplex
    /// used 0.1, so a player on 1 of 20 HP is launched 2.9x as far as one at
    /// full health by an otherwise identical hit.
    pub health_scale_per_hp: f32,
    /// Scales the trajectory vector before its length is read back out.
    pub trajectory_scale: f32,
    /// Floor on horizontal speed, applied before the strength term.
    pub speed_base: f32,
    /// Horizontal speed gained per unit of scaled trajectory length.
    pub speed_per_length: f32,
    /// Vertical impulse per unit of strength, before the cap.
    pub vertical_per_strength: f32,
    /// The vertical cap rises far more slowly than the vertical impulse does,
    /// so past a strength of about 2.5 a hit is essentially all horizontal.
    /// This is why Super Smash Mobs kills you sideways off an edge rather than
    /// straight up.
    pub vertical_cap_base: f32,
    pub vertical_cap_per_strength: f32,
    /// Added after the cap when the victim is standing on something, so a hit
    /// that connects on the ground still pops them into the air where the next
    /// hit can send them off.
    pub ground_boost: f32,
}

impl Default for KnockbackModel {
    /// Mineplex's values.
    fn default() -> Self {
        Self {
            min_damage: 2.0,
            health_scale_per_hp: 0.1,
            trajectory_scale: 0.6,
            speed_base: 0.2,
            speed_per_length: 0.8,
            vertical_per_strength: 0.2,
            vertical_cap_base: 0.4,
            vertical_cap_per_strength: 0.04,
            ground_boost: 0.2,
        }
    }
}

/// The victim's kit's knockback-taken percentage, as a multiplier.
///
/// One factor in the same multiplicative product as everything else, so 150%
/// here and a 2.0x ability multiplier compose to 3.0x. Kits mutate this in
/// place — Magma Cube's Fuel the Fire lowers it per kill and resets on death —
/// which is why it is a component on the victim rather than a constant read off
/// the kit.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct KnockbackTaken(pub f32);

impl Default for KnockbackTaken {
    fn default() -> Self {
        Self(1.0)
    }
}

/// What one ability or swing contributes, before the victim is considered.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Knockback {
    /// Where the hit came from. The launch direction is *away* from this, in
    /// the horizontal plane only: an attacker hovering above a victim still
    /// launches them sideways rather than into the floor.
    pub origin: Vec3,
    /// The ability's own multiplier. Melee is 1.0; Bone Explosion shipped at
    /// 2.5 and was later cut to 2.0.
    pub multiplier: f32,
}

impl Knockback {
    #[must_use]
    pub const fn from(origin: Vec3) -> Self {
        Self {
            origin,
            multiplier: 1.0,
        }
    }

    #[must_use]
    pub const fn times(mut self, multiplier: f32) -> Self {
        self.multiplier = multiplier;
        self
    }
}

/// The scalar every other term folds into, Mineplex's `knockback` local.
///
/// `log10(max(damage, floor)) * (1 + 0.1 * missing_health) * taken * ability`.
#[must_use]
pub fn strength(
    model: KnockbackModel,
    damage: f32,
    health: Health,
    taken: KnockbackTaken,
    ability_multiplier: f32,
) -> f32 {
    let base = damage.max(model.min_damage).log10();
    let missing = (health.max - health.current).max(0.0);
    let health_term = model.health_scale_per_hp.mul_add(missing, 1.0);
    base * health_term * taken.0.max(0.0) * ability_multiplier
}

/// Turn a strength into the velocity to add to the victim.
///
/// Mineplex scaled a unit trajectory by `0.6 * strength`, read its length back
/// out to get `0.2 + 0.8 * length`, then bolted the vertical component on
/// afterwards. The round trip through a vector length is redundant — it is just
/// `0.2 + 0.48 * strength` — but it is kept factored the same way so the
/// constants stay recognisable against the original.
#[must_use]
pub fn resolve(
    model: KnockbackModel,
    strength: f32,
    origin: Vec3,
    victim: Vec3,
    grounded: bool,
) -> Vec3 {
    let away = Vec3::new(victim.x - origin.x, 0.0, victim.z - origin.z).normalize_or_zero();
    if away == Vec3::ZERO {
        return Vec3::ZERO;
    }
    // `speed_base` and `ground_boost` are floors under a launch, not a launch of
    // their own. Without this a hit that asked for no knockback at all still
    // moved the victim 0.2 blocks a tick sideways and 0.2 up, which is most of
    // a jump: Blaze's Inferno is documented as having none and was quietly
    // repositioning everyone it touched.
    if strength <= 0.0 {
        return Vec3::ZERO;
    }

    let trajectory_length = model.trajectory_scale * strength;
    let speed = model
        .speed_per_length
        .mul_add(trajectory_length, model.speed_base);

    let cap = model
        .vertical_cap_per_strength
        .mul_add(strength, model.vertical_cap_base);
    let mut vertical = (model.vertical_per_strength * strength).min(cap);
    if grounded {
        vertical += model.ground_boost;
    }

    away * speed + Vec3::Y * vertical
}

/// Vanilla Minecraft knockback, for the comparison the design doc makes and for
/// a test that pins the difference.
///
/// Vanilla is flat: fixed horizontal and vertical impulses plus a bonus per
/// level of the Knockback enchantment, and it never looks at the victim. That
/// last property is the one Super Smash Mobs had to discard.
#[must_use]
pub fn vanilla(origin: Vec3, victim: Vec3, knockback_levels: u32) -> Vec3 {
    const HORIZONTAL: f32 = 0.4;
    const VERTICAL: f32 = 0.4;
    const PER_LEVEL: f32 = 0.5;

    let away = Vec3::new(victim.x - origin.x, 0.0, victim.z - origin.z).normalize_or_zero();
    away * PER_LEVEL.mul_add(knockback_levels as f32, HORIZONTAL) + Vec3::Y * VERTICAL
}

/// Emitted at a victim once the damage has already landed, so the knockback
/// sees the health that hit left them on.
#[derive(Component, Debug, Copy, Clone)]
pub struct Smashed {
    pub attacker: Option<Entity>,
    pub knockback: Knockback,
    /// Post-armour damage, which is what the base term is taken from.
    pub damage: f32,
}

#[derive(Component)]
pub struct KnockbackModule;

impl Module for KnockbackModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Knockback");

        world.component::<Knockback>();
        world.component::<KnockbackTaken>();
        world
            .component::<Player>()
            .add_trait::<(flecs::With, KnockbackTaken)>();
        world.component::<Smashed>();
        world
            .component::<KnockbackModel>()
            .add_trait::<flecs::Singleton>();
        world.set(KnockbackModel::default());

        world
            .observer_named::<Smashed, (&Health, &KnockbackTaken, &Position, &OnGround, &PlayerId)>(
                "smash::apply_knockback",
            )
            .with(Player::id())
            .each_iter(|it, _index, (health, taken, position, ground, player)| {
                let event = *it.param();

                it.world()
                    .get::<(&KnockbackModel, &ServerHandle)>(|(model, server)| {
                        let k = strength(
                            *model,
                            event.damage,
                            *health,
                            *taken,
                            event.knockback.multiplier,
                        );
                        let impulse =
                            resolve(*model, k, event.knockback.origin, position.0, ground.0);
                        server.add_velocity(*player, impulse);
                        // The one number in the game that says how hard a hit
                        // was is the one it launches you with, and it is
                        // already computed here. A jab and a full smash are the
                        // same sound at different pitch and volume, so what a
                        // player hears tracks what the physics did rather than
                        // which button produced it.
                        if let Some(sound) = sound::impact(impulse) {
                            server.play_sound(position.0, sound);
                        }
                    });
            });
    }
}
