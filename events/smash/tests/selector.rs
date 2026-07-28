//! The in-world kit selector: the ring, the click, and who owns which mob.
//!
//! Two halves, and the split is the same one `scoreboard::render` draws. The
//! geometry is a pure function of a count and is tested against the hub map
//! file rather than against itself, so a podium that would stand inside a spawn
//! point fails here. Everything else is driven through the same entry point the
//! packet handler calls, against the mock seam, so what is asserted is what a
//! client would be sent.

mod harness;

use flecs_ecs::prelude::*;
use glam::{IVec3, Vec3};
use harness::Game;
use hyperion::{BlockKind, simulation::entity_kind::EntityKind};
use smash::{
    map,
    module::{
        kit::{self, KitMob, KitName, Playing},
        lobby::{self, Lobby, LobbyConfig, Phase},
        selector::{
            self, FREE_BLOCK, MAX_RADIUS, MIN_RADIUS, PLINTH_Y, Plinth, StandsOn, TAKEN_BLOCK,
        },
    },
    server::{Channel, PlayerId, mock::Call},
};

/// The hub's glass wall, from `maps/hub.map`.
const WALL_RADIUS: f32 = 19.0;

/// The raised centre the countdown is read from, from `maps/hub.map`.
const DAIS_RADIUS: f32 = 4.0;

fn radius(at: IVec3) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a hub is tens of blocks across")]
    let (x, z) = (at.x as f32, at.z as f32);
    x.hypot(z)
}

// ---------------------------------------------------------------------------
// The ring, as pure geometry
// ---------------------------------------------------------------------------

#[test]
fn an_empty_roster_makes_no_ring() {
    assert!(selector::ring(0).is_empty());
}

#[test]
fn every_kit_gets_its_own_block() {
    for count in 1..=24 {
        let ring = selector::ring(count);
        assert_eq!(ring.len(), count);

        let mut seen = ring.clone();
        seen.sort_unstable_by_key(|at| (at.x, at.z));
        seen.dedup();
        assert_eq!(
            seen.len(),
            count,
            "a roster of {count} puts two podiums in one block: {ring:?}"
        );
    }
}

#[test]
fn the_ring_stands_between_the_dais_and_the_wall() {
    for count in 1..=24 {
        for at in selector::ring(count) {
            assert_eq!(at.y, PLINTH_Y);
            let radius = radius(at);
            assert!(
                radius > DAIS_RADIUS,
                "a podium at {at} stands on the raised centre"
            );
            // Rounding to a block can move a podium half a block either way.
            assert!(
                (MIN_RADIUS - 1.0..=MAX_RADIUS + 1.0).contains(&radius),
                "a podium at {at} is {radius} from the middle, outside the ring's own bounds"
            );
            assert!(
                radius < WALL_RADIUS,
                "a podium at {at} is inside the hub's glass wall"
            );
        }
    }
}

/// A podium on a spawn point is a player spawning inside a block, which reads
/// as the server being broken rather than as a placement mistake.
#[test]
fn no_podium_stands_on_a_hub_spawn_point() {
    let hub = map::parse(map::HUB).expect("the hub map parses");
    for count in 1..=24 {
        for at in selector::ring(count) {
            for spawn in &hub.spawns {
                let gap = Vec3::new(
                    #[expect(clippy::cast_precision_loss, reason = "hub-sized coordinates")]
                    (at.x as f32 - spawn.x),
                    0.0,
                    #[expect(clippy::cast_precision_loss, reason = "hub-sized coordinates")]
                    (at.z as f32 - spawn.z),
                )
                .length();
                assert!(
                    gap >= 1.5,
                    "a roster of {count} puts a podium at {at}, {gap} blocks from the spawn at \
                     {spawn}"
                );
            }
        }
    }
}

#[test]
fn the_wool_and_the_block_the_mob_stands_in_are_one_podium() {
    let plinth = Plinth {
        base: IVec3::new(8, PLINTH_Y, 0),
    };
    assert_eq!(plinth.stand(), IVec3::new(8, PLINTH_Y + 1, 0));
    assert!(plinth.covers(plinth.base));
    assert!(plinth.covers(plinth.stand()));
    assert!(!plinth.covers(IVec3::new(8, PLINTH_Y + 2, 0)));
    assert!(!plinth.covers(IVec3::new(9, PLINTH_Y, 0)));
}

// ---------------------------------------------------------------------------
// The roster's own data
// ---------------------------------------------------------------------------

/// A kit that does not say which mob it is gets an armour stand, which is
/// indistinguishable from every other kit that forgot.
#[test]
fn every_kit_declares_a_distinct_mob() {
    let game = Game::new();
    let mut declared: Vec<(&str, &str)> = Vec::new();

    for id in kit::registry(&game.world) {
        let entry = game.world.entity_from_id(id);
        let name = entry
            .try_get::<&KitName>(|name| name.0)
            .expect("a kit has a name");
        let mob = entry
            .try_get::<&KitMob>(|mob| mob.0)
            .unwrap_or_else(|| panic!("{name} declares no mob, so its podium would be empty"));
        assert_ne!(
            mob,
            kit::DEFAULT_MOB,
            "{name} names the fallback explicitly, which is the same problem"
        );
        declared.push((name, mob));
    }

    assert!(!declared.is_empty(), "the registry is empty");
    for (index, (name, mob)) in declared.iter().enumerate() {
        for (other, other_mob) in &declared[index + 1..] {
            assert_ne!(
                mob, other_mob,
                "{name} and {other} are the same mob, so the ring cannot be read"
            );
        }
    }
}

/// The host panics at boot on a mob or a block it cannot resolve, which is the
/// right thing for it to do and the wrong place to find out. A typo in one
/// `.mob(...)` line would take the whole server down, so the same two lookups
/// run here, where the failure is a test with a name on it.
#[test]
fn every_mob_and_block_the_ring_is_made_of_is_real() {
    let game = Game::new();
    selector::build(&game.world, IVec3::ZERO);

    for block in [FREE_BLOCK, TAKEN_BLOCK] {
        assert!(
            BlockKind::from_str(block.trim_start_matches("minecraft:")).is_some(),
            "{block} is not a block, so the server would panic while building the hub"
        );
    }

    let mobs = selector::mobs(&game.world);
    assert_eq!(mobs.len(), kit::registry(&game.world).len());
    for (_, _, mob) in mobs {
        assert!(
            EntityKind::named(mob).is_some(),
            "{mob} is not a mob, so the server would panic while building the hub"
        );
    }
}

// ---------------------------------------------------------------------------
// The world the podiums live in
// ---------------------------------------------------------------------------

/// A world with the ring standing at the origin and the lobby short enough to
/// run a whole match through in a test.
fn lobby_with_podiums() -> Game {
    let game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.4,
        countdown_at_three_quarters: 0.3,
        countdown_at_full: 0.2,
        prepare_seconds: 0.2,
        match_timeout_seconds: 30.0,
        results_seconds: 0.2,
    });
    selector::build(&game.world, IVec3::ZERO);
    game
}

/// The podium offering `name`, and the block a player clicks to take it.
fn podium_for(game: &Game, name: &str) -> Plinth {
    let kit = kit::by_name(&game.world, name).expect("the registry has that kit");
    selector::podiums(&game.world)
        .into_iter()
        .find(|(_, _, offered)| *offered == kit.id())
        .map(|(_, plinth, _)| plinth)
        .expect("every kit has a podium")
}

fn playing(game: &Game, player: Entity) -> Option<&'static str> {
    game.world
        .entity_from_id(player)
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitName>(|name| name.0))
}

fn plinth_block(game: &Game, at: IVec3) -> &'static str {
    selector::plinths(&game.world)
        .into_iter()
        .find(|(position, _)| *position == at)
        .map(|(_, block)| block)
        .expect("that block is a plinth")
}

use smash::flecs_ext::EntityViewExt;

#[test]
fn there_is_one_podium_per_kit_and_each_offers_a_different_one() {
    let game = lobby_with_podiums();
    let kits = kit::registry(&game.world);
    let podiums = selector::podiums(&game.world);

    assert_eq!(podiums.len(), kits.len());

    let mut offered: Vec<Entity> = podiums.iter().map(|(_, _, kit)| *kit).collect();
    offered.sort_unstable();
    offered.dedup();
    assert_eq!(offered.len(), kits.len(), "two podiums offer the same kit");
}

#[test]
fn building_twice_rebuilds_the_ring_rather_than_doubling_it() {
    let game = lobby_with_podiums();
    let before = selector::podiums(&game.world).len();
    selector::build(&game.world, IVec3::ZERO);
    assert_eq!(selector::podiums(&game.world).len(), before);
}

#[test]
fn a_right_click_on_a_podium_picks_that_mob() {
    let mut game = lobby_with_podiums();
    let player = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let podium = podium_for(&game, "Skeleton");

    assert!(selector::click(
        &game.world,
        game.world.entity_from_id(player),
        podium.stand()
    ));
    assert_eq!(playing(&game, player), Some("Skeleton"));
}

/// The wool is clickable as well as the mob, and it has to be: a click that
/// misses the mob by a pixel lands on the block behind it, and a player who
/// meant to pick the Slime should not be told nothing happened.
#[test]
fn the_wool_under_the_mob_picks_the_same_mob() {
    let mut game = lobby_with_podiums();
    let player = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let podium = podium_for(&game, "Slime");

    assert!(selector::click(
        &game.world,
        game.world.entity_from_id(player),
        podium.base
    ));
    assert_eq!(playing(&game, player), Some("Slime"));
}

/// The real path. A player right-clicks the mob, not the block under it, and
/// the click arrives naming an entity and nothing else. `(StandsOn, podium)`
/// is what turns that entity back into a kit.
///
/// The mob is stood up here rather than by the host, because the host is
/// hyperion and the point of this half of the crate is that it runs without
/// one. What the host does is the same two lines: make an entity, relate it.
#[test]
fn a_right_click_on_the_mob_picks_that_mob() {
    let mut game = lobby_with_podiums();
    let player = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let (podium, ..) = selector::podiums(&game.world)
        .into_iter()
        .find(|(_, plinth, _)| *plinth == podium_for(&game, "Creeper"))
        .expect("the Creeper has a podium");

    let mob = game.world.entity().add((StandsOn, podium)).id();

    assert!(selector::click_mob(
        &game.world,
        game.world.entity_from_id(player),
        mob
    ));
    assert_eq!(playing(&game, player), Some("Creeper"));
}

/// Every other entity in the world, which is mostly the other players.
#[test]
fn clicking_something_that_is_not_a_podium_mob_does_nothing() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));

    assert!(!selector::click_mob(
        &game.world,
        game.world.entity_from_id(alice),
        bob
    ));
    assert_eq!(playing(&game, alice), None);
}

/// Tearing the ring down takes its mobs with it. Nothing here says so; the
/// `(StandsOn, podium)` relation is declared to cascade, and this is the check
/// that the declaration is the one that does it.
#[test]
fn rebuilding_the_ring_does_not_leave_its_mobs_behind() {
    let game = lobby_with_podiums();
    let (podium, ..) = selector::podiums(&game.world)
        .into_iter()
        .next()
        .expect("a podium");
    let mob = game.world.entity().add((StandsOn, podium)).id();
    assert!(game.world.entity_from_id(mob).is_alive());

    selector::build(&game.world, IVec3::ZERO);

    assert!(
        !game.world.entity_from_id(mob).is_alive(),
        "the old ring's mobs are still standing in the new one"
    );
}

#[test]
fn clicking_a_block_that_is_not_a_podium_does_nothing() {
    let mut game = lobby_with_podiums();
    let player = game.player("alice", Vec3::new(0.0, 65.0, 0.0));

    assert!(!selector::click(
        &game.world,
        game.world.entity_from_id(player),
        IVec3::new(0, 64, 0)
    ));
    assert_eq!(playing(&game, player), None);
}

#[test]
fn a_mob_somebody_else_is_playing_is_refused_and_says_who_has_it() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));
    let podium = podium_for(&game, "Spider");

    assert!(selector::click(
        &game.world,
        game.world.entity_from_id(alice),
        podium.stand()
    ));
    game.server.take();

    assert!(selector::click(
        &game.world,
        game.world.entity_from_id(bob),
        podium.stand()
    ));

    assert_eq!(playing(&game, bob), None, "bob took a mob that was taken");
    assert_eq!(playing(&game, alice), Some("Spider"), "alice lost her mob");

    let calls = game.server.calls();
    let told = calls.iter().any(|call| {
        matches!(call, Call::Message(id, Channel::ActionBar, text)
            if *id == PlayerId(2)
                && text.plain().contains("Spider")
                && text.plain().contains("alice"))
    });
    assert!(told, "bob was not told who has the Spider: {calls:?}");
}

/// The same rule through the other door. A player who types the command and a
/// player who clicks the podium must be told the same thing, or they will
/// believe there are two rules.
#[test]
fn the_command_refuses_a_taken_mob_with_the_same_words() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));

    lobby::select_kit(&game.world, game.world.entity_from_id(alice), "Cow")
        .expect("the Cow is free");

    let refusal = lobby::select_kit(&game.world, game.world.entity_from_id(bob), "Cow")
        .expect_err("the Cow is taken");
    assert!(
        refusal.contains("Cow") && refusal.contains("alice"),
        "the command said {refusal:?}"
    );
    assert_eq!(playing(&game, bob), None);
}

#[test]
fn taking_the_mob_you_already_have_is_not_a_refusal() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let podium = podium_for(&game, "Wolf");

    for _ in 0..2 {
        assert!(selector::click(
            &game.world,
            game.world.entity_from_id(alice),
            podium.stand()
        ));
    }
    assert_eq!(playing(&game, alice), Some("Wolf"));

    let refused = game.server.calls().iter().any(
        |call| matches!(call, Call::Message(_, Channel::ActionBar, text) if text.plain().contains("taken")),
    );
    assert!(!refused, "clicking your own podium was refused");
}

/// The manifest is the only thing outside this crate that is told where a
/// podium is, so it has to agree with the world rather than describe one.
#[test]
fn the_manifest_names_every_podium_and_who_holds_it() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let taken = podium_for(&game, "Blaze");

    let before = selector::manifest(&game.world);
    assert_eq!(before.len(), selector::podiums(&game.world).len());
    assert!(
        before.iter().all(|offer| offer.held_by.is_none()),
        "somebody holds a mob in an empty lobby: {before:?}"
    );

    selector::click(&game.world, game.world.entity_from_id(alice), taken.stand());

    let after = selector::manifest(&game.world);
    let blaze = after
        .iter()
        .find(|offer| offer.name == "Blaze")
        .expect("the Blaze has a podium");
    assert_eq!(blaze.held_by.as_deref(), Some("alice"));
    assert_eq!(blaze.wool, TAKEN_BLOCK);
    assert_eq!(blaze.click, taken.stand());
    assert_eq!(blaze.base, taken.base);
    assert_eq!(
        after.iter().filter(|o| o.held_by.is_some()).count(),
        1,
        "one click held more than one mob"
    );

    // Clicking what the manifest says to click is the whole contract with the
    // gate: every entry has to be a block the click handler answers.
    for offer in &after {
        assert!(
            selector::podium_at(&game.world, offer.click).is_some(),
            "{} says click {:?}, which is not a podium",
            offer.name,
            offer.click
        );
    }
}

// ---------------------------------------------------------------------------
// The colour, which is the part a player reads
// ---------------------------------------------------------------------------

#[test]
fn a_podium_turns_red_when_its_mob_is_taken_and_nothing_else_does() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let taken = podium_for(&game, "Blaze");
    let free = podium_for(&game, "Chicken");

    assert_eq!(plinth_block(&game, taken.base), FREE_BLOCK);

    selector::click(&game.world, game.world.entity_from_id(alice), taken.stand());

    assert_eq!(plinth_block(&game, taken.base), TAKEN_BLOCK);
    assert_eq!(plinth_block(&game, free.base), FREE_BLOCK);

    let red = selector::plinths(&game.world)
        .into_iter()
        .filter(|(_, block)| *block == TAKEN_BLOCK)
        .count();
    assert_eq!(
        red, 1,
        "one player took one mob and lit more than one podium"
    );
}

/// Nothing anywhere frees a mob on disconnect, and that is the point: the claim
/// *is* the player's `(Playing, kit)` edge, so destroying the player destroys
/// the claim. A cached set of taken kits would need a handler here, and the day
/// somebody forgot to write one the mob would be reserved forever.
#[test]
fn a_holder_who_disconnects_frees_their_mob() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));
    let podium = podium_for(&game, "Creeper");

    selector::click(
        &game.world,
        game.world.entity_from_id(alice),
        podium.stand(),
    );
    assert_eq!(plinth_block(&game, podium.base), TAKEN_BLOCK);

    // What hyperion does to a player whose connection drops.
    game.world.entity_from_id(alice).destruct();

    assert_eq!(plinth_block(&game, podium.base), FREE_BLOCK);
    assert!(selector::click(
        &game.world,
        game.world.entity_from_id(bob),
        podium.stand()
    ));
    assert_eq!(playing(&game, bob), Some("Creeper"));
}

// ---------------------------------------------------------------------------
// The lobby running underneath it
// ---------------------------------------------------------------------------

#[test]
fn a_selection_made_in_the_hub_survives_the_countdown_and_the_scatter() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));

    let hers = podium_for(&game, "Enderman");
    let his = podium_for(&game, "Snowman");
    selector::click(&game.world, game.world.entity_from_id(alice), hers.stand());
    selector::click(&game.world, game.world.entity_from_id(bob), his.stand());

    // Two players is `min_players`, so the countdown runs on its own.
    game.advance(2.0, 40);
    assert_eq!(
        game.world.cloned::<&Lobby>().phase,
        Phase::Playing,
        "the match never started"
    );

    assert_eq!(playing(&game, alice), Some("Enderman"));
    assert_eq!(playing(&game, bob), Some("Snowman"));
    assert_eq!(plinth_block(&game, hers.base), TAKEN_BLOCK);
    assert_eq!(plinth_block(&game, his.base), TAKEN_BLOCK);
}

#[test]
fn a_podium_click_once_the_match_has_committed_is_refused() {
    let mut game = lobby_with_podiums();
    let alice = game.player("alice", Vec3::new(0.0, 65.0, 0.0));
    let bob = game.player("bob", Vec3::new(2.0, 65.0, 0.0));

    let hers = podium_for(&game, "Guardian");
    selector::click(&game.world, game.world.entity_from_id(alice), hers.stand());
    selector::click(
        &game.world,
        game.world.entity_from_id(bob),
        podium_for(&game, "Zombie").stand(),
    );

    game.advance(2.0, 40);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);
    game.server.take();

    let other = podium_for(&game, "Cow");
    selector::click(&game.world, game.world.entity_from_id(alice), other.stand());

    assert_eq!(
        playing(&game, alice),
        Some("Guardian"),
        "a mid-match click changed a kit"
    );
    let told = game.server.calls().iter().any(|call| {
        matches!(call, Call::Message(_, Channel::ActionBar, text)
            if text.plain().contains("cannot change kit"))
    });
    assert!(told, "the mid-match refusal said nothing");
}

/// One holder per mob, whatever order the clicks arrive in.
#[test]
fn no_two_players_ever_hold_the_same_mob() {
    let mut game = lobby_with_podiums();
    let players: Vec<Entity> = (0..4)
        .map(|index| {
            #[expect(clippy::cast_precision_loss, reason = "four players")]
            let x = index as f32;
            game.player(&format!("p{index}"), Vec3::new(x, 65.0, 0.0))
        })
        .collect();

    // Everybody goes for the same three podiums.
    let contested: Vec<_> = ["Iron Golem", "Slime", "Skeleton"]
        .into_iter()
        .map(|name| podium_for(&game, name))
        .collect();

    for player in &players {
        for podium in &contested {
            selector::click(
                &game.world,
                game.world.entity_from_id(*player),
                podium.stand(),
            );
        }
    }

    let mut held: Vec<Entity> = kit::claims(&game.world)
        .into_iter()
        .map(|claim| claim.kit)
        .collect();
    let total = held.len();
    held.sort_unstable();
    held.dedup();
    assert_eq!(held.len(), total, "two players ended up on one mob");
}
