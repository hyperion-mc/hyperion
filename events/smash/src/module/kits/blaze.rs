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

use crate::{
    flecs_ext::WorldRefExt,
    module::{
        ability::{self, Cast, Observable, splash_at},
        damage::{DamageKind, Damaged, hurt},
        effect::{self, Affliction, Shows},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        knockback::Knockback,
        player::{Health, Player, Position},
    },
    server::Cue,
};

/// `[VERIFIED]`: "it can be cancelled by taking 4 or more damage" while
/// charging Firefly.
pub const FIREFLY_CANCEL_DAMAGE: f32 = 4.0;

/// How long Inferno leaves somebody burning, and what the burn costs them.
///
/// `[APPROXIMATED]`. The wiki gives Inferno no figures at all -- only "low
/// damage ... keeping enemies in the Inferno will rack up a lot of damage,
/// regardless of armor" -- so these are vanilla's own fire: one point a second,
/// for the four seconds a `minecraft:fire_charge` sets. Tuned to the wiki's
/// description rather than to a number, which is why they are marked.
pub const BURN_SECONDS: f32 = 4.0;
pub const BURN_PER_SECOND: f32 = 1.0;
pub const BURN_INTERVAL: f32 = 1.0;

/// What a burning player looks and sounds like, once, so Inferno and Phoenix
/// cannot drift into looking like two different things.
const BURNING: Shows = Shows {
    cue: Cue::Burn,
    sound: "minecraft:entity.player.hurt_on_fire",
};

/// Set `victim` alight. Re-applying refreshes rather than stacks, which
/// [`effect::afflict`] arranges through the ability's own identity.
fn ignite(cast: &Cast<'_>, victim: EntityView<'_>) {
    effect::afflict(
        cast.world,
        victim,
        effect::Blame::cast(cast),
        Affliction::over_time(
            BURN_SECONDS,
            BURN_PER_SECOND,
            BURN_INTERVAL,
            // The reason the kit exists: Iron Golem's 64% reduction means
            // nothing against it, and the burn has to inherit that or the
            // lingering half of the ability quietly does not.
            DamageKind::Environment,
            BURNING,
        ),
    );
}

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
            select: "minecraft:entity.blaze.ambient",
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
            description: "Spew flame. They keep burning, and armour stops none of it.",
            cooldown: 0.5,
            energy_cost: Some(12.0),
            proves: &[Observable::HurtsTarget, Observable::AfflictsTarget],
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
            description: "Twenty seconds of Firefly with no charge and free flight. Everything \
                          you touch burns.",
            cooldown: 1.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::LaunchesCaster,
                Observable::AfflictsTarget,
                Observable::Sustains,
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
        let victim = cast.world.entity_from_id(victim);
        hurt(victim, Damaged {
            attacker: Some(caster),
            amount: DAMAGE,
            // No knockback at all, per the wiki.
            knockback: Knockback::from(cast.position.0).times(0.0),
            kind: DamageKind::Environment,
        });
        // The lingering half. "Keeping enemies in the Inferno will rack up a
        // lot of damage" is the wiki describing a burn that outlives the cone,
        // not a bigger number on the cone itself.
        ignite(cast, victim);
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

/// `[WIKI]` "twenty seconds of Firefly with no charge and free flight".
///
/// The twenty seconds is the ability. Free flight is not a movement mode the
/// seam can enter, so each beat pushes the Blaze along its own look direction,
/// which is what free flight looks like from the outside: a Blaze that keeps
/// going where it points until the crystal runs out.
fn phoenix(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, PHOENIX_INTERVAL, phoenix_pass),
    );
}

/// `[APPROXIMATED]`. Firefly's own charge is 1.5 s and Phoenix removes it, so a
/// pass a second is "Firefly with no charge" at about the rate a player could
/// press it.
const PHOENIX_INTERVAL: f32 = 1.0;

/// One uncharged Firefly, and the fire it leaves behind.
fn phoenix_pass(cast: &Cast<'_>) {
    let ahead = cast.position.0 + cast.facing.0.normalize_or_zero() * 3.0;
    cast.server.add_velocity(
        cast.player,
        cast.facing.0.normalize_or_zero() * 2.4 + Vec3::Y * 0.4,
    );
    for victim in splash_at(cast, ahead, 4.0, 7.0, 1.4) {
        ignite(cast, cast.world.entity_at(victim));
    }
}
