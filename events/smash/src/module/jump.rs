//! The mid-air jump, which is vanilla creative flight with the flight taken
//! out.
//!
//! `[SOURCE]` Mineplex's `PerkDoubleJump` grants `setAllowFlight(true)`,
//! cancels the `PlayerToggleFlightEvent` the client's double tap produces, and
//! applies a velocity by hand. Nothing about that needs a modified client, and
//! nothing about it needs a new packet: the serverbound abilities packet a
//! vanilla client already sends *is* the jump key, and the only thing the
//! server has to do is refuse the flight and answer with an impulse.
//!
//! Three systems, which is the whole mechanic:
//!
//! * `arm_double_jump` keeps the client's flight permission equal to "airborne
//!   with a jump left", pushing it across the seam only when the answer
//!   changes.
//! * `spend_double_jump` answers a press: one jump off the counter, the
//!   permission rewritten so the player is not left flying, and the impulse.
//! * `restore_double_jump` puts the counter back on ground contact, which is
//!   what re-arms the whole thing.
//!
//! The counter itself is [`JumpsLeft`] and it lives in
//! [`crate::module::player`] with the rest of the state a player carries. This
//! module registers no components; it imports the modules that own the ones it
//! reads.

use flecs_ecs::prelude::*;
use glam::Vec3;

use crate::{
    flecs_ext::EntityViewExt,
    module::{
        kit::{KitModule, KitStats, Playing},
        lives::{Eliminated, LivesModule, RespawnAt},
        player::{
            Facing, JumpPressed, JumpsLeft, MayFly, OnGround, Player, PlayerModule, Position,
        },
        sound, visuals,
    },
    server::{Flight, PlayerId, ServerHandle, Sound, SoundCategory},
};

/// The lift under a *controlled* jump, in blocks per tick.
///
/// `[SOURCE]` `UtilAction.velocity` takes a `yAdd` and Mineplex's jump perks
/// pass 0.2. Only the controlled kits need it: their jump goes where the
/// player is looking, so one taken at the horizon would be purely sideways,
/// and a recovery that buys no height at all is a dodge rather than a
/// recovery.
///
/// `[INFERRED]` The same call takes a `yMax` of 1.0, and that one is
/// deliberately **not** implemented. Every uncontrolled kit in the roster
/// declares a `jump_power` of at least 0.9, so a ceiling at 1.0 would clamp
/// all twelve of them to exactly the same jump and the per-kit number would
/// stop meaning anything -- which is the shape of bug that made this mechanic
/// worth writing in the first place. Either the sheet's powers are not what
/// Mineplex handed that call, or its cap was higher; both readings leave the
/// jump powers load-bearing and the cap is what has to give.
const LIFT: f32 = 0.2;

/// How loud a mid-air jump is, against 1.0 for an ordinary ability cast.
///
/// Under one. It happens more often than anything else in the game that makes
/// a noise, and volume is range -- a client culls a sound past `16 * volume`
/// blocks -- so a crowded arena at 1.0 would be a wall of wind nobody could
/// hear a hit through.
const JUMP_VOLUME: f32 = 0.7;

/// The velocity one mid-air jump adds, for a kit and the direction its player
/// is looking.
///
/// Pure and public, so a test can compare an uncontrolled jump against a
/// controlled one directly instead of inferring the difference from where two
/// simulated players ended up.
///
/// A controlled jump goes where you look and a controlled jump taken looking
/// at the floor therefore drives you into it. That is Mineplex's behaviour and
/// it is the price of the reach: Wolf and Spider cross more ground with theirs
/// than anybody, and the same aim that buys the distance is the aim that can
/// throw it away.
#[must_use]
pub fn impulse(stats: KitStats, facing: Vec3) -> Vec3 {
    if stats.jump_control {
        facing.normalize_or_zero() * stats.jump_power + Vec3::Y * LIFT
    } else {
        Vec3::Y * stats.jump_power
    }
}

/// How many mid-air jumps `player`'s kit allows.
///
/// A player who has not picked a kit gets [`KitStats::default`]'s one. The hub
/// is a place people move around in before a match starts, and a lobby where
/// the mechanic does not work at all reads as broken rather than as not having
/// begun.
#[must_use]
pub fn allowance(player: EntityView<'_>) -> u8 {
    stats_of(player).jump_count
}

/// The stats of the kit `player` is on, or the defaults if they are on none.
fn stats_of(player: EntityView<'_>) -> KitStats {
    player
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| *stats))
        .unwrap_or_default()
}

#[derive(Component)]
pub struct JumpModule;

impl Module for JumpModule {
    fn module(world: &World) {
        world.module::<Self>("smash::Jump");

        // Behaviour only: every component below belongs to one of these three,
        // and importing them is what guarantees each is registered before a
        // system names it. See CLAUDE.md.
        world.import::<PlayerModule>();
        world.import::<KitModule>();
        world.import::<LivesModule>();

        // A dead or eliminated player is a spectator, and a spectator already
        // flies. Writing an abilities packet at one would take that away and
        // leave them stuck to the floor of a match they are watching, so both
        // halves of the mechanic step around them.
        world
            .system_named::<(&OnGround, &JumpsLeft, &mut MayFly, &PlayerId)>("arm_double_jump")
            .with(Player::id())
            .without(Eliminated::id())
            .without(RespawnAt::id())
            .each_iter(|it, _index, (ground, jumps, may_fly, player)| {
                let armed = !ground.0 && jumps.0 > 0;
                if armed == may_fly.0 {
                    return;
                }
                may_fly.0 = armed;
                it.world().get::<&ServerHandle>(|server| {
                    server.set_flight(*player, Flight::armed(armed));
                });
            });

        world
            .system_named::<(
                &mut JumpPressed,
                &OnGround,
                &Facing,
                &Position,
                &mut JumpsLeft,
                &mut MayFly,
                &PlayerId,
            )>("spend_double_jump")
            .with(Player::id())
            .without(Eliminated::id())
            .without(RespawnAt::id())
            .each_entity(
                |entity, (pressed, ground, facing, position, jumps, may_fly, player)| {
                    if !pressed.0 {
                        return;
                    }
                    // Consumed here rather than left for the mirror to
                    // overwrite next tick. The mirror does clear it, and this
                    // line is still what bounds the damage if some future host
                    // reports the flying *bit* instead of the edge: one press
                    // is one jump, whatever the thing upstream says twice.
                    pressed.0 = false;

                    let world = entity.world();
                    let stats = stats_of(entity);

                    // Standing on something spends nothing. The client should
                    // not have been able to ask -- permission is withdrawn the
                    // moment they land -- but a packet in flight across the
                    // tick they landed on arrives after it, and vanilla's own
                    // "triple jump" is exactly this hole with a looser ground
                    // check under it.
                    //
                    // A press with nothing left to spend is the same case, and
                    // both still have to be *answered*: the client is flying
                    // at this instant whether or not the game meant to let it,
                    // and the packet below is the only thing that puts it back
                    // down.
                    let spent = !ground.0 && jumps.0 > 0;
                    if spent {
                        jumps.0 -= 1;
                    }
                    may_fly.0 = !ground.0 && jumps.0 > 0;

                    let flight = Flight::armed(may_fly.0);
                    let impulse = impulse(stats, facing.0);
                    let at = position.0;
                    world.get::<&ServerHandle>(|server| {
                        server.set_flight(*player, flight);
                        if !spent {
                            return;
                        }
                        server.add_velocity(*player, impulse);
                        server.particles(visuals::updraft(at));
                        server.play_sound(
                            at,
                            Sound::new(sound::DOUBLE_JUMP, SoundCategory::Players)
                                .volume(JUMP_VOLUME),
                        );
                    });
                },
            );

        // Touching the ground is what re-arms it -- `[SOURCE]`, and the reason
        // the counter is not a cooldown. It is also why the known "triple
        // jump" exists: a ground check loose enough to be true on the way past
        // a ledge hands back a jump nobody landed for.
        world
            .system_named::<(&OnGround, &mut JumpsLeft)>("restore_double_jump")
            .with(Player::id())
            .each_entity(|entity, (ground, jumps)| {
                if ground.0 {
                    jumps.0 = allowance(entity);
                }
            });
    }
}
