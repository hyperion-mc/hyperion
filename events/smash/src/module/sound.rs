//! Sounds are entities, and whatever makes a noise points at one.
//!
//! The alternative is a `match` from an ability's name to a string. That is a
//! second registry: it lives in a different file from the abilities, nothing
//! makes the two agree, and the day a kit is renamed the game goes quiet in a
//! way no test notices. Here the sound is a component on the ability entity
//! itself, reached the same way everything else about an ability is reached, so
//! there is one registry and it is the world.
//!
//! Three relationships, named for the occasion rather than for the sound:
//!
//! * `(PlaysOnCast, sound)` on an ability -- what firing it sounds like.
//! * `(PlaysOnHurt, sound)` on a kit -- what the mob you are playing says when
//!   it is hit.
//! * `(PlaysOnDeath, sound)` on a kit -- and when it dies.
//!
//! All three are `Exclusive`, because an occasion has one sound and a second
//! declaration is a correction rather than an addition. Without it a kit that
//! redeclares gets both edges and [`declared`] answers with whichever was added
//! first, which is a silently stale sound and no error anywhere.
//!
//! They are also `(OnInstantiate, Inherit)`, which is a storage choice and not
//! a behavioural one. [`crate::module::kit::apply`] gives a player their own
//! ability entities with `is_a(prefab)`; flecs' default is `Override`, which
//! copies the pair onto every instance, and `Inherit` leaves the one copy on
//! the prefab and resolves through the `IsA` edge. Both read back the same, and
//! this was checked by removing the trait and watching
//! `tests/sound.rs::firing_an_ability_plays_the_sound_it_declared` stay green.
//! `Inherit` is kept because a sound is static data shared by every instance,
//! which is what the trait is for, and that test is what would catch either
//! choice being wrong.
//!
//! What is *not* relational, deliberately: the impact sound a hit makes, the
//! countdown, and the two match-boundary sounds. See [`IMPACT`].

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    module::{
        kit::Playing,
        player::{Player, Position},
    },
    server::{PlayerId, ServerHandle, Sound, SoundCategory},
};

/// The vanilla sound event this entity stands for.
#[derive(Component, Debug, Copy, Clone, PartialEq, Eq)]
pub struct SoundId(pub &'static str);

/// How it plays when nothing scales it.
#[derive(Component, Debug, Copy, Clone, PartialEq)]
pub struct Levels {
    pub category: SoundCategory,
    pub volume: f32,
    pub pitch: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            category: SoundCategory::Hostile,
            volume: 1.0,
            pitch: 1.0,
        }
    }
}

/// Relationship: `(PlaysOnCast, sound)` on an ability.
#[derive(Component, Debug)]
pub struct PlaysOnCast;

/// Relationship: `(PlaysOnHurt, sound)` on a kit.
#[derive(Component, Debug)]
pub struct PlaysOnHurt;

/// Relationship: `(PlaysOnDeath, sound)` on a kit.
#[derive(Component, Debug)]
pub struct PlaysOnDeath;

/// What a hit sounds like, before the knockback scales it.
///
/// One sound for every hit in the game, on purpose, and the only place in this
/// file that is a constant rather than a relationship. Super Smash Mobs asks a
/// player one question hundreds of times a match -- did that connect, and how
/// hard -- and [`impact`] answers it by moving the pitch and the volume of this
/// sound. A comparison is only readable against a fixed reference: give fifteen
/// kits fifteen different impact timbres and the pitch shift stops meaning
/// "harder" and starts meaning "a different kit hit me". The kit's own voice is
/// still heard, on the *victim*, through `(PlaysOnHurt, sound)`.
pub const IMPACT: &str = "minecraft:entity.player.attack.strong";

/// A projectile connecting, at the point on its path where it did.
///
/// Separate from [`IMPACT`], which plays on the victim: this one says *where*
/// a shot crossed somebody, which for a fast projectile is not where either
/// end of that tick's step was. It is the only sound in the game positioned
/// somewhere other than at a player.
pub const PROJECTILE_HIT: &str = "minecraft:entity.arrow.hit";

/// The hitmarker, played to the shooter alone.
///
/// Vanilla's own answer to "did that land", and the reason it is a unicast at
/// the shooter's ears rather than a positioned sound is that a Barrage arrow
/// can connect forty blocks away, where a positioned sound is inaudible and the
/// shooter learns nothing.
pub const RANGED_HITMARKER: &str = "minecraft:entity.arrow.hit_player";

/// One second closer to the start of the match.
pub const COUNTDOWN_TICK: &str = "minecraft:block.note_block.hat";

/// How many of the last seconds of a countdown are ticked out loud.
///
/// Not the whole countdown, which is sixty seconds at the minimum player count:
/// a tick a second for a minute is a metronome nobody asked for. The last few
/// are the ones a player is actually counting.
pub const COUNTDOWN_AUDIBLE_SECONDS: u8 = 5;

/// How much the pitch climbs with each second closer to the start.
pub const COUNTDOWN_PITCH_STEP: f32 = 0.2;

/// One countdown tick, rising as the wait runs out.
///
/// `seconds_left` counts down to one, and the pitch climbs with each tick so
/// the sequence itself says how close the start is without a player having to
/// read the number. Inside the client's `0.5..=2.0` by construction: five
/// seconds out is 1.0 and one second out is 1.8.
#[must_use]
pub fn countdown_tick(seconds_left: f32) -> Sound {
    let window = f32::from(COUNTDOWN_AUDIBLE_SECONDS);
    let steps = (window - seconds_left).clamp(0.0, window);
    Sound::new(COUNTDOWN_TICK, SoundCategory::Ui).pitch(COUNTDOWN_PITCH_STEP.mul_add(steps, 1.0))
}

/// Go.
pub const MATCH_START: &str = "minecraft:entity.ender_dragon.growl";

/// Results are up.
pub const MATCH_END: &str = "minecraft:ui.toast.challenge_complete";

/// Somebody is out of lives and out of the match.
///
/// Positioned where they were standing rather than played to everyone: the
/// arena wants to know *where* it just lost a player, and the beacon's falling
/// tone is the one vanilla sound that reads as something switching off.
pub const ELIMINATION: &str = "minecraft:block.beacon.deactivate";

/// Knockback magnitude, in blocks per tick, that counts as a full smash.
///
/// Read off the model rather than guessed: [`crate::module::knockback::resolve`]
/// gives `0.2 + 0.48 * strength` horizontally, and a strength of about 2.5 is
/// where the vertical cap stops rising and a hit becomes the sideways launch
/// that ends a life. That is roughly 1.4 blocks a tick of total impulse, so a
/// hit at or above this is one a spectator should hear across the arena.
pub const SMASH_IMPULSE: f32 = 1.4;

/// The lightest hit that still makes a noise, in blocks per tick.
///
/// Below this the hit did essentially nothing -- a zero-knockback ability tick,
/// a victim standing exactly on a splash centre -- and a sound for it is noise
/// that makes the real hits harder to hear.
pub const QUIET_IMPULSE: f32 = 0.05;

/// Pitch of the lightest audible hit, and of the hardest.
///
/// Down, not up, as the hit gets harder. A bigger, heavier collision resonates
/// lower, so a falling pitch is the direction a player already reads as "that
/// was more". Both ends sit inside the client's `0.5..=2.0` clamp with room to
/// spare, because a value the client silently flattens is a value that stops
/// carrying information at exactly the moment the hit matters most.
pub const JAB_PITCH: f32 = 1.5;
pub const SMASH_PITCH: f32 = 0.7;

/// Volume of the lightest audible hit, and of the hardest.
///
/// Up as the hit gets harder, which does two things at once. It is louder, and
/// because a client culls a sound past `16 * volume` blocks it also carries
/// further: a jab is a local event and a full smash is heard by the whole
/// arena, which is the information a spectator wants and the reason volume is
/// scaled as well as pitch rather than instead of it.
pub const JAB_VOLUME: f32 = 0.55;
pub const SMASH_VOLUME: f32 = 1.6;

/// The sound one hit makes, given the impulse it launched the victim with.
///
/// `None` for a hit that moved nobody. The interpolation is linear in impulse
/// magnitude and saturates at [`SMASH_IMPULSE`], so every hit above a full
/// smash sounds the same rather than continuing to deepen into the clamp.
#[must_use]
pub fn impact(impulse: Vec3) -> Option<Sound> {
    let magnitude = impulse.length();
    if !magnitude.is_finite() || magnitude < QUIET_IMPULSE {
        return None;
    }
    let t = (magnitude / SMASH_IMPULSE).clamp(0.0, 1.0);
    Some(Sound {
        id: IMPACT,
        category: SoundCategory::Players,
        volume: (SMASH_VOLUME - JAB_VOLUME).mul_add(t, JAB_VOLUME),
        pitch: (SMASH_PITCH - JAB_PITCH).mul_add(t, JAB_PITCH),
    })
}

/// The entity standing for `id` played at `levels`, created the first time it
/// is asked for.
///
/// Interning rather than a fresh entity per declaration, so the graph has one
/// node per sound and "which abilities play this" is a query. Kits declare
/// themselves once at startup, so the linear scan costs nothing that matters
/// and buys not having to keep a side table in step with the world.
///
/// Keyed on the levels as well as the id, which is the part that is easy to get
/// wrong. Keying on the id alone means the second caller to ask for the same
/// sound at a different volume silently gets the first caller's volume back,
/// and the symptom is a kit that is quieter than it declared with nothing
/// anywhere saying so.
#[must_use]
pub fn intern<'w>(world: &'w World, id: &'static str, levels: Levels) -> EntityView<'w> {
    if let Some(found) = lookup(world, id, levels) {
        return found;
    }
    world.entity().set(SoundId(id)).set(levels)
}

/// The entity for `id` at `levels`, if one has been interned.
#[must_use]
pub fn lookup<'w>(world: &'w World, id: &str, levels: Levels) -> Option<EntityView<'w>> {
    let mut found: Option<Entity> = None;
    world
        .query::<(&SoundId, &Levels)>()
        .build()
        .each_entity(|entity, (sound, existing)| {
            if found.is_none() && sound.0 == id && *existing == levels {
                found = Some(entity.id());
            }
        });
    found.map(|id| world.entity_from_id(id))
}

/// The sound `subject` declares for `occasion`, if it declares one.
///
/// `target` and not `each_target`, because `ecs_get_target` is the one that
/// follows an `IsA` edge to the prefab a player's ability instance was made
/// from. See this module's own documentation for why that matters.
#[must_use]
pub fn declared(subject: EntityView<'_>, occasion: impl IntoEntity) -> Option<Sound> {
    let sound = subject.target(occasion, 0)?;
    let id = sound.try_get::<&SoundId>(|s| s.0)?;
    let levels = sound.try_get::<&Levels>(|l| *l).unwrap_or_default();
    Some(Sound {
        id,
        category: levels.category,
        volume: levels.volume,
        pitch: levels.pitch,
    })
}

/// The kit prefab a player is playing, which is where their voice lives.
#[must_use]
pub fn kit_of(player: EntityView<'_>) -> Option<EntityView<'_>> {
    player.target(Playing, 0)
}

/// Play whatever `subject` declares for `occasion`, at `at`. A subject that
/// declares nothing makes no noise, which is the honest outcome and is what
/// `tests/sound.rs` enumerates against.
pub fn play_declared(
    world: WorldRef<'_>,
    subject: EntityView<'_>,
    occasion: impl IntoEntity,
    at: Vec3,
) {
    let Some(sound) = declared(subject, occasion) else {
        return;
    };
    world.get::<&ServerHandle>(|server| server.play_sound(at, sound));
}

/// Play `sound` for everyone in the match, at their own ears.
///
/// A per-player unicast rather than one positioned broadcast: a countdown or a
/// result is about the match and not about a place, so it should be the same
/// loudness for the player standing on the far platform.
pub fn play_to_everyone(world: WorldRef<'_>, sound: Sound) {
    let mut listeners = Vec::new();
    world
        .query::<&PlayerId>()
        .with(Player::id())
        .build()
        .each(|id| listeners.push(*id));
    world.get::<&ServerHandle>(|server| {
        for listener in listeners {
            server.play_sound_to(listener, sound);
        }
    });
}

/// Where a player is, for a sound that belongs to something happening to them.
#[must_use]
pub fn position_of(player: EntityView<'_>) -> Vec3 {
    player.try_get::<&Position>(|p| p.0).unwrap_or(Vec3::ZERO)
}

/// Play whatever the kit `player` is on declares for `occasion`, at `at`.
///
/// The subject is the kit prefab and it is reached through the player's own
/// `(Playing, kit)` edge, so the damage and death paths never learn that kits
/// have names, let alone what they are.
pub fn play_kit_voice(
    world: WorldRef<'_>,
    player: EntityView<'_>,
    occasion: impl IntoEntity,
    at: Vec3,
) {
    let Some(kit) = kit_of(player) else {
        return;
    };
    play_declared(world, kit, occasion, at);
}

/// Play `sound` at `at`, for everyone close enough to hear it.
pub fn play_at(world: WorldRef<'_>, at: Vec3, sound: Sound) {
    world.get::<&ServerHandle>(|server| server.play_sound(at, sound));
}

#[derive(Component)]
pub struct SoundModule;

impl Module for SoundModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Sound");

        world.component::<SoundId>();
        world.component::<Levels>();

        // See this module's own documentation for what each of these two buys
        // and, for one of them, what it does not.
        for relationship in [
            world.component::<PlaysOnCast>().id(),
            world.component::<PlaysOnHurt>().id(),
            world.component::<PlaysOnDeath>().id(),
        ] {
            world
                .entity_from_id(relationship)
                .add(flecs::Exclusive)
                .add((flecs::OnInstantiate, flecs::Inherit));
        }
    }
}
