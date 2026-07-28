//! Wither Skeleton: a guided projectile and a clone you can swap places with.
//!
//! Wither Image is the reason this kit is hard: it drops a decoy carrying your
//! own username, and a second right-click swaps you with it. Used well it is
//! both a recovery and a bait; used badly it is the only way back onto the map
//! and it can be killed.
//!
//! Stats verified: 6.0 damage, 12 armour points (48%), 120% knockback taken,
//! 0.3 regen, 6000 gems. Ability numbers are `[APPROXIMATED]` -- the wiki
//! describes all three at length and gives no figures.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        ability::{self, Cast, Observable, splash_at},
        damage::MatchClock,
        effect::{self, Affliction},
        kit::{self, AbilitySpec, KitSounds, KitStats},
        projectile::{Flight, Impact, Payload, fire},
    },
    server::Cue,
};

/// How long a dropped image stands before it fades.
pub const IMAGE_SECONDS: f32 = 8.0;

/// Where this player's Wither Image is standing, and when it lapses.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Image {
    pub at: Vec3,
    pub until: f32,
}

#[derive(Component)]
pub struct WitherSkeleton;

impl Module for WitherSkeleton {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::WitherSkeleton");
        world.component::<Image>();

        kit::define(world, "Wither Skeleton", KitStats {
            melee_damage: 6.0,
            armor: 12.0,
            knockback_taken: 1.20,
            regen: 0.30,
            ..KitStats::default()
        })
        .sounds(KitSounds {
            select: "minecraft:entity.wither_skeleton.ambient",
            hurt: "minecraft:entity.wither_skeleton.hurt",
            death: "minecraft:entity.wither_skeleton.death",
        })
        .cost(6000)
        .skin(crate::kit_skin!("wither_skeleton"))
        .blurb("Leave a copy of yourself somewhere useful, then be there instead.")
        .mob("minecraft:wither_skeleton")
        .ability(AbilitySpec {
            name: "Guided Wither Skull",
            sound: "minecraft:entity.wither.shoot",
            item: "minecraft:iron_sword",
            description: "A skull with a wide blast. Hold to steer it.",
            cooldown: 7.0,
            charge_time: Some(0.6),
            proves: &[Observable::HurtsTarget, Observable::LaunchesTarget],
            activate: guided_wither_skull,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Wither Image",
            sound: "minecraft:entity.illusioner.mirror_move",
            item: "minecraft:iron_axe",
            description: "Drop a copy of yourself. Use it again to swap places with it.",
            cooldown: 10.0,
            proves: &[Observable::TeleportsCaster],
            activate: wither_image,
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Wither Swap",
            sound: "minecraft:entity.shulker.teleport",
            item: "minecraft:nether_star",
            description: "Twenty seconds of images, going off around you one after another.",
            cooldown: 20.0,
            proves: &[
                Observable::HurtsTarget,
                Observable::LaunchesTarget,
                Observable::Sustains,
            ],
            activate: wither_swap,
            ..AbilitySpec::DEFAULT
        })
        .register();
    }
}

fn guided_wither_skull(cast: &Cast<'_>) {
    fire(
        cast.world,
        cast.caster,
        Flight {
            position: cast.position.0,
            velocity: cast.facing.0.normalize_or_zero() * 8.0f32.mul_add(cast.charge, 18.0),
            gravity: 0.0,
            seconds_left: 3.0,
            // The wide blast radius the wiki calls the kit's best feature.
            radius: 1.6,
        },
        Payload::new(6.0, 1.3).then(burst),
    );
}

fn burst(impact: &Impact<'_>) {
    impact
        .world
        .get::<&crate::server::ServerHandle>(|server| server.cue(impact.at, Cue::Explosion));
}

/// First use drops the image; a second use while it stands swaps you to it.
///
/// The decoy does not fight. Making it chase and attack needs a host-side mob
/// with pathfinding, which the seam does not carry; the swap, which is the part
/// that decides recoveries, is what is modelled.
fn wither_image(cast: &Cast<'_>) {
    let now = cast.world.get::<&MatchClock>(|clock| clock.0);

    if let Some(image) = cast.caster.try_get::<&Image>(|image| *image)
        && now < image.until
    {
        cast.caster.remove(Image::id());
        cast.server.teleport(cast.player, image.at);
        cast.server.cue(image.at, Cue::Teleport);
        return;
    }

    cast.caster.set(Image {
        at: cast.position.0,
        until: now + IMAGE_SECONDS,
    });
}

/// `[APPROXIMATED]` throughout; the wiki names the ultimate and gives no
/// figures.
///
/// "Every image at once" read as one blast because there was no duration to
/// spread them over. Twenty seconds of an image going off every second and a
/// half is the kit's own trick, repeated, which is what its name says.
fn wither_swap(cast: &Cast<'_>) {
    effect::afflict(
        cast.world,
        cast.caster,
        effect::Blame::cast(cast),
        Affliction::mode(ability::ULTIMATE_SECONDS, SWAP_INTERVAL, swap_burst),
    );
}

const SWAP_INTERVAL: f32 = 1.5;

/// Per burst, and there are thirteen.
const SWAP_DAMAGE: f32 = 2.5;

fn swap_burst(cast: &Cast<'_>) {
    splash_at(cast, cast.position.0, 6.0, SWAP_DAMAGE, 1.8);
    cast.server.cue(cast.position.0, Cue::Explosion);
}
