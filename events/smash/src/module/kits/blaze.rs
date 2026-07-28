//! Blaze: chip damage that armour does not stop, and a charge that can be
//! interrupted.
//!
//! The only kit that deals fire damage, which is the point: Inferno bypasses
//! armour, so Iron Golem's 64% reduction means nothing against it.
//!
//! Stats verified: 6.0 damage, 12 armour points (48%), 150% knockback taken.
//! Regen is the one number where the kit table and the kit page disagree --
//! the table says 0.25, the page says 0.15 -- and the page is used here.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::module::{
    ability::{Cast, Observable, splash_at},
    damage::{DamageKind, Damaged, hurt},
    kit::{self, AbilitySpec, KitSounds, KitStats},
    knockback::Knockback,
    player::{Health, Player, Position},
};

/// `[VERIFIED]`: "it can be cancelled by taking 4 or more damage" while
/// charging Firefly.
pub const FIREFLY_CANCEL_DAMAGE: f32 = 4.0;

#[derive(Component)]
pub struct Blaze;

impl Module for Blaze {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Blaze");

        kit::define(world, "Blaze", KitStats {
            melee_damage: 6.0,
            armor: 12.0,
            knockback_taken: 1.50,
            regen: 0.15,
            // "Blaze also has permanent Speed I and a high jump height, making
            // it one of the most mobile mobs in the game."
            jump_power: 1.1,
            energy: Some((100.0, 20.0)),
            ..KitStats::default()
        })
        .sounds(KitSounds {
            hurt: "minecraft:entity.blaze.hurt",
            death: "minecraft:entity.blaze.death",
        })
        .cost(8000)
        .skin(crate::kit_skin!("blaze"))
        .blurb("Set people on fire. Armour will not save them.")
        .mob("minecraft:blaze")
        .ability(AbilitySpec {
            name: "Inferno",
            sound: "minecraft:entity.blaze.shoot",
            item: "minecraft:iron_sword",
            description: "Spew flame. No knockback, and armour does not reduce it.",
            cooldown: 0.5,
            energy_cost: Some(12.0),
            proves: &[Observable::HurtsTarget],
            activate: inferno,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Firefly",
            sound: "minecraft:entity.blaze.ambient",
            item: "minecraft:iron_axe",
            description: "Charge a second and a half, then ram. Four damage while charging \
                          cancels it.",
            cooldown: 9.0,
            charge_time: Some(1.5),
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: firefly,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Phoenix",
            sound: "minecraft:item.firecharge.use",
            item: "minecraft:nether_star",
            description: "Twenty seconds of Firefly with no charge and free flight.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
            ],
            activate: phoenix,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

/// A short cone of fire. `DamageKind::Environment` is what makes it ignore
/// armour, which is the whole reason the kit exists.
///
/// `[APPROXIMATED]` damage and range: the wiki says only "low damage ... keeping
/// enemies in the Inferno will rack up a lot of damage, regardless of armor".
fn inferno(cast: &Cast<'_>) {
    const REACH: f32 = 5.0;
    const DAMAGE: f32 = 1.5;

    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * (REACH * 0.5);
    let caster = cast.caster.id();
    let mut victims = Vec::new();
    cast.world
        .query::<(&Position, &Health)>()
        .with(Player::id())
        .build()
        .each_entity(|entity, (position, health)| {
            if entity.id() != caster && !health.is_dead() && position.0.distance(ahead) <= REACH {
                victims.push(entity.id());
            }
        });

    for victim in victims {
        hurt(cast.world.entity_from_id(victim), Damaged {
            attacker: Some(caster),
            amount: DAMAGE,
            // No knockback at all, per the wiki.
            knockback: Knockback::from(cast.position.0).times(0.0),
            kind: DamageKind::Environment,
        });
    }
}

/// `[APPROXIMATED]` damage; the 1.5-second charge is `[VERIFIED]`.
fn firefly(cast: &Cast<'_>) {
    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * 3.0;
    cast.server.add_velocity(
        cast.player,
        cast.facing.0.normalize_or_zero() * (2.2 * cast.charge.max(0.4)),
    );
    splash_at(cast, ahead, 3.0, 9.0 * cast.charge.max(0.4), 1.8);
}

/// `[APPROXIMATED]`: free flight for twenty seconds is a movement mode the game
/// half has no way to enter, so the damage pass is what is modelled.
fn phoenix(cast: &Cast<'_>) {
    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * 3.0;
    cast.server.add_velocity(
        cast.player,
        cast.facing.0.normalize_or_zero() * 2.4 + Vec3::Y * 0.4,
    );
    splash_at(cast, ahead, 4.0, 7.0, 1.4);
}
