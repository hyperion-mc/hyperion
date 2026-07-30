//! What the recurring moments of the game look like.
//!
//! An ability going off, a teleport landing, a player dying, a burn ticking, a
//! poison ticking. Each is one moment that happens in a dozen places and should
//! look the same in all of them, so each is written once, here.
//!
//! This is not the old `Cue` enum under another name. A `Cue` was a closed set
//! and the *only* thing an ability could ask for, which is why Bone Explosion,
//! Water Splash and Fish Flurry were all `Cue::Explosion` and all drew the same
//! grey puff. These are plain functions returning a [`Particles`], so a kit
//! that wants bones rather than a blast composes its own and does not have to
//! widen anything:
//!
//! ```ignore
//! cast.server.particles(
//!     Particles::sphere(Particle::Item { item_stack: bone }, at, 2.0)
//!         .points(30)
//!         .speed(0.1),
//! );
//! ```
//!
//! `[INFERRED]` throughout. Mineplex's own particle choices are not in the
//! leaked source, which loaded them from the same spreadsheet as everything
//! else, so these are read off what vanilla draws for the same events.

use glam::Vec3;

use crate::server::{Argb, Particle, Particles};

/// Half-width of the box a point effect scatters its particles through.
///
/// Small enough to read as one thing happening in one place rather than as
/// weather.
const SPREAD: f32 = 0.4;

/// How fast a scattered particle drifts. Nearly still: these mark a place, and
/// a puff that flies apart stops marking it.
const DRIFT: f32 = 0.02;

/// Roughly chest height on a standing player, where a status effect reads best.
const CHEST: f32 = 0.9;

/// An ability going off here.
///
/// The default for anything with no look of its own yet, which is most of
/// them, and the one to stop reaching for as soon as a kit has something to
/// say.
pub const fn blast(at: Vec3) -> Particles {
    Particles::burst(Particle::Explosion, at)
        .count(40)
        .offset(Vec3::splat(SPREAD))
        .speed(0.5)
        // Something that just happened to a player is worth seeing from
        // further out than the client's usual particle radius.
        .long_distance(true)
}

/// Somebody arriving somewhere they were not.
///
/// A sphere rather than a puff, because a teleport has a shape: the eye reads
/// a shell as an arrival and a scatter as damage.
pub fn teleport(at: Vec3) -> Particles {
    Particles::sphere(Particle::Portal, at + Vec3::Y, 1.0)
        .points(40)
        .count(2)
        .speed(0.05)
        .long_distance(true)
}

/// Somebody pushing off nothing.
///
/// `minecraft:small_gust`, the particle vanilla throws off a wind charge going
/// off: its one picture of air being shoved somewhere. A flat disc under the
/// feet rather than a puff around the body, because the fiction of a mid-air
/// jump is that the air below took the weight, and under the feet is also
/// where it is legible to the player standing beneath deciding whether to
/// contest the landing.
pub fn updraft(at: Vec3) -> Particles {
    Particles::disc(Particle::SmallGust, at + Vec3::Y * 0.1, 0.6)
        .points(12)
        .speed(0.05)
        .long_distance(true)
}

/// A player dying here.
pub fn death(at: Vec3) -> Particles {
    Particles::burst(Particle::Cloud, at + Vec3::Y)
        .count(40)
        .offset(Vec3::new(SPREAD, 0.8, SPREAD))
        .speed(0.1)
        .long_distance(true)
}

/// One tick of something burning a player.
///
/// `minecraft:flame`, which is what vanilla draws on a burning entity. This was
/// pinned to `crit` for as long as the protocol layer could spell only five
/// particles.
pub fn burn(at: Vec3) -> Particles {
    Particles::burst(Particle::Flame, at + Vec3::Y * CHEST)
        .count(12)
        .offset(Vec3::new(0.3, 0.5, 0.3))
        .speed(DRIFT)
}

/// One tick of something poisoning a player.
///
/// `minecraft:entity_effect` in vanilla's own poison green, which is what a
/// potion effect draws. Distinct from [`burn`] by its picture and by nothing
/// else, which is the whole point: both take a point of health a second off
/// somebody standing still, and a player who cannot tell them apart cannot tell
/// a Blaze from a Spider.
pub fn venom(at: Vec3) -> Particles {
    Particles::burst(
        Particle::EntityEffect {
            // Vanilla's own poison colour, which is what `entity_effect`
            // is drawn in when a `minecraft:poison` instance renders:
            // `MobEffects.POISON` carries `0x4E9331`. Taken from the effect
            // rather than picked, so the haze around a poisoned player is the
            // green a player already reads as poison rather than an arbitrary
            // green.
            color: Argb::opaque(0x4E, 0x93, 0x31),
        },
        at + Vec3::Y * CHEST,
    )
    .count(12)
    .offset(Vec3::new(0.3, 0.5, 0.3))
    .speed(DRIFT)
}

// ---------------------------------------------------------------------------
// The kit vocabulary.
//
// Everything above is a moment that happens to *any* player: a cast, an
// arrival, a death, a tick of burning. Everything below is a moment that
// belongs to one kit or a few, and each exists because an ability that had it
// was drawing nothing at all -- `tools/smash-match.py` joins a real client and
// refuses an ability that fires and sends no `level_particles`, and it named
// twenty-three.
//
// The temptation with twenty-three holes is one `blast` in each. That would
// pass the gate and leave the game where it started, because a player who sees
// the same grey puff for a bow, a web, an ink cloud and a tidal wave has learnt
// nothing from any of them. So each of these picks the particle vanilla already
// uses for the thing being depicted, which means a player who has played
// Minecraft can read it without being taught.
//
// `[INFERRED]` throughout, on the same terms as the rest of this file.
// ---------------------------------------------------------------------------

/// A bow releasing.
///
/// `minecraft:crit`, which is what vanilla draws on a fully-drawn bow shot and
/// on a critical hit -- both being "this one was at full power". Drawn as a
/// short line out of the caster rather than a puff at their feet, because
/// three of the Skeleton's four abilities fire arrows in different patterns
/// and the direction is the only part a victim can act on.
pub fn bowshot(from: Vec3, toward: Vec3) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(Particle::Crit, eye, eye + toward.normalize_or_zero() * 2.0)
        .points(8)
        .speed(DRIFT)
        .long_distance(true)
}

/// A taut line between two points: a hook, a rope, a tether.
///
/// Grey `minecraft:dust` at a small scale, which is the closest vanilla has to
/// a drawn chain. A line and not a puff because the whole content of the
/// ability is *what it connected you to*, and a puff at one end says nothing
/// about the other.
pub fn chain(from: Vec3, to: Vec3) -> Particles {
    Particles::line(
        Particle::Dust {
            // Iron grey. The hook is iron in the fiction and in the item.
            color: Argb::opaque(0x8C, 0x8C, 0x8C),
            scale: 0.6,
        },
        from + Vec3::Y * CHEST,
        to + Vec3::Y * CHEST,
    )
    .points(24)
    .long_distance(true)
}

/// Webbing thrown over an area.
///
/// `minecraft:item_cobweb`, the particle vanilla emits from a cobweb block, so
/// the ground reads as webbed without any block being placed. A disc rather
/// than a ring: the area is a place you should not walk *through*, not a line
/// you should not cross.
pub fn web(at: Vec3, radius: f32) -> Particles {
    Particles::disc(Particle::ItemCobweb, at + Vec3::Y * 0.1, radius)
        .points(40)
        .count(2)
        .speed(DRIFT)
        .long_distance(true)
}

/// Something heavy and soft landing.
///
/// `minecraft:item_slime`, which is what vanilla throws off a slime block being
/// bounced on. Kept low and wide, because the ability is a ground slam and the
/// thing a nearby player needs to judge is its radius.
pub fn slam(at: Vec3, radius: f32) -> Particles {
    Particles::disc(Particle::ItemSlime, at + Vec3::Y * 0.1, radius)
        .points(28)
        .count(3)
        .speed(0.08)
        .long_distance(true)
}

/// Ground torn up and thrown.
///
/// `minecraft:dust_plume`, vanilla's own "a lump of the floor just left the
/// floor". Drawn at the caster's feet, which is where the block came from --
/// the block itself is already a visible flying entity, so what was missing was
/// the moment of it being ripped up.
pub fn torn_earth(at: Vec3) -> Particles {
    Particles::burst(Particle::DustPlume, at + Vec3::Y * 0.2)
        .count(24)
        .offset(Vec3::new(0.5, 0.2, 0.5))
        .speed(0.15)
        .long_distance(true)
}

/// Something large and airborne beating its wings.
///
/// `minecraft:dragon_breath`, drawn below the rider rather than around them, so
/// it reads as thrust and marks the ground they are passing over for anyone
/// underneath.
pub fn wingbeat(at: Vec3) -> Particles {
    Particles::burst(
        Particle::DragonBreath {
            // Full power. The option scales how far vanilla throws the cloud;
            // a rider's downwash should look like the dragon's own breath and
            // not a weaker copy of it.
            power: 1.0,
        },
        at - Vec3::Y * 0.3,
    )
    .count(16)
    .offset(Vec3::new(0.6, 0.2, 0.6))
    .speed(0.06)
    .long_distance(true)
}

/// Something swimming through air that is not water.
///
/// `minecraft:bubble`, which is the joke the Sky Squid kit is built on and the
/// particle a player already associates with being underwater. A trailing wake
/// behind the caster rather than a burst, because the ability is a dash and the
/// wake is what says which way it went.
pub fn wake(at: Vec3) -> Particles {
    Particles::burst(Particle::Bubble, at + Vec3::Y * CHEST)
        .count(20)
        .offset(Vec3::new(0.4, 0.4, 0.4))
        .speed(0.1)
        .long_distance(true)
}

/// A cloud of ink fired forwards.
///
/// `minecraft:squid_ink`. Laid along the firing direction rather than at the
/// muzzle, because a shotgun's threat is the cone and a victim needs to see
/// whether they are in it.
pub fn ink(from: Vec3, toward: Vec3) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(
        Particle::SquidInk,
        eye,
        eye + toward.normalize_or_zero() * 4.0,
    )
    .points(16)
    .count(4)
    .offset(Vec3::splat(0.5))
    .speed(0.2)
    .long_distance(true)
}

/// A melee lunge landing.
///
/// `minecraft:sweep_attack`, the arc vanilla draws on a sweeping sword hit.
/// Drawn as a line from where the lunge began to where it ended, so the
/// distance covered is legible -- which for the Wolf is the ability, the whole
/// kit being about closing that gap.
pub fn pounce(from: Vec3, to: Vec3) -> Particles {
    Particles::line(
        Particle::SweepAttack,
        from + Vec3::Y * CHEST,
        to + Vec3::Y * CHEST,
    )
    .points(6)
    .long_distance(true)
}

/// A mode that makes the caster dangerous, ticking.
///
/// `minecraft:angry_villager`, which is vanilla's one unambiguous "this thing
/// is furious with you". Around the head, where a status on a player reads,
/// and it repeats: an ultimate that lasts twenty seconds and draws once is a
/// twenty-second window a victim cannot see they are inside.
pub fn snarl(at: Vec3) -> Particles {
    Particles::burst(Particle::AngryVillager, at + Vec3::Y * 1.9)
        .count(4)
        .offset(Vec3::new(0.3, 0.2, 0.3))
        .speed(DRIFT)
        .long_distance(true)
}

/// Snow thrown.
///
/// `minecraft:snowflake`. The Snowman had four abilities and not one of them
/// drew anything, so this and the two below are what the kit looks like.
pub fn frost(at: Vec3) -> Particles {
    Particles::burst(Particle::Snowflake, at + Vec3::Y * CHEST)
        .count(18)
        .offset(Vec3::splat(SPREAD))
        .speed(0.1)
        .long_distance(true)
}

/// A path of ice laid across the ground.
///
/// `minecraft:item_snowball` in a line along the path. This one is not
/// decoration: Ice Path is *documented* as laying ice blocks, the seam cannot
/// write blocks, and so the ability currently consists of a small hop and
/// nothing else. The line of snow is the ability's only remaining evidence
/// that anything happened, and it is drawn along the same direction the hop
/// carries the caster.
pub fn frost_path(from: Vec3, toward: Vec3) -> Particles {
    let ground = from + Vec3::Y * 0.1;
    Particles::line(
        Particle::ItemSnowball,
        ground,
        ground + toward.normalize_or_zero() * 6.0,
    )
    .points(24)
    .count(2)
    .speed(DRIFT)
    .long_distance(true)
}

/// The edge of an aura on the ground.
///
/// A ring and not a disc, because what a player needs from an aura is exactly
/// one fact -- am I inside it -- and a ring draws the boundary that answers
/// that. A filled disc at this radius would also be a great deal of particles
/// on a two-second cooldown.
pub fn frost_ring(at: Vec3, radius: f32) -> Particles {
    Particles::ring(Particle::Snowflake, at + Vec3::Y * 0.1, radius)
        .points(48)
        .count(2)
        .speed(DRIFT)
        .long_distance(true)
}

/// Hooves tearing up ground.
///
/// `minecraft:dust_plume` again, and deliberately the same particle as
/// [`torn_earth`]: both are "the floor has been disturbed here", and giving
/// them different pictures would be inventing a distinction the game does not
/// make.
pub fn hooves(at: Vec3) -> Particles {
    Particles::burst(Particle::DustPlume, at + Vec3::Y * 0.15)
        .count(14)
        .offset(Vec3::new(0.6, 0.1, 0.6))
        .speed(0.12)
        .long_distance(true)
}

/// A spray of something white.
///
/// White `minecraft:dust`, rising, for the Cow's milk. Vanilla has no milk
/// particle -- drinking a bucket draws nothing -- so this is a colour choice
/// rather than a citation, and it is called out as such.
pub fn milk(at: Vec3, radius: f32) -> Particles {
    Particles::ring(
        Particle::Dust {
            color: Argb::opaque(0xF2, 0xF2, 0xF0),
            scale: 1.2,
        },
        at + Vec3::Y * CHEST,
        radius,
    )
    .points(32)
    .count(2)
    .speed(0.05)
    .long_distance(true)
}

/// Mushroom spores.
///
/// `minecraft:mycelium`, the particle vanilla emits from mycelium blocks, which
/// is what a mooshroom stands on. The one particle in the game that already
/// means "fungus is happening here".
pub fn spores(at: Vec3, radius: f32) -> Particles {
    Particles::sphere(Particle::Mycelium, at + Vec3::Y, radius)
        .points(40)
        .count(2)
        .speed(0.05)
        .long_distance(true)
}

/// A small fire following something through the air.
///
/// `minecraft:small_flame` and not `flame`, so the Blaze's flight is
/// distinguishable at a glance from the Blaze's burn, which [`burn`] draws in
/// full `flame`. Same kit, two effects, and a player has to be able to tell
/// which one is on them.
pub fn ember_wake(at: Vec3) -> Particles {
    Particles::burst(Particle::SmallFlame, at + Vec3::Y * 0.4)
        .count(12)
        .offset(Vec3::new(0.3, 0.3, 0.3))
        .speed(0.04)
        .long_distance(true)
}

/// Water thrown off something spinning.
///
/// `minecraft:splash`, vanilla's own thrown-water particle. The Guardian's
/// whole visual identity is water, and this is the cheap half of it.
pub fn spray(at: Vec3) -> Particles {
    Particles::burst(Particle::Splash, at + Vec3::Y * CHEST)
        .count(16)
        .offset(Vec3::splat(SPREAD))
        .speed(0.2)
        .long_distance(true)
}

/// A wall of water arriving.
///
/// A ring of `minecraft:bubble_column_up` at the wave's radius: the column
/// particle rises, so a ring of it reads as a wall standing up out of the
/// ground rather than as a puddle. The one ultimate in the roster that is
/// explicitly a *wave*, and a wave with no front is just damage.
pub const fn tide(at: Vec3, radius: f32) -> Particles {
    Particles::ring(Particle::BubbleColumnUp, at, radius)
        .points(56)
        .count(3)
        .offset(Vec3::new(0.1, 0.8, 0.1))
        .speed(0.15)
        .long_distance(true)
}

/// A small animal thrown out of the caster's hands.
///
/// `minecraft:poof`, which is what vanilla draws when a mob appears or
/// disappears. Placed at chest height in front of the caster, where the cub
/// leaves them.
pub fn tossed_mob(at: Vec3, toward: Vec3) -> Particles {
    Particles::burst(
        Particle::Poof,
        at + Vec3::Y * CHEST + toward.normalize_or_zero(),
    )
    .count(12)
    .offset(Vec3::splat(0.3))
    .speed(0.05)
    .long_distance(true)
}

/// A mouthful of something caustic thrown at somebody.
///
/// `minecraft:spit`, vanilla's llama spit, which is the one particle in the
/// game that already means "a mob has launched a body fluid at you". Laid
/// along the firing direction like [`ink`], because the ability is a fan and
/// what a victim can act on is whether they are standing in it. `NoxiousGas`
/// lost: it is a cloud that hangs, so it would belong where the bile lands
/// rather than where it leaves, and the bile that lands is the half that
/// already draws.
pub fn bile(from: Vec3, toward: Vec3) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(Particle::Spit, eye, eye + toward.normalize_or_zero() * 2.5)
        .points(10)
        .count(3)
        .offset(Vec3::splat(0.3))
        .speed(0.15)
        .long_distance(true)
}

/// Something dead reaching out for somebody.
///
/// `minecraft:soul`, which vanilla lifts off soul sand under a player wearing
/// Soul Speed: its one picture of the dead grabbing at whatever walks past.
/// [`bowshot`]'s `crit` is the closer fit to the *arrow* and lost anyway: the
/// arrow is only the delivery, the hand on the far end of it is the ability,
/// and the particle is the Skeleton's besides.
pub fn grasp(from: Vec3, toward: Vec3) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(Particle::Soul, eye, eye + toward.normalize_or_zero() * 3.0)
        .points(12)
        .speed(DRIFT)
        .long_distance(true)
}

/// Eggs leaving somebody at speed.
///
/// `minecraft:egg_crack`, what vanilla draws when a thrown egg breaks and when
/// a chicken lays one. The eggs are already flying entities, so what was
/// missing is the muzzle, on the same reasoning as [`torn_earth`]: the thing
/// nobody could see was the moment of them leaving.
pub fn egg_burst(from: Vec3, toward: Vec3) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(
        Particle::EggCrack,
        eye,
        eye + toward.normalize_or_zero() * 2.0,
    )
    .points(8)
    .count(2)
    .offset(Vec3::splat(0.25))
    .speed(0.1)
    .long_distance(true)
}

/// Feathers knocked loose by something small beating its wings hard.
///
/// White `minecraft:dust`. Vanilla draws nothing at all when a chicken flaps
/// and has no feather particle, so this is a colour choice rather than a
/// citation, called out on the same terms as [`milk`]. [`wingbeat`] lost:
/// `dragon_breath` is sized and coloured for the largest thing in the game and
/// the Chicken is the smallest.
pub fn feathers(at: Vec3) -> Particles {
    Particles::burst(
        Particle::Dust {
            color: Argb::opaque(0xFF, 0xFF, 0xFF),
            scale: 0.8,
        },
        at + Vec3::Y * CHEST,
    )
    .count(14)
    .offset(Vec3::new(0.4, 0.5, 0.4))
    .speed(0.08)
    .long_distance(true)
}

/// A body big enough that standing near it is the whole danger.
///
/// `minecraft:item_slime` again, deliberately the same particle as [`slam`]:
/// both are the Slime's own body arriving on somebody, and a second slime
/// picture would invent a distinction the kit does not make. A shell at the
/// radius that hurts and not a puff at the caster, because the only fact a
/// player needs off a Giga Slime is whether they are inside it.
pub const fn giga_body(at: Vec3, radius: f32) -> Particles {
    Particles::sphere(Particle::ItemSlime, at, radius)
        .points(48)
        .count(2)
        .speed(0.05)
        .long_distance(true)
}

/// Something catching light and staying alight.
///
/// `minecraft:flame`, full size, as a shell around the caster. [`burn`] draws
/// the same particle as a puff at the chest, and the difference in shape is
/// load-bearing: that one is a point of damage ticking on a victim, this one
/// is a Blaze announcing twenty seconds of being a hazard, and a player has to
/// be able to tell at a glance which of the two is in front of them.
pub fn pyre(at: Vec3, radius: f32) -> Particles {
    Particles::sphere(Particle::Flame, at + Vec3::Y, radius)
        .points(40)
        .count(2)
        .speed(0.05)
        .long_distance(true)
}

/// The violet a Guardian's beam reads as.
///
/// Vanilla's guardian laser is a rendered beam texture and not a particle at
/// all, so like [`milk`] this is a colour chosen rather than one cited off an
/// effect. Named once because two functions draw it, and a flare in one violet
/// with a beam in another would be two lasers.
const LASER_VIOLET: Argb = Argb::opaque(0x9A, 0x5C, 0xD0);

/// A beam singling one player out.
///
/// Drawn between the two players: the entire content of a mark is *who* it
/// landed on, and a puff at the caster answers everything except that.
pub fn mark_beam(from: Vec3, to: Vec3) -> Particles {
    Particles::line(
        Particle::Dust {
            color: LASER_VIOLET,
            scale: 1.0,
        },
        from + Vec3::Y * CHEST,
        to + Vec3::Y * CHEST,
    )
    .points(32)
    .long_distance(true)
}

/// An eye lighting up, whether or not a beam follows it.
///
/// [`mark_beam`]'s violet at [`mark_beam`]'s own origin, so the flare sits
/// exactly where the beam roots and the two read as one ability. This exists
/// because the beam needs a target and the press does not: a Guardian who
/// presses the button with nobody in range spends the ability and, without
/// this, sees nothing at all.
pub fn laser_eye(at: Vec3) -> Particles {
    Particles::burst(
        Particle::Dust {
            color: LASER_VIOLET,
            scale: 1.0,
        },
        at + Vec3::Y * CHEST,
    )
    .count(10)
    .offset(Vec3::splat(0.2))
    .speed(DRIFT)
    .long_distance(true)
}

/// A jet of fire out of somebody's hands.
///
/// Drawn as a line with a wide scatter at every sampled point rather than as a
/// cone, because `Particles` has no cone and a flamethrower is the only thing
/// in the game that wants one. What that gives up is the widening: the jet is
/// as broad at the caster's hands as it is five blocks out, so it reads as a
/// column of fire rather than a spray. The alternative was a new shape in the
/// engine's effect builder for a single call site, which is the trade being
/// refused here; if a second kit ever wants a cone, add the shape rather than a
/// second copy of this.
pub fn flamethrower(from: Vec3, toward: Vec3, reach: f32) -> Particles {
    let eye = from + Vec3::Y * CHEST;
    Particles::line(
        Particle::Flame,
        eye,
        eye + toward.normalize_or_zero() * reach,
    )
    .points(24)
    .count(2)
    .offset(Vec3::splat(0.35))
    .speed(0.05)
    .long_distance(true)
}

/// A standing copy of a player, left behind where they were.
///
/// A column of `minecraft:soul_fire_flame` about a player tall, so what is
/// drawn is the shape of the thing left rather than a puff at the place it was
/// left. Deliberately unlike [`teleport`]: planting an image and swapping onto
/// it are two presses of one button, and a player who cannot tell the two
/// apart at a glance cannot tell whether their opponent still has the escape.
/// So the plant is a figure in the Wither Skeleton's own soul fire and the
/// arrival stays the portal shell every teleport in the game uses.
pub fn effigy(at: Vec3) -> Particles {
    Particles::line(Particle::SoulFireFlame, at, at + Vec3::Y * 1.8)
        .points(14)
        .speed(DRIFT)
        .long_distance(true)
}
