//! Creeper: the glass cannon.
//!
//! Second-lowest armour in the game paid for with the fastest regen and the
//! single hardest hit a kit can land unbuffed. Everything about it is a trade.
//!
//! Stats verified against the wiki: 6.0 damage, 8 armour points (32%), 165%
//! knockback taken, 0.4 regen. Explosion's 18 damage and 1.5-second charge are
//! the wiki's numbers; Sulphur Bomb's are approximated.

use flecs_ecs::prelude::*;

use crate::{
    module::{
        ability::{Cast, Observable, splash},
        damage::DamageKind,
        kit::{self, AbilitySpec, KitSounds, KitStats},
        player::Player,
        projectile::{Flight, Impact, Payload, fire},
    },
    server::Cue,
};

/// Speed II while charged, per the wiki's Lightning Shield description.
pub const CHARGED_SPEED_AMPLIFIER: u8 = 2;

/// Set on a Creeper who has been hit by something that is not a melee attack.
/// Consumed by the next melee swing.
#[derive(Component, Debug)]
pub struct Charged;

#[derive(Component)]
pub struct Creeper;

impl Module for Creeper {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Creeper");
        world.component::<Charged>();

        kit::define(world, "Creeper", KitStats {
            melee_damage: 6.0,
            armor: 8.0,
            knockback_taken: 1.65,
            regen: 0.40,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            hurt: "minecraft:entity.creeper.hurt",
            death: "minecraft:entity.creeper.death",
        })
        .cost(4000)
        .blurb("Hits harder than anything and dies to a stiff breeze.")
        .mob("minecraft:creeper")
        .ability(AbilitySpec {
            name: "Sulphur Bomb",
            sound: "minecraft:entity.creeper.primed",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "Throw coal. It goes off on whatever it touches.",
            cooldown: 6.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: sulphur_bomb,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Explosion",
            sound: "minecraft:entity.generic.explode",
            item: "minecraft:iron_shovel",
            slot: 2,
            // The wiki: "charge up for 1.5 seconds, then explode ... achieving
            // 18 damage, the highest that Creeper can inflict".
            description: "Charge for a second and a half, then take everything nearby with you.",
            cooldown: 10.0,
            charge_time: Some(1.5),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: explosion,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Atomic Blast",
            sound: "minecraft:entity.wither.spawn",
            item: "minecraft:nether_star",
            slot: 8,
            description: "The same idea, without the restraint.",
            cooldown: 20.0,
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: atomic_blast,
            ..AbilitySpec::DEFAULT
        })
        .register();

        // Lightning Shield. The wiki is specific that it arms on a projectile
        // or a contact skill and not on a melee hit, which is why `DamageKind`
        // travels with every hit rather than being reconstructed at the far end.
        world
            .observer_named::<crate::module::damage::Damaged, ()>("smash::kits::creeper::arm")
            .with(Player::id())
            .each_iter(|it, index, ()| {
                let event = *it.param();
                let victim = it.entity(index);
                if event.kind == DamageKind::Melee || !plays_creeper(victim) {
                    return;
                }
                victim.add(Charged::id());
            });
    }
}

/// Whether this player is on the Creeper kit, without any module knowing the
/// name of a kit but this one.
fn plays_creeper(player: EntityView<'_>) -> bool {
    use crate::{
        flecs_ext::EntityViewExt,
        module::kit::{KitName, Playing},
    };
    player
        .find_target(Playing, |kit| {
            kit.try_get::<&KitName>(|name| name.0 == "Creeper") == Some(true)
        })
        .is_some()
}

/// `[APPROXIMATED]`: "good damage and knock-back in a small area".
fn sulphur_bomb(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 22.0,
            gravity: 16.0,
            seconds_left: 2.5,
            radius: 0.7,
        },
        Payload::new(5.0, 1.2).then(detonate),
    );
}

fn detonate(impact: &Impact<'_>) {
    impact
        .world
        .get::<&crate::server::ServerHandle>(|server| server.cue(impact.at, Cue::Explosion));
}

/// 18 damage at full charge. `[VERIFIED]` from the wiki; the taper down to a
/// tap is `[APPROXIMATED]`, since only the ceiling is published.
fn explosion(cast: &Cast<'_>) {
    const MAX_DAMAGE: f32 = 18.0;
    let damage = MAX_DAMAGE * cast.charge.clamp(0.25, 1.0);
    splash(cast, 5.0, damage, 2.0 * cast.charge.max(0.5));
}

/// "Massive damage and knockback in a very large area." `[APPROXIMATED]`.
fn atomic_blast(cast: &Cast<'_>) {
    splash(cast, 12.0, 22.0, 3.0);
}
