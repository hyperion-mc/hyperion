//! The nine keys a player has, and what every kit puts on them.
//!
//! The bug this file exists to refuse: twelve of the fifteen kits left slot 0
//! empty and started at slot 1. Slot 0 is where a client's hand rests when it
//! spawns, so twelve kits gave a player a bar whose first key did nothing and
//! whose first ability could not be fired until they scrolled. Nothing failed.
//! The abilities were all there, all reachable, all correctly bound; the hotbar
//! was simply one key to the right of where a hand was.
//!
//! There is nothing a per-ability slot number can be checked against on its
//! own, which is why it drifted: `slot: 1` is wrong only in relation to the
//! rest of its kit. So the layout is no longer written down per ability at all.
//! [`kit::KitBuilder::ability`] hands out slots in declaration order from 0 and
//! [`kit::KitBuilder::ultimate`] always takes [`kit::ULTIMATE_SLOT`], which
//! makes the empty first key unreachable rather than merely absent today.
//!
//! What is left for this file is to hold that derivation to what it claims,
//! over the whole roster and over the path the adapter actually pushes. Every
//! sweep enumerates [`kit::registry`] or [`ability::manifest`], both of which
//! are queries over the world, so a kit added tomorrow is covered the moment
//! its module runs.

mod harness;

use std::collections::{BTreeMap, BTreeSet};

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::module::{
    ability::{self, Declared},
    kit::{self, HOTBAR_SLOTS, KitName, ULTIMATE_SLOT},
};

/// The whole roster's abilities, grouped by the kit that declared them.
fn by_kit(game: &Game) -> BTreeMap<&'static str, Vec<Declared>> {
    let mut out: BTreeMap<&'static str, Vec<Declared>> = BTreeMap::new();
    for entry in ability::manifest(&game.world) {
        out.entry(entry.kit).or_default().push(entry);
    }
    out
}

/// The guard that makes every sweep below mandatory.
///
/// A sweep over an empty registry passes, and a registry that stopped being
/// discovered is exactly the failure that would empty it. The count is a lower
/// bound so adding a kit does not mean editing a number here.
#[test]
fn the_registry_holds_the_whole_roster() {
    let game = Game::new();
    let kits = by_kit(&game);
    assert!(
        kits.len() >= 15,
        "the registry only found {} kits, so a layout sweep would be checking almost nothing: {:?}",
        kits.len(),
        kits.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        kits.len(),
        kit::registry(&game.world).len(),
        "a registered kit declares no abilities at all, so it reaches a player with an empty \
         hotbar"
    );
}

/// The operator's bug, stated as the thing it is.
///
/// Separate from the layout sweep below even though that one subsumes it,
/// because this is the sentence someone reads in a failure log, and "Blaze
/// leaves slot 0 empty" is a bug report where "Blaze lays out {1, 2} rather
/// than {0, 1}" is a puzzle.
#[test]
fn every_kit_puts_something_on_the_key_a_hand_rests_on() {
    let game = Game::new();
    let empty: Vec<&str> = by_kit(&game)
        .iter()
        .filter(|(_, abilities)| !abilities.iter().any(|entry| entry.slot == 0))
        .map(|(kit, _)| *kit)
        .collect();
    assert!(
        empty.is_empty(),
        "these kits leave hotbar slot 0 empty, so a player who spawns and right-clicks fires \
         nothing: {empty:?}"
    );
}

/// The layout, in full: starting abilities fill 0..n with no holes.
///
/// Contiguity and not merely "starts at 0", because a gap is a key that does
/// nothing in the middle of a bar rather than at the end of it, and there is no
/// reading of the hotbar under which a kit wants one. Two or three abilities
/// have no business being spread across nine keys.
#[test]
fn starting_abilities_fill_the_left_of_the_bar_with_no_holes() {
    let game = Game::new();
    let mut wrong = Vec::new();
    for (kit, abilities) in by_kit(&game) {
        let starting: BTreeSet<u8> = abilities
            .iter()
            .filter(|entry| !entry.ultimate)
            .map(|entry| entry.slot)
            .collect();
        let wanted: BTreeSet<u8> = (0..u8::try_from(starting.len()).unwrap()).collect();
        if starting != wanted {
            wrong.push(format!("{kit} lays out {starting:?}, wanted {wanted:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "a kit's starting abilities must occupy slot 0 upwards with no gaps: {wrong:?}"
    );
}

/// The far right of the bar is the Smash Crystal's, on every kit.
///
/// The one binding a player carries across every mob they play, which is worth
/// nothing if a kit can put it somewhere else.
#[test]
fn every_ultimate_sits_in_the_last_slot() {
    let game = Game::new();
    let stray: Vec<String> = ability::manifest(&game.world)
        .into_iter()
        .filter(|entry| entry.ultimate && entry.slot != ULTIMATE_SLOT)
        .map(|entry| format!("{} / {} in slot {}", entry.kit, entry.name, entry.slot))
        .collect();
    assert!(
        stray.is_empty(),
        "an ultimate must sit in slot {ULTIMATE_SLOT}, the far right of the bar: {stray:?}"
    );

    let missing: Vec<&str> = by_kit(&game)
        .iter()
        .filter(|(_, abilities)| !abilities.iter().any(|entry| entry.ultimate))
        .map(|(kit, _)| *kit)
        .collect();
    assert!(
        missing.is_empty(),
        "these kits declare no Smash Crystal ability, so the crystal does nothing for the players \
         holding them: {missing:?}"
    );
}

/// No ability may be bound to a key that does not exist.
///
/// Worth having on its own and not only as a consequence of the sweeps above:
/// this is the one that catches a typo, and nothing else in the game would.
/// `set_hotbar` on an out of range index returns an error the adapter discards,
/// so an ability in slot 12 is silently absent rather than loud.
#[test]
fn no_ability_is_bound_off_the_end_of_the_bar() {
    let game = Game::new();
    let unreachable: Vec<String> = ability::manifest(&game.world)
        .into_iter()
        .filter(|entry| entry.slot >= HOTBAR_SLOTS)
        .map(|entry| format!("{} / {} in slot {}", entry.kit, entry.name, entry.slot))
        .collect();
    assert!(
        unreachable.is_empty(),
        "the hotbar has {HOTBAR_SLOTS} keys and no player can reach past them: {unreachable:?}"
    );
}

/// Two abilities on one key means one of them cannot be fired at all.
///
/// Moved here from `tests/abilities.rs`, which is about what an ability does
/// rather than where it sits, and which named only the slot. Naming both
/// abilities is the difference between a bug report and a puzzle.
#[test]
fn no_kit_binds_two_abilities_to_one_key() {
    let game = Game::new();
    let mut clashes = Vec::new();
    for (kit, abilities) in by_kit(&game) {
        let mut seen: BTreeMap<u8, &'static str> = BTreeMap::new();
        for entry in &abilities {
            if let Some(first) = seen.insert(entry.slot, entry.name) {
                clashes.push(format!(
                    "{kit} slot {}: {first} and {}",
                    entry.slot, entry.name
                ));
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "abilities share a hotbar key, so one of each pair is unreachable: {clashes:?}"
    );
}

/// The same claim, one layer out: what a player is actually handed.
///
/// Every sweep above reads the kit prefab. This one plays each kit on a real
/// player and reads [`kit::hotbar`], which is the function `push_stale_hotbars`
/// calls and whose output the adapter writes into the inventory. The two can
/// disagree: `hotbar` reads the player's own instantiated ability entities
/// through `(Grants, ability)`, and a kit whose declaration is perfect still
/// fails here if instantiating it drops something.
#[test]
fn playing_any_kit_hands_out_a_bar_that_starts_at_slot_zero() {
    let mut game = Game::new();
    let player = game.player("hand", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    let mut wrong = Vec::new();
    for kit in kit::registry(&game.world) {
        let kit = game.world.entity_from_id(kit);
        let name = kit
            .try_get::<&KitName>(|name| name.0)
            .unwrap_or("<unnamed>");
        kit::apply(&game.world, player, kit);

        let items = kit::hotbar(player);
        let slots: Vec<u8> = items.iter().map(|item| item.slot).collect();
        let wanted: Vec<u8> = (0..u8::try_from(items.len()).unwrap()).collect();
        if slots != wanted {
            wrong.push(format!("{name} is handed {slots:?}, wanted {wanted:?}"));
        }
    }
    assert!(
        wrong.is_empty(),
        "playing a kit must fill the bar from slot 0 up, because slot 0 is the key a client has \
         selected when it spawns: {wrong:?}"
    );
}

/// The Smash Crystal's ability arrives on the far right and nowhere else.
///
/// A grant is the one thing that changes a live player's bar, and it goes
/// through a different path from `kit::apply`: one ability instance rather than
/// a whole kit. Slot 8 stays empty until it happens.
#[test]
fn the_crystal_adds_the_last_key_and_moves_nothing_else() {
    let mut game = Game::new();
    let player = game.player("holder", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    let mut wrong = Vec::new();
    for kit in kit::registry(&game.world) {
        let kit = game.world.entity_from_id(kit);
        let name = kit
            .try_get::<&KitName>(|name| name.0)
            .unwrap_or("<unnamed>");
        kit::apply(&game.world, player, kit);
        let before: Vec<u8> = kit::hotbar(player).iter().map(|item| item.slot).collect();

        assert!(
            kit::grant_ultimate(&game.world, player, ability::ULTIMATE_SECONDS),
            "{name} refused a Smash Crystal"
        );
        let after: Vec<u8> = kit::hotbar(player).iter().map(|item| item.slot).collect();

        let mut wanted = before.clone();
        wanted.push(ULTIMATE_SLOT);
        if after != wanted {
            wrong.push(format!(
                "{name}: {before:?} became {after:?}, wanted {wanted:?}"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "a Smash Crystal must add slot {ULTIMATE_SLOT} and disturb nothing else: {wrong:?}"
    );
}
