//! The mid-air jump: what a press costs, what it buys, and when it buys
//! nothing.
//!
//! The input is a serverbound abilities packet, which only a real client
//! sends, so what these drive is `Flying`, the component `src/mirror.rs` turns
//! that packet into. Two links are therefore outside them: the mirror's copy
//! and the adapter's write back onto hyperion's own `Flight`. Both are held by
//! `Match.prove_double_jump` in `tools/smash-match.py`, and that is not a
//! formality -- the first version of the mirror could never fire and every
//! assertion in this file passed anyway.
//!
//! Everything downstream of the press is here, and on the `MockServer` call
//! log rather than on the world, because what matters is not that the counter
//! moved but that an impulse reached a client.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::{Game, TICK};
use smash::{
    module::{
        damage::MatchClock,
        jump,
        kit::{self, KitStats},
        lives::{DeathCause, RespawnAt, kill},
        player::{Flying, JumpsLeft, MayFly, OnGround, Position},
    },
    server::{Flight, PlayerId, mock::Call},
};

/// A player in mid-air, holding whatever their kit allows them.
///
/// A grounded tick first, because `JumpsLeft` defaults to zero and landing is
/// what fills it: somebody who has never touched the floor of this world has
/// nothing to spend, which would make every assertion below pass for the wrong
/// reason.
fn airborne(game: &Game, player: Entity) {
    let player = game.world.entity_from_id(player);
    player.set(OnGround(true));
    game.world.progress_time(TICK);
    player.set(OnGround(false));
    game.world.progress_time(TICK);
}

/// One press of the jump key, as the mirror reports one.
///
/// Set, ticked, and cleared, because that is what the mirror does with it: the
/// game answers a press by clearing the host's flying bit, so the next mirror
/// read is false again. Leaving it set would simulate a host that never got
/// the answer, and the run would spend a jump every tick.
fn press(game: &Game, player: Entity) {
    let player = game.world.entity_from_id(player);
    player.set(Flying(true));
    game.world.progress_time(TICK);
    player.set(Flying(false));
}

/// Every impulse the game asked the host to add to `player`.
fn impulses(game: &Game, player: PlayerId) -> Vec<Vec3> {
    game.server
        .calls()
        .iter()
        .filter_map(|call| match call {
            Call::AddVelocity(id, delta) if *id == player => Some(*delta),
            _ => None,
        })
        .collect()
}

/// The whole mechanic in one run: airborne, press, launched, and one poorer.
#[test]
fn a_mid_air_press_spends_one_jump_and_launches_the_player() {
    let mut game = Game::new();
    let player = game.player("jumper", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();

    airborne(&game, player);
    assert_eq!(
        game.world.entity_from_id(player).cloned::<&JumpsLeft>().0,
        1,
        "leaving the ground should not cost a jump"
    );

    game.server.take();
    press(&game, player);

    assert_eq!(
        game.world.entity_from_id(player).cloned::<&JumpsLeft>().0,
        0,
        "the press should have spent the one jump"
    );
    let impulses = impulses(&game, id);
    assert_eq!(
        impulses.len(),
        1,
        "one press is one impulse, got {impulses:?}"
    );
    assert!(
        impulses[0].y > 0.0,
        "a double jump has to go up; got {:?}",
        impulses[0]
    );
}

/// A press that arrives while the player is standing on something buys
/// nothing.
///
/// The client should never be able to send one -- permission is withdrawn on
/// landing -- but a packet already in flight arrives after the landing tick,
/// and the known Mineplex "triple jump" is that hole with a looser ground
/// check under it.
#[test]
fn a_grounded_press_spends_nothing() {
    let mut game = Game::new();
    let player = game.player("grounded", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();

    game.world.progress_time(TICK);
    game.server.take();
    press(&game, player);

    assert_eq!(
        game.world.entity_from_id(player).cloned::<&JumpsLeft>().0,
        1,
        "a press on the ground should leave the counter alone"
    );
    assert!(
        impulses(&game, id).is_empty(),
        "a press on the ground launched the player anyway"
    );
}

/// A player who has already used their jump gets no second one.
///
/// They are still answered across the seam, because whatever the game intended
/// the client is flying at that instant and something has to put it down.
#[test]
fn a_player_out_of_jumps_gets_nothing() {
    let mut game = Game::new();
    let player = game.player("spent", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();

    airborne(&game, player);
    press(&game, player);
    game.server.take();

    // A second press, with the counter already at zero.
    press(&game, player);

    assert!(
        impulses(&game, id).is_empty(),
        "a player with no jumps left was launched anyway"
    );
    assert_eq!(
        game.server.flight_of(id),
        vec![Flight::Disarmed],
        "the client was left flying with nothing to spend"
    );
}

/// Permission tracks "airborne with a jump left", and is pushed on change
/// rather than every tick.
#[test]
fn flight_is_armed_on_leaving_the_ground_and_disarmed_once_spent() {
    let mut game = Game::new();
    let player = game.player("armer", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();

    // Grounded and holding a jump: nothing to say yet.
    game.world.progress_time(TICK);
    assert!(
        game.server.flight_of(id).is_empty(),
        "a player standing on the floor was told about flight"
    );

    airborne(&game, player);
    // Several ticks in the air with the answer unchanged.
    game.advance(TICK * 5.0, 5);
    assert_eq!(
        game.server.flight_of(id),
        vec![Flight::Armed],
        "the arming state should be pushed once, not once a tick"
    );

    press(&game, player);
    assert_eq!(
        game.server.flight_of(id),
        vec![Flight::Armed, Flight::Disarmed],
        "spending the last jump should take the permission back"
    );

    game.advance(TICK * 5.0, 5);
    assert_eq!(
        game.server.flight_of(id).len(),
        2,
        "an unchanged answer should not be resent"
    );
}

/// Landing puts the jump back, which is what re-arms the whole mechanic.
#[test]
fn landing_restores_the_jump_and_re_arms_the_client() {
    let mut game = Game::new();
    let player = game.player("lander", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
    let view = game.world.entity_from_id(player);

    airborne(&game, player);
    press(&game, player);
    assert_eq!(view.cloned::<&JumpsLeft>().0, 0);

    view.set(OnGround(true));
    game.world.progress_time(TICK);
    assert_eq!(
        view.cloned::<&JumpsLeft>().0,
        1,
        "touching the ground is what re-arms it"
    );

    game.server.take();
    view.set(OnGround(false));
    game.world.progress_time(TICK);
    assert_eq!(
        game.server.flight_of(id),
        vec![Flight::Armed],
        "leaving the ground again should hand the permission back"
    );
}

/// The Chicken's eight, which is why the count is a kit stat.
///
/// `[VERIFIED]` on the wiki: "Chicken is the only mob who can double jump eight
/// times." A count that lived on the mechanic rather than on the kit could not
/// express that at all, and this is what says so.
#[test]
fn the_chicken_flaps_eight_times_and_a_ninth_does_nothing() {
    let mut game = Game::new();
    let player = game.player("chicken", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
    let view = game.world.entity_from_id(player);

    let chicken = kit::by_name(&game.world, "Chicken").expect("the Chicken kit is registered");
    kit::apply(&game.world, view, chicken);
    assert_eq!(
        view.cloned::<&JumpsLeft>().0,
        8,
        "picking the Chicken should hand over eight jumps"
    );

    airborne(&game, player);
    game.server.take();
    for _ in 0..8 {
        press(&game, player);
    }
    assert_eq!(view.cloned::<&JumpsLeft>().0, 0);
    assert_eq!(
        impulses(&game, id).len(),
        8,
        "eight presses should be eight launches"
    );

    press(&game, player);
    assert_eq!(
        impulses(&game, id).len(),
        8,
        "the ninth flap should buy nothing"
    );
}

/// Everybody else gets one, and picking a kit is what says so.
#[test]
fn an_ordinary_kit_gets_one_jump() {
    let mut game = Game::new();
    let player = game.player("wolf", Vec3::new(0.0, 64.0, 0.0));
    let view = game.world.entity_from_id(player);

    let wolf = kit::by_name(&game.world, "Wolf").expect("the Wolf kit is registered");
    kit::apply(&game.world, view, wolf);
    assert_eq!(view.cloned::<&JumpsLeft>().0, 1);
}

/// A jump is drawn and heard where it happened.
///
/// An invisible, silent double jump is the same bug as an invisible ability:
/// the physics are right and the only symptom is that nobody watching can tell
/// a recovery from a fall.
#[test]
fn a_jump_is_seen_and_heard_where_it_happened() {
    let mut game = Game::new();
    let at = Vec3::new(12.0, 70.0, -4.0);
    let player = game.player("noisy", at);

    airborne(&game, player);
    game.server.take();
    press(&game, player);

    let calls = game.server.calls();
    assert!(
        calls.iter().any(
            |call| matches!(call, Call::Particles(effect) if effect.origin().y >= at.y
                && effect.origin().y < at.y + 1.0)
        ),
        "a jump drew nothing at the player's feet: {calls:?}"
    );
    let sounds = game.server.sounds();
    assert_eq!(sounds.len(), 1, "a jump should make exactly one noise");
    assert_eq!(sounds[0].1.id, smash::module::sound::DOUBLE_JUMP);
    assert!(
        sounds[0].0.distance(at) < 1.0,
        "the jump was heard {:?} away from the player who took it",
        sounds[0].0 - at
    );
}

/// Controlled and uncontrolled are two different jumps, and the kit is what
/// decides which.
///
/// Driven through the pure function rather than through two simulated players,
/// because what is being compared is the pair of vectors and not where anybody
/// ended up.
#[test]
fn a_controlled_jump_goes_where_the_player_is_looking() {
    let plain = KitStats::default();
    let steered = KitStats {
        jump_control: true,
        ..KitStats::default()
    };
    let look = Vec3::new(0.0, 0.0, 1.0);

    let up = jump::impulse(plain, look);
    assert_eq!(
        up,
        Vec3::Y * plain.jump_power,
        "an uncontrolled jump goes straight up whatever the player is looking at"
    );

    let along = jump::impulse(steered, look);
    assert!(
        along.z > 0.0,
        "a controlled jump should carry the player where they look; got {along:?}"
    );
    assert!(
        along.y > 0.0,
        "a controlled jump still has to gain height; got {along:?}"
    );
}

/// A kit's jump power is the thing that reaches the client.
///
/// The whole of ENG-11440 was that every kit declared this number and nothing
/// read it, so the assertion that matters is not that an impulse happened but
/// that a heavier kit's is bigger.
#[test]
fn a_kit_with_more_jump_power_is_launched_further() {
    let power_of = |name: &str| {
        let mut game = Game::new();
        let player = game.player(name, Vec3::new(0.0, 64.0, 0.0));
        let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
        let view = game.world.entity_from_id(player);
        let kit = kit::by_name(&game.world, name).expect("kit is registered");
        kit::apply(&game.world, view, kit);
        airborne(&game, player);
        game.server.take();
        press(&game, player);
        impulses(&game, id)
            .first()
            .copied()
            .unwrap_or(Vec3::ZERO)
            .length()
    };

    // Slime declares 1.2 and the Iron Golem 0.9, both uncontrolled.
    assert!(
        power_of("Slime") > power_of("Iron Golem"),
        "the Slime's higher jump power did not reach the client"
    );
}

/// A player watching the match from the sky is left alone.
///
/// A spectator already flies, and an abilities packet that said otherwise
/// would stick them to the floor of a game they are no longer in.
#[test]
fn a_dead_player_is_not_armed() {
    let mut game = Game::new();
    let player = game.player("dead", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
    let view = game.world.entity_from_id(player);

    view.set(smash::module::lives::RespawnAt(f32::MAX));
    view.set(OnGround(false));
    game.server.take();
    game.advance(TICK * 3.0, 3);

    assert!(
        game.server.flight_of(id).is_empty(),
        "a player waiting to respawn was sent an abilities packet"
    );
    assert!(
        !view.cloned::<&MayFly>().0,
        "a spectating player was armed for a double jump"
    );
    assert_eq!(
        view.cloned::<&Position>().0,
        Vec3::new(0.0, 64.0, 0.0),
        "nothing should have moved them"
    );
}

/// Dying in mid-air must not carry the jump you had, or the press you made,
/// across the respawn.
///
/// Both halves of this are the same root cause and neither is visible from
/// `a_dead_player_is_not_armed` above, which asserts a spectator is left alone
/// and then stops. `MayFly` is a write-cache and `Flying` is a mirror, and
/// while a player is dead the two systems that maintain them are filtered out
/// by `RespawnAt`, so whatever was true at the moment of death is still there
/// at the moment of return:
///
/// * The client resets its own abilities on the gamemode change respawning
///   causes -- `GameType.updatePlayerAbilities` sets `mayfly = false`, out of
///   band and without telling the server. A cache that still says `true` means
///   `arm_double_jump` compares equal and sends nothing, and the player has a
///   `JumpsLeft` their client will not spend. That is the mechanic failing in
///   exactly the situation it exists for, because the normal way to die here is
///   in mid-air and the first thing that happens to a fresh respawn is being
///   knocked off again.
/// * A press made on the tick of death is still `true` on the host, so the
///   first tick the filter stops applying it is spent as a real jump, at the
///   spawn point, from a keypress made before the player died.
///
/// The real-client leg cannot see either: `Match.prove_double_jump` runs in the
/// hub before anybody has died.
#[test]
fn respawning_in_mid_air_hands_back_a_jump_the_client_can_actually_take() {
    let mut game = Game::new();
    let player = game.player("recovering", Vec3::new(0.0, 64.0, 0.0));
    let id = game.world.entity_from_id(player).cloned::<&PlayerId>();
    let view = game.world.entity_from_id(player);

    // Airborne with a jump in hand, and a press in flight on the tick they die.
    airborne(&game, player);
    assert!(view.cloned::<&MayFly>().0, "should be armed before dying");
    view.set(Flying(true));
    kill(view, DeathCause::Void);

    // The respawn is gated on the match clock, which only advances while the
    // lobby says the game is running.
    game.world.get::<&mut MatchClock>(|clock| clock.0 = 10.0);
    game.world.progress_time(TICK);
    assert!(
        view.try_get::<&RespawnAt>(|r| r.0).is_none(),
        "the respawn should have landed"
    );

    game.server.take();
    // hyperion reaches the host through a teleport the client has to confirm,
    // so `is_grounded` still reports the pre-death airborne position for a few
    // ticks. This is the window the bug lives in, not an edge case.
    view.set(OnGround(false));
    game.advance(TICK * 3.0, 3);

    assert!(
        impulses(&game, id).is_empty(),
        "a press made before dying was replayed at the spawn point: {:?}",
        impulses(&game, id)
    );
    assert_eq!(
        game.server.flight_of(id),
        vec![Flight::Armed],
        "a player who respawned in mid-air was never told they may jump, so the JumpsLeft they \
         were handed is one the client cannot spend"
    );
}
