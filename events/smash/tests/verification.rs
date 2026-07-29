//! The exact values and the exact boundaries.
//!
//! Written against a mutation testing report rather than from imagination.
//! Every test here kills a mutant that the suite previously executed without
//! checking: a `<` that could have been `<=`, an arithmetic operator that could
//! have been any other one, a whole function that could have returned a
//! constant. The distinction that matters is between a test that *runs* a line
//! and a test that *pins* it, and these pin.
//!
//! Boundaries get their own tests because an off-by-one in a comparison is
//! invisible to a test that samples either side of it generously. Where a rule
//! says "within ten seconds", the interesting inputs are exactly ten and the
//! smallest step either way, and nothing else.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        ability::{self, Refusal, charge_steps},
        damage::{Armor, DamageKind, Damaged, KILL_CREDIT_WINDOW, LastHitAt, MatchClock, hurt},
        kit::{self, AbilitySpec, KitStats},
        knockback::{Knockback, KnockbackModel, KnockbackTaken, resolve, strength, vanilla},
        lives::{
            DeathCause, Eliminated, InvulnerableUntil, Lives, MAX_LIVES, Placement, RespawnAt,
            is_invulnerable, kill, killer_of, remaining_alive, tint_of,
        },
        lobby::{Lobby, LobbyConfig, Phase, alive_count, player_count, step},
        player::{Energy, Health, OnGround},
        scoreboard::{COLLAPSE_ABOVE, Row, SIDEBAR_WIDTH, render},
    },
    server::{Channel, Component, NamedColor, PlayerId, Score, SidebarLine, TextColor, mock::Call},
};

const EPS: f32 = 1e-5;

// ---------------------------------------------------------------------------
// Vanilla knockback, which nothing else touches
// ---------------------------------------------------------------------------

/// Vanilla's numbers, to the digit.
///
/// `vanilla` exists only to make the design document's comparison checkable, so
/// nothing in the game calls it and nothing was pinning it. Eight separate
/// mutations of its arithmetic went unnoticed. It is three constants and one
/// multiply, and this is what they are.
#[test]
fn vanilla_knockback_is_a_flat_impulse_plus_half_a_block_per_enchantment_level() {
    // Straight along +X, so the horizontal component is entirely in x.
    let impulse = vanilla(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), 0);
    assert!(
        (impulse.x - 0.4).abs() < EPS,
        "horizontal was {}",
        impulse.x
    );
    assert!((impulse.y - 0.4).abs() < EPS, "vertical was {}", impulse.y);
    assert!(impulse.z.abs() < EPS);

    for (levels, want) in [(0u32, 0.4f32), (1, 0.9), (2, 1.4), (3, 1.9)] {
        let impulse = vanilla(Vec3::ZERO, Vec3::new(5.0, 0.0, 0.0), levels);
        assert!(
            (impulse.x - want).abs() < EPS,
            "Knockback {levels} gave {} horizontally, expected {want}",
            impulse.x
        );
        assert!(
            (impulse.y - 0.4).abs() < EPS,
            "the vertical impulse must not depend on the enchantment"
        );
    }
}

/// The push is away from where the attacker actually is.
///
/// Every case above puts the attacker at the origin, where `victim - origin`
/// and `victim + origin` are the same vector, so none of them can tell a
/// direction from its negation. This one puts the attacker somewhere.
#[test]
fn vanilla_knockback_pushes_away_from_the_attacker_wherever_they_stand() {
    // Attacker to the east of the victim, so the push is west.
    let impulse = vanilla(Vec3::new(20.0, 0.0, 0.0), Vec3::new(8.0, 0.0, 0.0), 0);
    assert!(
        (impulse.x + 0.4).abs() < EPS,
        "expected a push of -0.4 in x, got {}",
        impulse.x
    );

    // And on a diagonal, with both offsets non-zero so neither axis can hide a
    // sign error in the other.
    let impulse = vanilla(Vec3::new(10.0, 5.0, -30.0), Vec3::new(4.0, 5.0, -18.0), 0);
    assert!(impulse.x < 0.0, "x was {}", impulse.x);
    assert!(impulse.z > 0.0, "z was {}", impulse.z);
    let horizontal = Vec3::new(impulse.x, 0.0, impulse.z);
    assert!(
        (horizontal.length() - 0.4).abs() < EPS,
        "the horizontal magnitude was {}",
        horizontal.length()
    );
}

/// Vanilla never looks at the victim's condition, which is the whole point of
/// the comparison.
#[test]
fn vanilla_knockback_ignores_health_and_distance() {
    let near = vanilla(Vec3::ZERO, Vec3::new(0.5, 0.0, 0.0), 1);
    let far = vanilla(Vec3::ZERO, Vec3::new(500.0, 0.0, 0.0), 1);
    assert!((near - far).length() < EPS, "{near} against {far}");

    // Co-located: no direction, so no horizontal push, but the lift stays.
    let degenerate = vanilla(Vec3::ZERO, Vec3::ZERO, 3);
    assert!(degenerate.x.abs() < EPS && degenerate.z.abs() < EPS);
    assert!((degenerate.y - 0.4).abs() < EPS);
}

/// The smash formula and vanilla's agree at exactly one point, and diverge
/// either side of it.
#[test]
fn the_smash_formula_and_vanilla_are_different_functions() {
    let model = KnockbackModel::default();
    let full = strength(model, 6.0, Health::full(20.0), KnockbackTaken(1.0), 1.0);
    let smash = resolve(model, full, Vec3::ZERO, Vec3::X, false);
    let flat = vanilla(Vec3::ZERO, Vec3::X, 0);
    assert!(
        (smash.x - flat.x).abs() > 0.01,
        "the two formulas should not coincide at full health: {smash} against {flat}"
    );
}

// ---------------------------------------------------------------------------
// Health and energy, to the value
// ---------------------------------------------------------------------------

/// Healing adds exactly what was asked for, up to the bar.
#[test]
fn healing_adds_the_amount_asked_for_and_stops_at_the_bar() {
    let mut health = Health::full(20.0);
    health.damage(12.0);
    assert!((health.current - 8.0).abs() < EPS, "{}", health.current);

    health.heal(3.0);
    assert!((health.current - 11.0).abs() < EPS, "{}", health.current);

    // Overhealing clamps rather than scaling, wrapping or being ignored.
    health.heal(100.0);
    assert!((health.current - 20.0).abs() < EPS, "{}", health.current);

    // A zero heal is a no-op, not a multiply.
    health.damage(5.0);
    let before = health.current;
    health.heal(0.0);
    assert!((health.current - before).abs() < EPS);
}

/// The fraction is the ratio, not a rounding of it.
#[test]
fn the_health_fraction_is_current_over_max() {
    for (current, max, want) in [
        (20.0f32, 20.0f32, 1.0f32),
        (10.0, 20.0, 0.5),
        (5.0, 20.0, 0.25),
        (0.0, 20.0, 0.0),
        (3.0, 12.0, 0.25),
    ] {
        let health = Health { current, max };
        assert!(
            (health.fraction() - want).abs() < EPS,
            "{current}/{max} gave {}, expected {want}",
            health.fraction()
        );
    }

    // A zero maximum is the degenerate case, and it is defined as dead rather
    // than as a division by zero.
    let broken = Health {
        current: 5.0,
        max: 0.0,
    };
    assert!((broken.fraction() - 0.0).abs() < EPS);
    assert!(
        Health {
            current: 0.0,
            max: -1.0,
        }
        .fraction()
        .abs()
            < EPS
    );
}

/// Death is at zero and not below it.
#[test]
fn a_player_is_dead_at_exactly_zero_health() {
    assert!(
        !Health {
            current: 0.001,
            max: 20.0
        }
        .is_dead()
    );
    assert!(
        Health {
            current: 0.0,
            max: 20.0
        }
        .is_dead()
    );
    assert!(
        Health {
            current: -1.0,
            max: 20.0
        }
        .is_dead()
    );
}

/// Spending exactly what is left succeeds; spending a hair more does not.
#[test]
fn energy_can_be_spent_down_to_exactly_empty_and_no_further() {
    let mut energy = Energy::full(1.0, 0.2);

    assert!(!energy.try_spend(1.0 + 0.01), "overdrew the bar");
    assert!((energy.current - 1.0).abs() < EPS, "a refusal took energy");

    assert!(energy.try_spend(1.0), "could not spend a full bar");
    assert!(energy.current.abs() < EPS, "{} left over", energy.current);

    assert!(!energy.try_spend(0.01), "spent from an empty bar");
    assert!(energy.current.abs() < EPS);

    // A partial spend takes exactly its cost.
    let mut energy = Energy::full(2.0, 0.0);
    assert!(energy.try_spend(0.75));
    assert!((energy.current - 1.25).abs() < EPS, "{}", energy.current);
}

// ---------------------------------------------------------------------------
// Armour, to the percentage
// ---------------------------------------------------------------------------

/// The energy gate's slack is exactly one epsilon and not a whole comparison.
///
/// `try_spend` reads `current + EPSILON < amount`, and the only input that
/// tells `<` from `<=` there is a cost equal to the slack itself. Obscure, but
/// it is the difference between a bar that can be spent to exactly empty and
/// one that cannot, which is what an ultimate costing a full bar depends on.
#[test]
fn an_empty_bar_can_still_pay_a_cost_of_nothing() {
    let mut energy = Energy::full(1.0, 0.0);
    assert!(energy.try_spend(1.0));
    assert!(energy.current.abs() < EPS, "{}", energy.current);

    assert!(
        energy.try_spend(0.0),
        "an empty bar could not pay a cost of zero"
    );
    assert!(
        energy.try_spend(f32::EPSILON),
        "the slack in the comparison is not the width it is written as"
    );
}

/// The wiki's own pairings: twelve points is 48%, sixteen is 64%.
#[test]
fn armour_points_convert_at_four_percent_each_up_to_eighty() {
    for (points, want) in [
        (0.0f32, 0.0f32),
        (1.0, 0.04),
        (10.0, 0.40),
        (12.0, 0.48), // Skeleton
        (16.0, 0.64), // Iron Golem
        (20.0, 0.80), // exactly at the cap
        (25.0, 0.80), // past it
        (1000.0, 0.80),
    ] {
        let got = Armor(points).reduction();
        assert!(
            (got - want).abs() < EPS,
            "{points} points reduced by {got}, expected {want}"
        );
    }

    // Zero armour is the identity, which a `*` turned into a `+` is not.
    assert!((Armor(0.0).apply(17.0) - 17.0).abs() < EPS);
    // And the reduction is applied, not added.
    assert!((Armor(10.0).apply(10.0) - 6.0).abs() < EPS);
    assert!((Armor(20.0).apply(10.0) - 2.0).abs() < EPS);
}

/// Only hunger, lava and the map ignore armour.
#[test]
fn armour_applies_to_everything_except_the_environment() {
    assert!(DamageKind::Melee.is_reduced_by_armor());
    assert!(DamageKind::Projectile.is_reduced_by_armor());
    assert!(DamageKind::Ability.is_reduced_by_armor());
    assert!(!DamageKind::Environment.is_reduced_by_armor());
}

// ---------------------------------------------------------------------------
// Lives, and the boundaries around a death
// ---------------------------------------------------------------------------

/// The kill plane is below, not at.
///
/// A player standing exactly on the configured minimum is inside the map. The
/// generated property either side of the plane cannot reach this: the boundary
/// is one value out of a continuum, and it is the only value at which `<` and
/// `<=` differ.
#[test]
fn standing_exactly_on_the_kill_plane_is_alive() {
    use smash::module::arena::Arena;

    for kill_y in [-64.0f32, 0.0, 34.5, 128.0] {
        let arena = Arena {
            kill_y,
            ..Arena::default()
        };
        assert!(
            !arena.is_out_of_bounds(Vec3::new(0.0, kill_y, 0.0)),
            "the kill plane at {kill_y} killed somebody standing on it"
        );
        assert!(
            arena.is_out_of_bounds(Vec3::new(
                0.0,
                kill_y - f32::EPSILON.mul_add(kill_y.abs(), 1e-3),
                0.0
            )),
            "the kill plane at {kill_y} did not kill somebody below it"
        );
        assert!(!arena.is_out_of_bounds(Vec3::new(0.0, kill_y + 1e-3, 0.0)));
    }
}

/// The colour of every possible remaining-lives count, read through the
/// relation.
///
/// The whole table, because the failure being guarded against is one band
/// going missing and a count falling through to the next colour, which no spot
/// check notices. Driven through the real world rather than a pure function:
/// what matters is that `(ShownAs, tier)` lands on the player and carries the
/// tint, because that edge is what anything asking "who is about to go out"
/// will query.
#[test]
fn the_life_counter_has_one_colour_per_count() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    for (lives, expected) in [
        (0, NamedColor::Gray),
        (1, NamedColor::Red),
        (2, NamedColor::Gold),
        (3, NamedColor::Yellow),
        (4, NamedColor::Green),
        // Four or more is green: nothing hands out a fifth, but the bands are
        // total and the widest one has to absorb everything above it.
        (5, NamedColor::Green),
        (u8::MAX, NamedColor::Green),
    ] {
        player.set(Lives(lives));
        game.advance(0.05, 1);
        assert_eq!(
            tint_of(player),
            Some(expected),
            "{lives} lives should be drawn {expected:?}"
        );
    }
}

/// Being eliminated is gray whatever the life count still says.
///
/// Elimination and a zero life count are separate facts, and the sidebar reads
/// the first: a player put out by something that did not decrement `Lives`
/// would otherwise still be drawn as if they were in the match.
#[test]
fn an_eliminated_player_is_gray_whatever_their_lives_say() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    player.set(Lives(3));
    game.advance(0.05, 1);
    assert_eq!(tint_of(player), Some(NamedColor::Yellow));

    player.add(Eliminated::id());
    game.advance(0.05, 1);
    assert_eq!(tint_of(player), Some(NamedColor::Gray));
}

/// Kill credit expires at exactly the window, not a moment either side.
#[test]
fn kill_credit_expires_exactly_at_the_window() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));
    let victim = game.world.entity_from_id(victim);

    // The clock is set well away from zero first. With a hit stamped at zero,
    // `now - at` and `now + at` are the same number, so every arithmetic
    // mutation of the window check reads as correct.
    game.world.get::<&mut MatchClock>(|clock| clock.0 = 250.0);
    hurt(victim, Damaged {
        attacker: Some(attacker),
        amount: 5.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });
    let at = victim.cloned::<&LastHitAt>().0;
    assert!((at - 250.0).abs() < EPS, "the hit was stamped at {at}");

    assert_eq!(
        killer_of(victim, at),
        Some(attacker),
        "credit at zero delay"
    );
    assert_eq!(
        killer_of(victim, at + KILL_CREDIT_WINDOW),
        Some(attacker),
        "the window is inclusive of its own end"
    );
    assert_eq!(
        killer_of(victim, at + KILL_CREDIT_WINDOW + 0.001),
        None,
        "a hit past the window is not a kill"
    );
    // And a victim nobody has hit has no killer at all.
    let bystander = game.player("bystander", Vec3::new(9.0, 0.0, 0.0));
    assert_eq!(killer_of(game.world.entity_from_id(bystander), 0.0), None);
}

/// Immunity ends at the instant it says it does.
#[test]
fn respawn_immunity_ends_exactly_at_its_deadline() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    assert!(
        !is_invulnerable(player, 0.0),
        "a player with no immunity component is not immune"
    );

    player.set(InvulnerableUntil(10.0));
    assert!(is_invulnerable(player, 9.999));
    assert!(
        !is_invulnerable(player, 10.0),
        "the deadline itself is not covered"
    );
    assert!(!is_invulnerable(player, 10.001));
}

/// Placement counts down from however many are still in.
///
/// The last player standing takes first place, and each earlier elimination
/// takes the place matching how many were left when it happened. A
/// `remaining_alive` stuck at a constant gives everybody the same place.
#[test]
fn placement_is_assigned_in_reverse_elimination_order() {
    let mut game = Game::new();
    let names = ["a", "b", "c", "d"];
    let players: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| game.player(name, Vec3::new(index as f32 * 8.0, 40.0, 0.0)))
        .collect();

    assert_eq!(remaining_alive(game.world.real_world()), 4);

    let mut placements = Vec::new();
    for (index, player) in players.iter().take(3).enumerate() {
        let player = game.world.entity_from_id(*player);
        for _ in 0..MAX_LIVES {
            kill(player, DeathCause::Void);
            player.remove(RespawnAt::id());
            player.get::<&mut Health>(|health| health.current = health.max);
        }
        assert!(
            player.has(Eliminated::id()),
            "{} survived four deaths",
            names[index]
        );
        placements.push(player.cloned::<&Placement>().0);
        assert_eq!(
            remaining_alive(game.world.real_world()),
            3 - index,
            "the alive count did not fall"
        );
    }

    // Out first takes the worst place; each later exit takes a better one, and
    // the survivor never gets one at all because nothing eliminates them. With
    // four players that is fourth, third and second, which is what a results
    // screen wants: first place belongs to whoever is still standing.
    assert_eq!(placements, vec![4, 3, 2], "{placements:?}");
}

/// Each death says how many lives are left, and the last one says it is over.
///
/// The branch is one `lives.0 == 0`, and a test that only looks for "GAME OVER"
/// somewhere in the log passes just as happily when the condition is inverted:
/// the message still appears, on the wrong deaths. Both sides have to be
/// pinned, in order.
#[test]
fn a_death_tells_the_player_what_it_cost_them() {
    let mut game = Game::new();
    let player = game.player("doomed", Vec3::ZERO);
    game.player("other", Vec3::new(30.0, 0.0, 0.0));
    let player = game.world.entity_from_id(player);

    let mut said = Vec::new();
    for _ in 0..MAX_LIVES {
        game.server.take();
        kill(player, DeathCause::Void);
        player.remove(RespawnAt::id());
        player.get::<&mut Health>(|health| health.current = health.max);
        said.push(
            game.server
                .titles_to(PlayerId(1))
                .into_iter()
                .map(|title| title.title.plain())
                .collect::<Vec<_>>(),
        );
    }

    assert_eq!(said[0], vec!["3 lives left!".to_owned()], "{:?}", said[0]);
    assert_eq!(said[1], vec!["2 lives left!".to_owned()], "{:?}", said[1]);
    assert_eq!(said[2], vec!["1 life left!".to_owned()], "{:?}", said[2]);
    assert_eq!(said[3], vec!["GAME OVER".to_owned()], "{:?}", said[3]);
}

/// A respawn is immune for exactly the documented window and no longer.
#[test]
fn a_respawn_is_immune_for_exactly_the_documented_window() {
    use smash::module::lives::RESPAWN_INVULNERABLE_SECS;

    let mut game = Game::new();
    let player = game.player("p", Vec3::new(0.0, 40.0, 0.0));
    let player = game.world.entity_from_id(player);

    // A clock away from zero, so that multiplying by the window is not the same
    // as adding it.
    game.world.get::<&mut MatchClock>(|clock| clock.0 = 400.0);
    kill(player, DeathCause::Void);

    let respawn_at = player.cloned::<&RespawnAt>().0;
    game.world
        .get::<&mut MatchClock>(|clock| clock.0 = respawn_at);
    game.advance(0.05, 1);

    let until = player.cloned::<&InvulnerableUntil>().0;
    assert!(
        (until - (respawn_at + RESPAWN_INVULNERABLE_SECS)).abs() < 1e-3,
        "immune until {until}, expected {}",
        respawn_at + RESPAWN_INVULNERABLE_SECS
    );
    assert!(is_invulnerable(player, until - 0.001));
    assert!(!is_invulnerable(player, until));
}

/// A player out of lives stays out, and a further death changes nothing.
#[test]
fn an_eliminated_player_absorbs_further_deaths_without_changing() {
    let mut game = Game::new();
    let player = game.player("doomed", Vec3::ZERO);
    game.player("other", Vec3::new(30.0, 0.0, 0.0));
    let player = game.world.entity_from_id(player);

    for _ in 0..MAX_LIVES {
        kill(player, DeathCause::Void);
        player.remove(RespawnAt::id());
        player.get::<&mut Health>(|health| health.current = health.max);
    }
    let placement = player.cloned::<&Placement>();
    let before = game.server.calls().len();

    kill(player, DeathCause::Void);
    assert_eq!(player.cloned::<&Lives>().0, 0);
    assert_eq!(player.cloned::<&Placement>(), placement);
    assert_eq!(
        game.server.calls().len(),
        before,
        "a death after elimination still talked to the server"
    );
}

/// The respawn lands on the tick the clock reaches it, not before.
#[test]
fn a_respawn_waits_for_its_deadline_and_then_lands() {
    use smash::module::lives::DEATH_SPECTATE_SECS;

    let mut game = Game::new();
    let player = game.player("p", Vec3::new(0.0, 40.0, 0.0));
    let player = game.world.entity_from_id(player);

    game.world.get::<&mut MatchClock>(|clock| clock.0 = 100.0);
    kill(player, DeathCause::Void);

    let at = player.cloned::<&RespawnAt>().0;
    assert!(
        (at - (100.0 + DEATH_SPECTATE_SECS)).abs() < EPS,
        "the spectate window is {} rather than {DEATH_SPECTATE_SECS}",
        at - 100.0
    );

    // A tick just short of the deadline changes nothing.
    game.world
        .get::<&mut MatchClock>(|clock| clock.0 = at - 0.001);
    game.advance(0.05, 1);
    assert!(
        player.has(RespawnAt::id()),
        "respawned before the spectate window was up"
    );

    game.world.get::<&mut MatchClock>(|clock| clock.0 = at);
    game.advance(0.05, 1);
    assert!(!player.has(RespawnAt::id()), "never respawned");
    let health = player.cloned::<&Health>();
    assert!((health.current - health.max).abs() < EPS);
}

// ---------------------------------------------------------------------------
// Abilities
// ---------------------------------------------------------------------------

/// Charge steps round to the nearest whole step across the range.
#[test]
fn charge_steps_round_to_the_nearest_step() {
    assert_eq!(charge_steps(0.0, 8), 0);
    assert_eq!(charge_steps(0.5, 8), 4);
    assert_eq!(charge_steps(1.0, 8), 8);
    // Rounding, not truncation: three eighths of eight is exactly three.
    assert_eq!(charge_steps(0.375, 8), 3);
    assert_eq!(charge_steps(0.4, 8), 3);
    assert_eq!(charge_steps(0.44, 8), 4);
    // Out of range clamps rather than overflowing.
    assert_eq!(charge_steps(-5.0, 8), 0);
    assert_eq!(charge_steps(9.0, 8), 8);
    assert_eq!(charge_steps(1.0, 0), 0);
}

/// Holding a slot charges it, and letting go fires it at the charge reached.
///
/// The hold-and-release path was entirely unexercised: nothing called
/// `release_slot`, so the charge accumulator, the fraction it is divided into
/// and the release dispatcher were all dead as far as the suite was concerned.
/// Barrage, Block Toss and Slime Rocket are all this shape, and the fraction is
/// what decides how many arrows come out.
#[test]
fn holding_a_slot_charges_it_and_releasing_fires_at_that_charge() {
    /// The charge fraction the last release handed to the ability.
    #[derive(Component, Debug, Default, Clone, Copy)]
    struct LastCharge(f32);

    /// How many times it fired, so a release that does nothing is not confused
    /// with one that fires at zero charge.
    #[derive(Component, Debug, Default, Clone, Copy)]
    struct Fired(u32);

    fn record(cast: &ability::Cast<'_>) {
        cast.world
            .get::<&mut LastCharge>(|last| last.0 = cast.charge);
        cast.world.get::<&mut Fired>(|fired| fired.0 += 1);
    }

    #[derive(Component)]
    struct Drawn;

    impl Module for Drawn {
        fn module(world: &World) {
            world.module::<Self>("smash::kits::Drawn");
            world.component::<LastCharge>();
            world.component::<Fired>();
            world.set(LastCharge::default());
            world.set(Fired::default());

            kit::define(world, "Drawn", KitStats::default())
                .ability(AbilitySpec {
                    name: "Draw",
                    item: "minecraft:bow",
                    description: "Hold to draw.",
                    cooldown: 0.0,
                    charge_time: Some(2.0),
                    activate: record,
                    ..AbilitySpec::DEFAULT
                })
                .register();
        }
    }

    let mut game = Game::new();
    game.world.import::<Drawn>();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Drawn").expect("just defined"),
    );

    // Pressing a charge ability starts the charge rather than firing it.
    ability::use_slot(player, 0);
    assert_eq!(
        game.world.cloned::<&Fired>().0,
        0,
        "a charge ability fired on the press instead of the release"
    );

    // Half of the two second charge time.
    game.advance(1.0, 20);
    ability::release_slot(player, 0);

    assert_eq!(
        game.world.cloned::<&Fired>().0,
        1,
        "the release did not fire"
    );
    let charge = game.world.cloned::<&LastCharge>().0;
    assert!(
        (charge - 0.5).abs() < 1e-3,
        "one second of a two second draw came out as {charge}, expected 0.5"
    );

    // Held past full, the fraction clamps rather than running away.
    ability::use_slot(player, 0);
    game.advance(10.0, 200);
    ability::release_slot(player, 0);
    let charge = game.world.cloned::<&LastCharge>().0;
    assert!(
        (charge - 1.0).abs() < 1e-6,
        "an overdrawn bow reported a charge of {charge}"
    );

    // Tapped, it fires at nothing.
    ability::use_slot(player, 0);
    ability::release_slot(player, 0);
    assert!(
        game.world.cloned::<&LastCharge>().0.abs() < 1e-6,
        "a tap fired at {} charge",
        game.world.cloned::<&LastCharge>().0
    );
    assert_eq!(game.world.cloned::<&Fired>().0, 3);

    // Releasing a slot with nothing in it is silent rather than a refusal.
    game.server.take();
    ability::release_slot(player, 5);
    assert_eq!(game.world.cloned::<&Fired>().0, 3);
    assert!(
        game.server.messages_to(PlayerId(1)).is_empty(),
        "releasing an empty slot said something: {:?}",
        game.server.messages_to(PlayerId(1))
    );
}

/// An ability costing exactly the energy in the bar is allowed.
///
/// The gate reads `current + EPSILON >= cost` rather than `current >= cost`,
/// and the epsilon is the whole point: the bar is a float that has been
/// regenerated in increments, so a player who has visibly refilled it is a few
/// ulps short of the number the kit declared. Without the slack an ultimate
/// costing a full bar is refused at full bar, which reads as the ability being
/// broken.
#[test]
fn an_ability_costing_the_whole_bar_is_allowed_at_a_full_bar() {
    #[derive(Component)]
    struct Costly;

    impl Module for Costly {
        fn module(world: &World) {
            world.module::<Self>("smash::kits::Costly");
            kit::define(world, "Costly", KitStats {
                energy: Some((1.0, 0.0)),
                ..KitStats::default()
            })
            .ability(AbilitySpec {
                name: "Everything",
                item: "minecraft:stick",
                energy_cost: Some(1.0),
                ..AbilitySpec::DEFAULT
            })
            .register();
        }
    }

    let mut game = Game::new();
    game.world.import::<Costly>();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Costly").expect("just defined"),
    );

    // Exactly full, and the cost is exactly the bar.
    player.set(Energy {
        current: 1.0,
        max: 1.0,
        regen: 0.0,
    });
    assert_eq!(
        ability::activate(player, 0, 1.0),
        Ok(()),
        "a full bar could not pay a cost equal to it"
    );
    assert!(player.cloned::<&Energy>().current.abs() < EPS);

    // Empty, and it is refused.
    assert_eq!(
        ability::activate(player, 0, 1.0),
        Err(Refusal::NotEnoughEnergy)
    );

    // A hair under the cost is still refused: the slack is an epsilon, not a
    // discount.
    player.set(Energy {
        current: 0.9,
        max: 1.0,
        regen: 0.0,
    });
    assert_eq!(
        ability::activate(player, 0, 1.0),
        Err(Refusal::NotEnoughEnergy),
        "the energy gate is not a gate"
    );
}

/// The Smash Crystal's grant lasts exactly as long as it was given for.
///
/// `grant_ultimate` puts a countdown on the ability entity and the tick takes
/// the grant back at zero. Both ends matter: taken back early and the crystal
/// is worthless, taken back late and an ultimate outlives the window it was
/// bought for. The comparison is one `<=`, and only the tick that lands exactly
/// on the deadline tells it from `>`.
#[test]
fn a_temporary_ultimate_lasts_exactly_as_long_as_it_was_granted_for() {
    use smash::{flecs_ext::EntityViewExt, module::kit::Ultimate};

    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Skeleton").expect("Skeleton is a stock kit"),
    );

    let holds_ultimate = || {
        player
            .find_target(smash::module::ability::Grants, |ability| {
                ability.has(Ultimate::id())
            })
            .is_some()
    };
    assert!(!holds_ultimate(), "the ultimate is not granted at spawn");

    // One second, in twenty ticks of the game's own length.
    assert!(smash::module::kit::grant_ultimate(&game.world, player, 1.0));
    assert!(holds_ultimate(), "the grant did not take");

    // Nineteen ticks in, it is still there.
    game.advance(0.95, 19);
    assert!(
        holds_ultimate(),
        "the ultimate was taken back before its second was up"
    );

    // The twentieth is the one that lands on zero.
    game.advance(0.05, 1);
    assert!(
        !holds_ultimate(),
        "the ultimate outlived the grant it was given"
    );

    // And it can be granted again afterwards, rather than the expiry leaving
    // something behind that refuses the next one.
    assert!(smash::module::kit::grant_ultimate(&game.world, player, 1.0));
    assert!(holds_ultimate());
    assert!(
        !smash::module::kit::grant_ultimate(&game.world, player, 1.0),
        "a second crystal stacked a second ultimate"
    );
}

/// Every observation has the name the `/abilities` wire format uses.
///
/// These strings are a protocol between the server and the harness that reads
/// them, so a typo is a check that silently never matches rather than an error
/// anybody sees.
#[test]
fn every_observation_has_its_wire_name() {
    use smash::module::ability::Observable;

    for (observable, name) in [
        (Observable::HurtsTarget, "hurts_target"),
        (Observable::LaunchesTarget, "launches_target"),
        (Observable::LaunchesCaster, "launches_caster"),
        (Observable::TeleportsCaster, "teleports_caster"),
        (Observable::HealsCaster, "heals_caster"),
        (Observable::BuffsMelee, "buffs_melee"),
    ] {
        assert_eq!(observable.as_str(), name, "{observable:?}");
    }

    // All distinct, so no two observations collapse into one on the wire.
    let names = [
        Observable::HurtsTarget,
        Observable::LaunchesTarget,
        Observable::LaunchesCaster,
        Observable::TeleportsCaster,
        Observable::HealsCaster,
        Observable::BuffsMelee,
    ]
    .map(Observable::as_str);
    let mut sorted = names.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), names.len(), "two observations share a name");
}

/// A refusal reaches the player, with the reason on it.
///
/// The refusal text and the code that sends it were both dead as far as the
/// suite was concerned: an ability could be silently refused and nothing
/// noticed. On the action bar it is the only feedback a player gets.
#[test]
fn a_refused_ability_tells_the_player_why() {
    assert_eq!(Refusal::OnCooldown.message(), "That ability is recharging.");
    assert_eq!(Refusal::NotEnoughEnergy.message(), "Not enough energy.");
    assert_eq!(Refusal::NotGrounded.message(), "You must be on the ground.");

    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Skeleton").expect("Skeleton is a stock kit"),
    );

    // First use goes through; the second is on cooldown, and that is the one
    // the player must be told about.
    //
    // Slot 1 and not slot 0: Skeleton's first key is Barrage, which is drawn
    // and released rather than tapped, so a bare `use_slot` on it starts a
    // charge and never reaches the cooldown this is about.
    ability::use_slot(player, 1);
    game.server.take();
    ability::use_slot(player, 1);

    let refusals: Vec<_> = game
        .server
        .calls()
        .into_iter()
        .filter_map(|call| match call {
            Call::Message(PlayerId(1), Channel::ActionBar, text) => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(
        refusals.iter().map(Component::plain).collect::<Vec<_>>(),
        vec![Refusal::OnCooldown.message().to_owned()],
        "the player was not told the ability was recharging"
    );
    assert_eq!(
        refusals[0].runs()[0].color(),
        Some(TextColor::Named(NamedColor::Red)),
        "a refusal has to look like one"
    );
}

/// An ability the player does not have is not a refusal.
#[test]
fn using_an_empty_slot_says_nothing() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    player.set(OnGround(true));

    assert_eq!(ability::activate(player, 7, 1.0), Ok(()));
    assert!(
        game.server.messages_to(PlayerId(1)).is_empty(),
        "an empty hotbar slot produced chatter: {:?}",
        game.server.messages_to(PlayerId(1))
    );
}

/// The hotbar carries a tooltip only when there is one to carry.
#[test]
fn an_ability_with_no_description_has_no_lore() {
    #[derive(Component)]
    struct Terse;

    impl Module for Terse {
        fn module(world: &World) {
            world.module::<Self>("smash::kits::Terse");
            kit::define(world, "Terse", KitStats::default())
                .ability(AbilitySpec {
                    name: "Quiet",
                    item: "minecraft:stick",
                    description: "",
                    ..AbilitySpec::DEFAULT
                })
                .ability(AbilitySpec {
                    name: "Loud",
                    item: "minecraft:stone",
                    description: "It says something.",
                    ..AbilitySpec::DEFAULT
                })
                .register();
        }
    }

    let mut game = Game::new();
    game.world.import::<Terse>();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);
    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Terse").expect("just defined"),
    );

    let hotbar = kit::hotbar(player);
    assert_eq!(hotbar.len(), 2);
    assert_eq!(hotbar[0].slot, 0);
    assert!(
        hotbar[0].lore.is_empty(),
        "an empty description became a blank tooltip line: {:?}",
        hotbar[0].lore
    );
    assert_eq!(hotbar[1].lore, vec!["It says something.".to_owned()]);
}

// ---------------------------------------------------------------------------
// The scoreboard
// ---------------------------------------------------------------------------

/// The collapse happens above fourteen rows, not at fourteen.
#[test]
fn the_scoreboard_collapses_only_above_the_threshold() {
    let rows = |count: usize| -> Vec<Row> {
        (0..count)
            .map(|index| Row {
                name: format!("p{index:02}"),
                lives: u8::try_from(index % 5).unwrap_or(0),
                colour: NamedColor::Green,
            })
            .collect()
    };

    let at = render(Phase::Playing, rows(COLLAPSE_ABOVE));
    assert_eq!(
        at.len(),
        COLLAPSE_ABOVE,
        "exactly {COLLAPSE_ABOVE} players should still be listed one per line"
    );

    let above = render(Phase::Playing, rows(COLLAPSE_ABOVE + 1));
    assert_eq!(above.len(), 2, "{above:?}");
    assert!(above[0].text.plain().starts_with("Players Alive: "));
}

/// A collapsed sidebar counts the living and the dead correctly.
///
/// The two numbers are the whole content of a big lobby's scoreboard, and the
/// split is one comparison. A test that only checks the lines are there passes
/// with both numbers wrong.
#[test]
fn a_collapsed_sidebar_splits_the_living_from_the_dead() {
    // Eighteen rows: five with no lives left, thirteen still in.
    let rows: Vec<Row> = (0..18)
        .map(|index| Row {
            name: format!("p{index:02}"),
            lives: if index < 5 { 0 } else { 3 },
            colour: NamedColor::Green,
        })
        .collect();

    let lines = render(Phase::Playing, rows);
    assert_eq!(
        lines
            .iter()
            .map(|line| line.text.plain())
            .collect::<Vec<_>>(),
        vec!["Players Alive: 13".to_owned(), "Players Dead: 5".to_owned()]
    );
    // Rank only. A count is not a rank, and putting it in the score column
    // would reorder the two rows the moment the dead outnumbered the living.
    assert_eq!(lines[0].score, Score::Rank(2));
    assert_eq!(lines[1].score, Score::Rank(1));

    // A player on their last life is alive, not dead.
    let rows: Vec<Row> = (0..16)
        .map(|index| Row {
            name: format!("p{index:02}"),
            lives: u8::from(index > 0),
            colour: NamedColor::Red,
        })
        .collect();
    let lines = render(Phase::Playing, rows);
    assert_eq!(lines[0].text.plain(), "Players Alive: 15");
    assert_eq!(lines[1].text.plain(), "Players Dead: 1");
}

/// The hub gets a waiting line and a match does not.
#[test]
fn the_sidebar_says_it_is_waiting_only_in_the_hub() {
    let rows = vec![Row {
        name: "solo".to_owned(),
        lives: 4,
        colour: NamedColor::Green,
    }];
    for phase in [Phase::Waiting, Phase::Countdown] {
        let lines = render(phase, rows.clone());
        assert_eq!(lines.len(), 2, "{phase:?}: {lines:?}");
        assert_eq!(lines[1].text.plain(), "Waiting for players");
        // A status line has no number to show, and the client draws one
        // anyway unless it is told not to.
        assert_eq!(lines[1].score, Score::Rank(0));
    }
    for phase in [Phase::Preparing, Phase::Playing, Phase::Ended] {
        let lines = render(phase, rows.clone());
        assert_eq!(lines.len(), 1, "{phase:?}: {lines:?}");
    }
}

/// Ties are broken by name, so the sidebar does not shuffle every tick.
#[test]
fn equal_scores_are_ordered_by_name() {
    let rows = vec![
        Row {
            name: "zoe".to_owned(),
            lives: 2,
            colour: NamedColor::Gold,
        },
        Row {
            name: "amy".to_owned(),
            lives: 2,
            colour: NamedColor::Gold,
        },
        Row {
            name: "bob".to_owned(),
            lives: 4,
            colour: NamedColor::Green,
        },
    ];
    let lines = render(Phase::Playing, rows);
    let names: Vec<String> = lines.iter().map(|line| line.text.plain()).collect();
    assert_eq!(names, ["bob", "amy", "zoe"]);
}

/// The panel from the bug report, pinned whole.
///
/// This is the sidebar a player photographed, rendered by the code that
/// replaced the one that drew it. Every field a player can see is asserted,
/// because each of the three faults was invisible to a test that checked only
/// one of them: an assertion on the text alone passes with the colour dropped,
/// and an assertion on the colour alone passes with `[green]` still in the
/// string.
///
/// What the reporter saw, with the score column on the right:
///
/// ```text
/// Super Smash Mobs
/// [green] Emerald_Explorer 4        3
/// [green] Emerald_Explorer 4        2
/// Waiting for players               1
/// ```
///
/// What this asserts:
///
/// ```text
/// Super Smash Mobs
/// Emerald_Explorer                  4     <- drawn green
/// Emerald_Explorer                  4     <- drawn green
/// Waiting for players                     <- no number
/// ```
#[test]
fn the_sidebar_from_the_bug_report_is_a_coloured_name_and_a_life_count() {
    let rows = vec![
        Row {
            name: "Emerald_Explorer".to_owned(),
            lives: 4,
            colour: NamedColor::Green,
        },
        Row {
            name: "Emerald_Explorer".to_owned(),
            lives: 4,
            colour: NamedColor::Green,
        },
    ];

    let drawn: Vec<(String, Option<TextColor>, Option<i32>)> = render(Phase::Waiting, rows)
        .iter()
        .map(|line| {
            let runs = line.text.runs();
            assert_eq!(runs.len(), 1, "a row is one run of text: {:?}", line.text);
            (
                runs[0].text.clone().into_owned(),
                runs[0].color(),
                line.score.drawn(),
            )
        })
        .collect();

    assert_eq!(drawn, vec![
        (
            "Emerald_Explorer".to_owned(),
            Some(TextColor::Named(NamedColor::Green)),
            Some(4),
        ),
        (
            "Emerald_Explorer".to_owned(),
            Some(TextColor::Named(NamedColor::Green)),
            Some(4),
        ),
        ("Waiting for players".to_owned(), None, None),
    ]);
}

/// No row can widen the panel past its budget.
///
/// The panel is as wide as its widest row and the score is drawn hard against
/// the right edge, so one long row is what pushes the red number onto the
/// text. Budgeting is therefore a property of every row against one number,
/// and the number the score takes has to be counted in: a row that fits until
/// somebody has a hundred lives is not budgeted.
#[test]
fn no_row_can_widen_the_panel_past_its_budget() {
    // What the client draws: the text, then a space, then the number against
    // the right edge. A row with no number drawn pays for neither.
    let drawn_width = |line: &SidebarLine| {
        line.text.plain().chars().count()
            + line
                .score
                .drawn()
                .map_or(0, |value| value.to_string().chars().count() + 1)
    };

    // The longest name Minecraft allows, against the widest life count a `u8`
    // can hold. Nothing the game generates is truncated.
    let rows = vec![Row {
        name: "M".repeat(16),
        lives: u8::MAX,
        colour: NamedColor::Green,
    }];
    let lines = render(Phase::Waiting, rows);
    assert_eq!(
        lines[0].text.plain(),
        "M".repeat(16),
        "a real name survives"
    );
    for line in &lines {
        assert!(
            drawn_width(line) <= SIDEBAR_WIDTH,
            "{:?} is {} wide, past the {SIDEBAR_WIDTH} budget",
            line.text.plain(),
            drawn_width(line)
        );
    }

    // A row that cannot fit is cut, not allowed through. The cut keeps the
    // colour, because the colour is the warning and truncation is not a reason
    // to drop it.
    let rows = vec![Row {
        name: "x".repeat(80),
        lives: 1,
        colour: NamedColor::Red,
    }];
    let lines = render(Phase::Playing, rows);
    assert_eq!(lines.len(), 1);
    assert_eq!(
        drawn_width(&lines[0]),
        SIDEBAR_WIDTH,
        "a cut row should spend exactly the budget: {:?}",
        lines[0].text.plain()
    );
    assert!(
        lines[0].text.plain().ends_with(".."),
        "a cut row should say it was cut: {:?}",
        lines[0].text.plain()
    );
    assert_eq!(
        lines[0].text.runs()[0].color(),
        Some(TextColor::Named(NamedColor::Red))
    );

    // A full lobby of the worst rows the game can produce, in both shapes.
    for count in [1, COLLAPSE_ABOVE, COLLAPSE_ABOVE + 1, 64] {
        let rows: Vec<Row> = (0..count)
            .map(|index| Row {
                // Sixteen characters, the longest name the client will send.
                name: format!("player{index:010}"),
                lives: u8::try_from(index % 5).unwrap_or(0),
                colour: NamedColor::Gold,
            })
            .collect();
        for phase in [Phase::Waiting, Phase::Playing] {
            for line in &render(phase, rows.clone()) {
                assert!(
                    drawn_width(line) <= SIDEBAR_WIDTH,
                    "{count} rows in {phase:?}: {:?} is {} wide",
                    line.text.plain(),
                    drawn_width(line)
                );
            }
        }
    }
}

/// An unchanged sidebar is not sent again.
///
/// A redraw is several packets per viewer per tick, so the dedup is the
/// difference between a few hundred packets a minute and a few hundred
/// thousand. Nothing was checking it held.
#[test]
fn an_unchanged_sidebar_is_not_redrawn() {
    let mut game = Game::new();
    game.player("a", Vec3::new(0.0, 40.0, 0.0));
    game.player("b", Vec3::new(8.0, 40.0, 0.0));

    game.advance(0.05, 1);
    let sidebars = |game: &Game| {
        game.server
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::Sidebar(_, _, _)))
            .count()
    };
    assert!(sidebars(&game) > 0, "the sidebar was never drawn");

    game.server.take();
    game.advance(1.0, 20);
    assert_eq!(
        sidebars(&game),
        0,
        "the sidebar was redrawn with nothing to say"
    );

    // A death changes a row, and then it is sent again.
    let players = game.players();
    kill(game.world.entity_from_id(players[0]), DeathCause::Void);
    game.advance(0.05, 1);
    assert!(
        sidebars(&game) > 0,
        "a changed scoreboard was not sent to anybody"
    );
}

/// A player is on the sidebar on the tick they appear, not the one after.
///
/// The sidebar chooses a colour from the tier table rather than from a
/// player's `(ShownAs, tier)` edge, and this is why. Adding that edge inside a
/// system is a deferred command, so on the tick a player is created there is
/// no edge to read; a sidebar built on one left the newcomer out of the rows
/// *and* out of the viewer list, so the panel under-counted the lobby and the
/// player it had just dropped was never sent one.
#[test]
fn a_player_is_on_the_sidebar_the_tick_they_appear() {
    let mut game = Game::new();
    let joiner = game.player("joiner", Vec3::ZERO);
    let joiner = game.world.entity_from_id(joiner).cloned::<&PlayerId>();

    // Exactly one tick. The edge has not landed yet, and the panel must not
    // care.
    game.advance(0.05, 1);

    let (to, lines) = game
        .server
        .calls()
        .into_iter()
        .find_map(|call| match call {
            Call::Sidebar(id, _, lines) => Some((id, lines)),
            _ => None,
        })
        .expect("no sidebar was drawn on the tick the player appeared");
    assert_eq!(to, joiner);
    assert_eq!(lines[0].text.plain(), "joiner");
    assert_eq!(
        lines[0].text.runs()[0].color(),
        Some(TextColor::Named(NamedColor::Green)),
        "a full-health name is green on its first tick, not uncoloured"
    );
    assert_eq!(lines[0].score, Score::Shown(i32::from(MAX_LIVES)));

    // And the edge does land, one tick later, for whoever queries it.
    assert_eq!(
        tint_of(game.world.entity_from_id(game.players()[0])),
        Some(NamedColor::Green)
    );
}

/// A new viewer gets the sidebar even when its text has not changed.
///
/// The dedup compares the lines *and* the viewers, and only the viewer half
/// catches this: somebody who has just connected has never been sent a
/// scoreboard, so skipping the draw because the text is the same leaves them
/// looking at nothing. A big lobby is where it bites, because there the text is
/// two aggregate counters that a join need not change at all.
#[test]
fn a_player_who_arrives_gets_the_sidebar_even_if_it_reads_the_same() {
    let mut game = Game::new();
    // Past the collapse threshold, so the text is two counters rather than one
    // line per name.
    for index in 0..16 {
        game.player(&format!("p{index:02}"), Vec3::new(index as f32, 40.0, 0.0));
    }
    game.advance(0.05, 1);

    let sidebars_to = |game: &Game, who: PlayerId| {
        game.server
            .calls()
            .into_iter()
            .filter(|call| matches!(call, Call::Sidebar(id, _, _) if *id == who))
            .count()
    };
    assert!(sidebars_to(&game, PlayerId(1)) > 0, "nobody got a sidebar");

    // Swap one player for another. The counters are identical afterwards, so
    // the rendered text is byte for byte what it was.
    let before = game
        .server
        .calls()
        .into_iter()
        .find_map(|call| match call {
            Call::Sidebar(_, _, lines) => Some(lines),
            _ => None,
        })
        .expect("a sidebar was drawn");

    game.world.entity_from_id(game.players()[0]).destruct();
    let arrival = game.player("newcomer", Vec3::new(99.0, 40.0, 0.0));
    let arrival = game.world.entity_from_id(arrival).cloned::<&PlayerId>();

    game.server.take();
    game.advance(0.05, 1);

    let after = game
        .server
        .calls()
        .into_iter()
        .find_map(|call| match call {
            Call::Sidebar(_, _, lines) => Some(lines),
            _ => None,
        })
        .expect("the sidebar was not sent to the new player at all");
    assert_eq!(
        before, after,
        "the setup is only meaningful if the text is unchanged"
    );
    assert!(
        sidebars_to(&game, arrival) > 0,
        "the player who just joined was never sent a scoreboard"
    );
}

// ---------------------------------------------------------------------------
// The lobby's side effects
// ---------------------------------------------------------------------------

/// Counting players, and counting the ones still in.
#[test]
fn the_lobby_counts_players_and_survivors_separately() {
    let mut game = Game::new();
    for index in 0..5 {
        game.player(
            &format!("p{index}"),
            Vec3::new(index as f32 * 8.0, 40.0, 0.0),
        );
    }
    assert_eq!(player_count(&game.world), 5);
    assert_eq!(alive_count(&game.world), 5);

    let players = game.players();
    for player in players.iter().take(2) {
        game.world.entity_from_id(*player).add(Eliminated::id());
    }
    assert_eq!(
        player_count(&game.world),
        5,
        "eliminated players are still here"
    );
    assert_eq!(alive_count(&game.world), 3);
}

/// Starting a match puts everybody on a distinct spawn point.
///
/// The scatter is what makes a match start rather than a countdown that ends
/// with everyone standing in the hub, and its index step was unchecked: an
/// index that fails to advance puts the whole lobby on one platform.
#[test]
fn preparing_scatters_everyone_onto_different_spawns() {
    use smash::module::{arena::Arena, player::Position};

    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.2,
        countdown_at_three_quarters: 0.2,
        countdown_at_full: 0.2,
        prepare_seconds: 5.0,
        ..LobbyConfig::default()
    });
    for index in 0..4 {
        game.player(&format!("p{index}"), Vec3::new(0.0, 100.0, 0.0));
    }

    // Waiting -> Countdown -> Preparing.
    game.advance(1.0, 20);
    assert_eq!(
        game.world.cloned::<&Lobby>().phase,
        Phase::Preparing,
        "the countdown never committed"
    );

    let spawns = game.world.cloned::<&Arena>().spawns;
    let mut placed: Vec<Vec3> = game
        .players()
        .into_iter()
        .map(|player| game.world.entity_from_id(player).cloned::<&Position>().0)
        .collect();
    for at in &placed {
        assert!(spawns.contains(at), "{at} is not a spawn point");
    }
    placed.sort_by(|a, b| a.to_array().partial_cmp(&b.to_array()).expect("finite"));
    placed.dedup();
    assert_eq!(
        placed.len(),
        4,
        "four players landed on {} distinct spawn points",
        placed.len()
    );

    assert!(
        game.server
            .broadcasts()
            .iter()
            .any(|line| line.contains("Get ready")),
        "nobody was told the match was starting: {:?}",
        game.server.broadcasts()
    );
}

/// Returning to the hub hands everybody their lives and health back.
///
/// The reset is what makes one world able to host a second match. It was
/// entirely unchecked, and a reset that does nothing leaves the next match
/// starting with a lobby of corpses.
#[test]
fn returning_to_the_hub_restores_lives_and_health() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.2,
        countdown_at_three_quarters: 0.2,
        countdown_at_full: 0.2,
        prepare_seconds: 0.2,
        match_timeout_seconds: 0.2,
        results_seconds: 0.2,
    });
    for index in 0..4 {
        game.player(
            &format!("p{index}"),
            Vec3::new(index as f32 * 8.0, 40.0, 0.0),
        );
    }

    // Into the match, then hurt and kill people while it runs.
    game.advance(0.6, 12);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);

    let players = game.players();
    let casualty = game.world.entity_from_id(players[0]);
    kill(casualty, DeathCause::Void);
    casualty.remove(RespawnAt::id());
    let wounded = game.world.entity_from_id(players[1]);
    wounded.get::<&mut Health>(|health| health.current = 1.0);

    assert!(casualty.cloned::<&Lives>().0 < MAX_LIVES);

    // Time out the match, sit through the results, and come back to the hub.
    //
    // Tick by tick to the moment it arrives, not for a fixed span: a lobby that
    // still has four players in it leaves `Waiting` again on the very next
    // step, so a fixed advance sails straight past the state being tested and
    // into the next match.
    let mut reached_hub = false;
    for _ in 0..200 {
        game.advance(0.05, 1);
        if game.world.cloned::<&Lobby>().phase == Phase::Waiting {
            reached_hub = true;
            break;
        }
    }
    assert!(reached_hub, "never returned to the hub");

    for player in game.players() {
        let player = game.world.entity_from_id(player);
        assert_eq!(
            player.cloned::<&Lives>().0,
            MAX_LIVES,
            "{} did not get their lives back",
            player.name()
        );
        let health = player.cloned::<&Health>();
        assert!(
            (health.current - health.max).abs() < EPS,
            "{} came back on {} health",
            player.name(),
            health.current
        );
        assert!(!player.has(Eliminated::id()));
    }
    assert!(
        game.world.cloned::<&MatchClock>().0.abs() < EPS,
        "the clock was not reset"
    );
}

/// No per-match state survives into the next match.
///
/// The reset restores lives and health. The question this asks is what it does
/// *not* restore: a finishing position, a queued respawn or a respawn immunity
/// left on a player from the previous game is state the next game reads and
/// gets wrong. A stale `RespawnAt` in particular is measured against a clock
/// that has just been set back to zero, which is how a player who died near the
/// end of one match spends the opening of the next one in spectator mode.
#[test]
fn a_new_match_starts_with_no_leftovers_from_the_last_one() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.2,
        countdown_at_three_quarters: 0.2,
        countdown_at_full: 0.2,
        prepare_seconds: 0.2,
        match_timeout_seconds: 0.2,
        results_seconds: 0.2,
    });
    for index in 0..4 {
        game.player(
            &format!("p{index}"),
            Vec3::new(index as f32 * 8.0, 40.0, 0.0),
        );
    }

    game.advance(0.6, 12);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Playing);

    let players = game.players();
    // One player eliminated outright, and one killed but still waiting to come
    // back when the match runs out. Both are ordinary ways for a match to end.
    let eliminated = game.world.entity_from_id(players[0]);
    for _ in 0..MAX_LIVES {
        kill(eliminated, DeathCause::Void);
        eliminated.remove(RespawnAt::id());
        eliminated.get::<&mut Health>(|health| health.current = health.max);
    }
    let waiting = game.world.entity_from_id(players[1]);
    kill(waiting, DeathCause::Void);
    assert!(
        waiting.has(RespawnAt::id()),
        "the setup did not queue a respawn"
    );

    let mut reached_hub = false;
    for _ in 0..200 {
        game.advance(0.05, 1);
        if game.world.cloned::<&Lobby>().phase == Phase::Waiting {
            reached_hub = true;
            break;
        }
    }
    assert!(reached_hub, "never returned to the hub");

    for player in game.players() {
        let player = game.world.entity_from_id(player);
        let name = player.name();
        assert!(
            player.try_get::<&Placement>(|p| p.0).is_none(),
            "{name} carried a finishing position of {:?} into the next match",
            player.try_get::<&Placement>(|p| p.0)
        );
        assert!(
            player.try_get::<&RespawnAt>(|r| r.0).is_none(),
            "{name} is still queued to respawn at {:?}, against a clock that has been reset to \
             zero",
            player.try_get::<&RespawnAt>(|r| r.0)
        );
        assert!(
            player.try_get::<&InvulnerableUntil>(|u| u.0).is_none(),
            "{name} kept a respawn immunity until {:?} from the previous match",
            player.try_get::<&InvulnerableUntil>(|u| u.0)
        );
    }
}

/// Every phase change is announced.
#[test]
fn each_phase_change_is_broadcast() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.2,
        countdown_at_three_quarters: 0.2,
        countdown_at_full: 0.2,
        prepare_seconds: 0.2,
        match_timeout_seconds: 0.2,
        results_seconds: 0.2,
    });
    for index in 0..4 {
        game.player(
            &format!("p{index}"),
            Vec3::new(index as f32 * 8.0, 40.0, 0.0),
        );
    }
    game.advance(3.0, 60);

    let said = game.server.broadcasts();
    for expected in ["starts shortly", "Get ready", "Go!", "Game over"] {
        assert!(
            said.iter().any(|line| line.contains(expected)),
            "never said {expected:?}: {said:?}"
        );
    }
}

/// The results screen ends when its timer runs out, and not before.
///
/// Generated cases reach this boundary only by luck: a phase drawn one time in
/// five and a timer that has to land inside one step of zero. It is the only
/// input at which the comparison's direction shows, so it is written out.
#[test]
fn the_results_screen_ends_exactly_when_its_timer_does() {
    let config = LobbyConfig::default();
    let ended = |timer| Lobby {
        phase: Phase::Ended,
        timer,
    };

    let next = step(&config, ended(1.0), 0.5, 8, 1);
    assert_eq!(next.phase, Phase::Ended);
    assert!((next.timer - 0.5).abs() < EPS, "{}", next.timer);

    // Exactly out, which is the boundary.
    let next = step(&config, ended(0.5), 0.5, 8, 1);
    assert_eq!(
        next.phase,
        Phase::Waiting,
        "a results screen whose timer hit exactly zero did not end"
    );
    assert!(next.timer.abs() < EPS);

    let next = step(&config, ended(0.1), 0.5, 8, 1);
    assert_eq!(next.phase, Phase::Waiting);
    assert!(next.timer.abs() < EPS, "{}", next.timer);
}

/// Preparation becomes play at exactly zero as well.
#[test]
fn preparation_becomes_play_exactly_when_its_timer_does() {
    let config = LobbyConfig::default();
    let preparing = |timer| Lobby {
        phase: Phase::Preparing,
        timer,
    };

    let next = step(&config, preparing(1.0), 0.5, 8, 8);
    assert_eq!(next.phase, Phase::Preparing);
    assert!((next.timer - 0.5).abs() < EPS);

    let next = step(&config, preparing(0.5), 0.5, 8, 8);
    assert_eq!(next.phase, Phase::Playing);
    assert!(next.timer.abs() < EPS);
}

/// The match clock counts the seconds a match has been running.
///
/// It is what every deadline in the game is measured against -- respawns, kill
/// credit, respawn immunity -- so a clock that advances by the wrong amount
/// moves all of them at once, and one that never advances freezes them all.
#[test]
fn the_match_clock_counts_up_while_a_match_runs() {
    let mut game = Game::new();
    game.world.set(LobbyConfig {
        min_players: 2,
        full_players: 4,
        countdown_at_min: 0.2,
        countdown_at_three_quarters: 0.2,
        countdown_at_full: 0.2,
        prepare_seconds: 0.2,
        match_timeout_seconds: 600.0,
        results_seconds: 0.2,
    });
    for index in 0..4 {
        game.player(
            &format!("p{index}"),
            Vec3::new(index as f32 * 8.0, 40.0, 0.0),
        );
    }

    let mut started = false;
    for _ in 0..100 {
        game.advance(0.05, 1);
        if game.world.cloned::<&Lobby>().phase == Phase::Playing {
            started = true;
            break;
        }
    }
    assert!(started, "the match never started");

    let before = game.world.cloned::<&MatchClock>().0;
    game.advance(1.0, 20);
    let after = game.world.cloned::<&MatchClock>().0;

    assert!(
        (after - before - 1.0).abs() < 1e-3,
        "a second of play moved the clock from {before} to {after}"
    );
}

/// The match clock only runs while a match does.
#[test]
fn the_match_clock_advances_only_during_play() {
    let mut game = Game::new();
    game.player("solo", Vec3::new(0.0, 40.0, 0.0));

    // One player is never enough to start, so the lobby stays in the hub.
    game.advance(2.0, 40);
    assert_eq!(game.world.cloned::<&Lobby>().phase, Phase::Waiting);
    assert!(
        game.world.cloned::<&MatchClock>().0.abs() < EPS,
        "the clock ran in the hub: {}",
        game.world.cloned::<&MatchClock>().0
    );
}

// ---------------------------------------------------------------------------
// Projectiles
// ---------------------------------------------------------------------------

mod projectiles {
    use flecs_ecs::prelude::*;
    use glam::Vec3;
    use hyperion::simulation::{entity_kind::EntityKind, projectile_motion::EYE_HEIGHT};
    use smash::{
        module::{
            player::{Health, Player, Position},
            projectile::{Flight, Payload, Projectile, Visual, fire},
        },
        server::{PlayerId, mock::Call},
    };

    use super::{EPS, Game};

    fn count_projectiles(game: &Game) -> i32 {
        game.world
            .query::<()>()
            .with(Projectile::id())
            .build()
            .count()
    }

    /// A projectile flies, falls, and expires on its own timer.
    ///
    /// The whole integration step was unchecked: gravity, the position update
    /// and the countdown are one line each and any of them could have been the
    /// wrong operator without a test noticing.
    #[test]
    fn a_projectile_flies_falls_and_expires() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::ZERO);
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::new(0.0, 50.0, 0.0),
                velocity: Vec3::new(10.0, 0.0, 0.0),
                gravity: 20.0,
                seconds_left: 1.0,
                radius: 0.5,
            },
            Payload::new(5.0, 1.0),
        );
        assert_eq!(count_projectiles(&game), 1);

        // One tenth of a second, in two ticks of 0.05.
        game.advance(0.1, 2);

        let mut seen: Option<Flight> = None;
        game.world
            .query::<&Flight>()
            .build()
            .each(|flight| seen = Some(*flight));
        let flight = seen.expect("the projectile is still in the air");

        // Horizontal motion is unaffected by gravity: 10 blocks a second for a
        // tenth of a second is one block.
        assert!(
            (flight.position.x - 1.0).abs() < 1e-3,
            "travelled {} horizontally in 0.1s at 10 blocks a second",
            flight.position.x
        );
        // And it has started falling rather than rising. `fire` launches from
        // the shooter's eye, `EYE_HEIGHT` above the given point, so the arrow
        // starts at 50 + 1.62 and this checks it is below that, i.e. falling.
        assert!(
            flight.position.y < 50.0 + EYE_HEIGHT,
            "gravity pushed it up to {}",
            flight.position.y
        );
        assert!(flight.velocity.y < 0.0, "velocity is {}", flight.velocity.y);
        assert!(
            (flight.seconds_left - 0.9).abs() < 1e-3,
            "{} seconds left after 0.1s of a 1.0s life",
            flight.seconds_left
        );

        // Past its lifetime it is gone rather than lingering.
        game.advance(1.0, 20);
        assert_eq!(
            count_projectiles(&game),
            0,
            "an expired projectile survived"
        );
    }

    /// A projectile hits the nearest player in range and then stops existing.
    #[test]
    fn a_projectile_hits_the_nearest_target_and_is_consumed() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(-20.0, 0.0, 0.0));
        let near = game.player("near", Vec3::new(0.0, 0.0, 0.0));
        let far = game.player("far", Vec3::new(1.5, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::new(0.0, 0.0, 0.0),
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 5.0,
                radius: 4.0,
            },
            Payload::new(6.0, 1.0),
        );
        game.advance(0.05, 1);

        let health_of = |entity| game.world.entity_from_id(entity).cloned::<&Health>();
        assert!(
            health_of(near).current < health_of(near).max,
            "the nearest player was not hit"
        );
        assert!(
            (health_of(far).current - health_of(far).max).abs() < EPS,
            "a projectile hit two players at once"
        );
        assert_eq!(
            count_projectiles(&game),
            0,
            "the projectile was not consumed"
        );
        assert!(
            game.server
                .calls()
                .iter()
                .any(|call| matches!(call, Call::AddVelocity(id, _) if *id == PlayerId(2))),
            "the victim was not knocked back"
        );
    }

    /// A target standing exactly on the projectile's radius is hit.
    ///
    /// The contact check is one `distance > radius`, and the boundary is the
    /// only input that tells it from `>=`. Everything either side of it agrees.
    #[test]
    fn a_target_exactly_on_the_radius_is_hit() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(-40.0, 0.0, 0.0));
        let edge = game.player("edge", Vec3::new(3.0, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 0.5,
                // Exactly the distance to `edge`, which is representable
                // exactly, so the comparison is not decided by rounding.
                radius: 3.0,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.05, 1);

        let health = game.world.entity_from_id(edge).cloned::<&Health>();
        assert!(
            health.current < health.max,
            "a target exactly on the radius was missed"
        );
    }

    /// The nearest is chosen however the world happens to store them.
    ///
    /// The comparison that picks a winner runs over entities in whatever order
    /// the query yields, so a test where the nearest is also the first proves
    /// only that the first one wins. Here the distant target is created first
    /// and has to be displaced.
    #[test]
    fn the_nearest_target_wins_even_when_it_is_seen_last() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(-60.0, 0.0, 0.0));
        // Created before the nearer one on purpose.
        let distant = game.player("distant", Vec3::new(3.5, 0.0, 0.0));
        let close = game.player("close", Vec3::new(0.5, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 0.5,
                radius: 6.0,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.05, 1);

        let health_of = |entity| game.world.entity_from_id(entity).cloned::<&Health>();
        assert!(
            health_of(close).current < health_of(close).max,
            "the nearer target was not the one hit"
        );
        assert!(
            (health_of(distant).current - health_of(distant).max).abs() < EPS,
            "the further target was hit as well"
        );
    }

    /// A projectile that passes through somebody hits them, however fast it
    /// was going.
    ///
    /// One tick of a fast projectile is a long line: Barrage's arrows cover
    /// three blocks a step against a hit radius well under one, so a contact
    /// test that only looks at where the projectile ended up leaves most of the
    /// flight path as a hole a player can stand in. This puts the target
    /// squarely in the middle of a step, where both endpoints miss it by a
    /// wide margin and only the swept segment connects.
    #[test]
    fn a_fast_projectile_hits_what_it_passes_through() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 40.0));
        let target = game.player("target", Vec3::ZERO);
        let shooter = game.world.entity_from_id(shooter);

        // 120 blocks a second is six blocks in one 0.05s tick: from -3 to +3,
        // straight through a target at the origin. Both endpoints are three
        // blocks away from a radius of half a block.
        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::new(-3.0, 0.0, 0.0),
                velocity: Vec3::new(120.0, 0.0, 0.0),
                gravity: 0.0,
                seconds_left: 5.0,
                radius: 0.5,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.05, 1);

        let health = game.world.entity_from_id(target).cloned::<&Health>();
        assert!(
            health.current < health.max,
            "a projectile passed straight through a player without touching them"
        );

        // And it connected where it actually crossed, not at either end of the
        // step. That point is what the knockback is measured away from, so a
        // wrong one launches the victim in a wrong direction.
        let contact = game
            .server
            .calls()
            .into_iter()
            .find_map(|call| match call {
                Call::Sound(at, sound) if sound.id == smash::module::sound::PROJECTILE_HIT => {
                    Some(at)
                }
                _ => None,
            })
            .expect("a hit is heard where it landed");
        // The crossing is at the target's column (x, z near 0); the height is
        // the eye level the arrow now flies at, since `fire` launches from the
        // eye, so check the horizontal crossing rather than distance to the feet.
        assert!(
            contact.x.abs() < 0.5 && contact.z.abs() < 0.5,
            "the hit was reported at {contact}, which is not where the path crossed the target"
        );
    }

    /// A path that goes past somebody still misses them.
    ///
    /// The other half of the segment test: widening the contact check until
    /// everything within a step is hit would pass the test above and break the
    /// game.
    #[test]
    fn a_fast_projectile_that_passes_wide_still_misses() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(0.0, 0.0, 40.0));
        let bystander = game.player("bystander", Vec3::new(0.0, 0.0, 4.0));
        let shooter = game.world.entity_from_id(shooter);

        // The same six block step along x, with the bystander four blocks off
        // it in z.
        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::new(-3.0, 0.0, 0.0),
                velocity: Vec3::new(120.0, 0.0, 0.0),
                gravity: 0.0,
                seconds_left: 5.0,
                radius: 0.5,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.05, 1);

        let health = game.world.entity_from_id(bystander).cloned::<&Health>();
        assert!(
            (health.current - health.max).abs() < EPS,
            "a projectile four blocks off its path hit somebody"
        );
    }

    /// A projectile never hits the player who fired it.
    #[test]
    fn a_projectile_passes_through_its_own_shooter() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::ZERO);
        let shooter = game.world.entity_from_id(shooter);
        let before = shooter.cloned::<&Health>();

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 0.5,
                radius: 10.0,
            },
            Payload::new(9.0, 1.0),
        );
        game.advance(0.2, 4);

        assert!(
            (shooter.cloned::<&Health>().current - before.current).abs() < EPS,
            "a projectile hit the player who fired it"
        );
    }

    /// A projectile passes through a corpse to reach somebody still alive.
    #[test]
    fn a_projectile_ignores_the_dead() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(-30.0, 0.0, 0.0));
        let corpse = game.player("corpse", Vec3::ZERO);
        let alive = game.player("alive", Vec3::new(2.0, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);
        game.world
            .entity_from_id(corpse)
            .get::<&mut Health>(|health| health.current = 0.0);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 0.5,
                radius: 5.0,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.05, 1);

        let health = game.world.entity_from_id(alive).cloned::<&Health>();
        assert!(
            health.current < health.max,
            "the projectile stopped on a body instead of reaching the living"
        );
    }

    /// Nothing in range means nothing happens, and the projectile flies on.
    #[test]
    fn a_projectile_with_nothing_in_range_keeps_going() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::ZERO);
        game.player("distant", Vec3::new(500.0, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::new(100.0, 0.0, 0.0),
                velocity: Vec3::new(1.0, 0.0, 0.0),
                gravity: 0.0,
                seconds_left: 5.0,
                radius: 2.0,
            },
            Payload::new(4.0, 1.0),
        );
        game.advance(0.2, 4);
        assert_eq!(
            count_projectiles(&game),
            1,
            "a projectile hit nothing and died"
        );
    }

    /// The extra effect runs, with the victim it hit.
    #[test]
    fn the_on_hit_payload_runs_against_the_victim() {
        use smash::module::projectile::Impact;

        #[derive(Component, Debug, Default)]
        struct Marked;

        fn mark(impact: &Impact<'_>) {
            impact.victim.add(Marked::id());
        }

        let mut game = Game::new();
        game.world.component::<Marked>();
        let shooter = game.player("shooter", Vec3::new(-40.0, 0.0, 0.0));
        let victim = game.player("victim", Vec3::ZERO);
        let shooter = game.world.entity_from_id(shooter);

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 1.0,
                radius: 3.0,
            },
            Payload::new(3.0, 1.0).then(mark),
        );
        game.advance(0.05, 1);

        assert!(
            game.world.entity_from_id(victim).has(Marked::id()),
            "the on-hit effect never ran"
        );
    }

    /// A player entity with no `Player` tag is not a target.
    #[test]
    fn a_projectile_only_targets_players() {
        let mut game = Game::new();
        let shooter = game.player("shooter", Vec3::new(-40.0, 0.0, 0.0));
        let shooter = game.world.entity_from_id(shooter);
        // A prop: it has the components but not the tag.
        game.world
            .entity_named("scenery")
            .set(Position(Vec3::ZERO))
            .set(Health::full(20.0));

        fire(
            shooter.world(),
            shooter,
            Visual(EntityKind::Arrow),
            Flight {
                position: Vec3::ZERO,
                velocity: Vec3::ZERO,
                gravity: 0.0,
                seconds_left: 0.3,
                radius: 3.0,
            },
            Payload::new(3.0, 1.0),
        );
        game.advance(0.05, 1);
        assert_eq!(
            count_projectiles(&game),
            1,
            "the projectile hit something that is not a player"
        );
        let _ = Player::id();
    }
}
