//! The modularity claim, made falsifiable.
//!
//! The claim is: *adding a kit is one new module plus one `world.import`,
//! touching no existing match statement, no enum and no dispatch table.*
//!
//! This file is the proof. It defines a complete kit — stats, three abilities
//! with three different activation shapes, an ultimate and a passive that
//! reacts to being hit — entirely from outside the crate, using only the public
//! API. Then it asserts the kit is discoverable, selectable and lethal, and
//! finally greps the crate's own source to confirm that nothing in `src/`
//! mentions it.

mod harness;

use std::sync::atomic::{AtomicU32, Ordering};

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        ability::{self, Cast, Cooldown, Grants, Refusal, Slot, splash},
        damage::{DamageKind, Damaged, hurt},
        kit::{self, AbilitySpec, KitName, KitStats, Playing},
        knockback::KnockbackTaken,
        player::{Energy, Health, Player},
    },
    server::PlayerId,
};

/// How many times the passive fired, so the test can prove an out-of-crate
/// module can hook the damage pipeline.
static SHIELD_TRIGGERS: AtomicU32 = AtomicU32::new(0);

/// A kit that exists only in this test file.
#[derive(Component)]
struct Porcupine;

impl Module for Porcupine {
    fn module(world: &World) {
        world.module::<Self>("smash::kits::Porcupine");

        kit::define(world, "Porcupine", KitStats {
            melee_damage: 4.0,
            armor: 6.0,
            knockback_taken: 1.9,
            regen: 0.45,
            hunger_interval: 6.25,
            jump_power: 1.1,
            energy: Some((1.0, 0.2)),
            ..KitStats::default()
        })
        .cost(1234)
        .blurb("Made up entirely for a test, and no worse for it.")
        .ability(AbilitySpec {
            name: "Quill Burst",
            item: "minecraft:iron_axe",
            slot: 1,
            description: "Everything nearby gets a face full of quills.",
            cooldown: 5.0,
            activate: quill_burst,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Bristle",
            item: "minecraft:iron_sword",
            slot: 2,
            description: "Hold to bristle, release to launch.",
            cooldown: 3.0,
            charge_time: Some(2.0),
            energy_cost: Some(0.5),
            activate: bristle,
            ..AbilitySpec::DEFAULT
        })
        .ability(AbilitySpec {
            name: "Burrow",
            item: "minecraft:iron_shovel",
            slot: 3,
            description: "Only works with your feet on the floor.",
            cooldown: 9.0,
            requires_ground: true,
            activate: |_| {},
            ..AbilitySpec::DEFAULT
        })
        .ultimate(AbilitySpec {
            name: "Quillstorm",
            item: "minecraft:nether_star",
            slot: 8,
            description: "All of the quills, all at once.",
            cooldown: 12.0,
            activate: |cast| splash(cast, 20.0, 18.0, 3.0),
            ..AbilitySpec::DEFAULT
        })
        .register();

        // A passive, as an observer this module owns. Nothing in the crate
        // knows it exists.
        world
            .observer_named::<Damaged, &Health>("smash::kits::Porcupine::retaliate")
            .with(Player::id())
            .each_iter(|it, _index, _health| {
                if it.param().kind != DamageKind::Melee {
                    SHIELD_TRIGGERS.fetch_add(1, Ordering::SeqCst);
                }
            });
    }
}

fn quill_burst(cast: &Cast<'_>) {
    splash(cast, 6.0, 5.0, 2.2);
}

fn bristle(cast: &Cast<'_>) {
    splash(
        cast,
        cast.charge.mul_add(4.0, 3.0),
        cast.charge.mul_add(6.0, 2.0),
        1.5,
    );
}

fn game_with_porcupine() -> Game {
    let game = Game::new();
    game.world.import::<Porcupine>();
    game
}

#[test]
fn a_kit_defined_outside_the_crate_appears_in_the_registry() {
    let game = game_with_porcupine();

    let names: Vec<String> = kit::registry(&game.world)
        .into_iter()
        .filter_map(|entity| {
            game.world
                .entity_from_id(entity)
                .try_get::<&KitName>(|name| name.0.to_owned())
        })
        .collect();

    assert!(
        names.iter().any(|name| name == "Porcupine"),
        "registry did not pick the kit up: {names:?}"
    );
    // And the stock kits are still there, so registration is additive.
    for stock in ["Skeleton", "Iron Golem", "Enderman", "Slime"] {
        assert!(names.iter().any(|name| name == stock), "lost {stock}");
    }
}

#[test]
fn selecting_it_applies_its_stats_and_grants_its_abilities() {
    let mut game = game_with_porcupine();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    let porcupine = kit::by_name(&game.world, "Porcupine").expect("kit not found");
    kit::apply(&game.world, player, porcupine);

    assert!(player.has((Playing, porcupine)));
    assert!((player.cloned::<&KnockbackTaken>().0 - 1.9).abs() < 1e-6);
    assert!((player.cloned::<&smash::module::damage::Armor>().0 - 6.0).abs() < 1e-6);
    assert!(
        player.try_get::<&Energy>(|e| e.max).is_some(),
        "no energy bar"
    );

    let mut slots: Vec<u8> = Vec::new();
    player.each_target(Grants, |granted| {
        if let Some(slot) = granted.try_get::<&Slot>(|s| s.0) {
            slots.push(slot);
        }
    });
    slots.sort_unstable();
    assert_eq!(slots, vec![1, 2, 3], "the ultimate is not granted at spawn");

    let hotbar = kit::hotbar(player);
    assert_eq!(hotbar.len(), 3);
    assert_eq!(hotbar[0].name, "Quill Burst");
    assert_eq!(hotbar[0].item, "minecraft:iron_axe");
}

#[test]
fn its_abilities_go_through_the_same_dispatcher_as_every_other_kit() {
    let mut game = game_with_porcupine();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(3.0, 0.0, 0.0));

    let attacker = game.world.entity_from_id(attacker);
    kit::apply(
        &game.world,
        attacker,
        kit::by_name(&game.world, "Porcupine").unwrap(),
    );

    ability::use_slot(attacker, 1);

    let health = game.world.entity_from_id(victim).cloned::<&Health>();
    assert!(health.current < health.max, "Quill Burst did nothing");
    assert!(
        game.server.total_velocity(PlayerId(2)).length() > 0.0,
        "no knockback was applied"
    );
}

#[test]
fn its_cooldowns_are_enforced_by_the_shared_gate() {
    let mut game = game_with_porcupine();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Porcupine").unwrap(),
    );

    assert_eq!(ability::activate(player, 1, 1.0), Ok(()));
    assert_eq!(
        ability::activate(player, 1, 1.0),
        Err(Refusal::OnCooldown),
        "the second use should have been refused"
    );

    game.advance(6.0, 60);
    assert_eq!(
        ability::activate(player, 1, 1.0),
        Ok(()),
        "the cooldown never expired"
    );
}

#[test]
fn its_ground_requirement_is_enforced_by_the_shared_gate() {
    use smash::module::player::OnGround;

    let mut game = game_with_porcupine();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Porcupine").unwrap(),
    );

    player.set(OnGround(false));
    assert_eq!(ability::activate(player, 3, 1.0), Err(Refusal::NotGrounded));

    player.set(OnGround(true));
    assert_eq!(ability::activate(player, 3, 1.0), Ok(()));
}

#[test]
fn its_energy_cost_is_enforced_by_the_shared_gate() {
    let mut game = game_with_porcupine();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Porcupine").unwrap(),
    );

    player.set(Energy {
        current: 0.1,
        max: 1.0,
        regen: 0.2,
    });
    assert_eq!(
        ability::activate(player, 2, 1.0),
        Err(Refusal::NotEnoughEnergy)
    );

    player.set(Energy {
        current: 1.0,
        max: 1.0,
        regen: 0.2,
    });
    assert_eq!(ability::activate(player, 2, 1.0), Ok(()));
    assert!(
        player.cloned::<&Energy>().current < 1.0,
        "activation did not spend energy"
    );
}

#[test]
fn a_passive_owned_by_the_kit_module_sees_the_damage_pipeline() {
    let mut game = game_with_porcupine();
    let victim = game.player("victim", Vec3::new(3.0, 0.0, 0.0));

    SHIELD_TRIGGERS.store(0, Ordering::SeqCst);
    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: None,
        amount: 2.0,
        knockback: smash::module::knockback::Knockback::from(Vec3::ZERO),
        kind: DamageKind::Projectile,
    });
    assert_eq!(SHIELD_TRIGGERS.load(Ordering::SeqCst), 1);

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: None,
        amount: 2.0,
        knockback: smash::module::knockback::Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });
    assert_eq!(
        SHIELD_TRIGGERS.load(Ordering::SeqCst),
        1,
        "the passive should only arm against non-melee"
    );
}

#[test]
fn cooldowns_are_per_player_not_per_kit() {
    let mut game = game_with_porcupine();
    let one = game.player("one", Vec3::ZERO);
    let two = game.player("two", Vec3::new(40.0, 0.0, 0.0));
    let porcupine = kit::by_name(&game.world, "Porcupine").unwrap();
    let one = game.world.entity_from_id(one);
    let two = game.world.entity_from_id(two);
    kit::apply(&game.world, one, porcupine);
    kit::apply(&game.world, two, porcupine);

    assert_eq!(ability::activate(one, 1, 1.0), Ok(()));
    assert_eq!(
        ability::activate(two, 1, 1.0),
        Ok(()),
        "one player's cooldown must not gate another's"
    );

    let cooldown_of = |player: EntityView<'_>| {
        ability::granted_in_slot(player, 1)
            .and_then(|a| a.try_get::<&Cooldown>(|c| c.remaining))
            .unwrap_or(0.0)
    };
    assert!(cooldown_of(one) > 0.0 && cooldown_of(two) > 0.0);
}

/// The mechanical half of the claim: nothing shipped in the crate names this
/// kit. If adding one had required editing a dispatch table, this would fail.
#[test]
fn no_file_in_the_crate_mentions_the_new_kit() {
    let src = concat!(env!("CARGO_MANIFEST_DIR"), "/src");
    let mut offenders = Vec::new();
    visit(std::path::Path::new(src), &mut |path| {
        let text = std::fs::read_to_string(path).expect("read source file");
        if text.contains("Porcupine") || text.contains("Quill") {
            offenders.push(path.display().to_string());
        }
    });
    assert!(
        offenders.is_empty(),
        "adding a kit should not have required touching: {offenders:?}"
    );
}

fn visit(dir: &std::path::Path, f: &mut impl FnMut(&std::path::Path)) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("read directory entry").path();
        if path.is_dir() {
            visit(&path, f);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            f(&path);
        }
    }
}
