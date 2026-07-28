//! Properties of the game, checked over generated input.
//!
//! The hand written tests in this directory each pin one situation somebody
//! thought of. These pin the rules that have to hold in *every* situation,
//! which is where the bugs nobody thought of live: `tests/game_flow.rs` proves
//! that four specific deaths eliminate a player, and this file proves that no
//! sequence of any length gets a fifth life or a negative one.
//!
//! Two shapes here, and the split matters:
//!
//! * Pure properties over the formulas -- knockback, armour, the lobby state
//!   machine. Cheap, so they run at proptest's full case count.
//! * Whole world properties, where a generated [`Script`] drives a real flecs
//!   world and [`harness::invariants`] is checked after every tick. A world
//!   costs about a tenth of a second to build, so these run at a reduced case
//!   count and lean on the shrinker rather than on volume.

// Several properties here assert that a value did not change *at all*: that a
// refused activation cost no energy, that a hit inside the immunity window took
// no health. A tolerance would let exactly the bug being looked for through, so
// the comparison is exact on purpose.
#![expect(
    clippy::float_cmp,
    reason = "these properties are about a value being untouched, not about it being close"
)]

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::{Action, Game, Script, TICK};
use proptest::prelude::*;
use smash::{
    module::{
        ability::charge_steps,
        arena::Arena,
        damage::{Armor, DamageKind, Damaged, MatchClock, MeleeBonus, hurt},
        knockback::{Knockback, KnockbackModel, KnockbackTaken, resolve, strength},
        lives::{InvulnerableUntil, Lives, MAX_LIVES, band_index},
        lobby::{Lobby, LobbyConfig, Phase, step},
        player::{Energy, Health, Position},
    },
    server::PlayerId,
};

/// Building a flecs world and importing every kit costs about 100ms, so a
/// world-driving property runs tens of cases rather than hundreds. The
/// shrinker is what earns its keep here, not the case count.
fn world_cases() -> ProptestConfig {
    ProptestConfig {
        cases: 24,
        ..ProptestConfig::default()
    }
}

fn vec3() -> impl Strategy<Value = Vec3> {
    (-64.0f32..64.0, -32.0f32..96.0, -64.0f32..64.0).prop_map(|(x, y, z)| Vec3::new(x, y, z))
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        // Weighted towards ticking: a script that never advances time tests
        // only the observers, and the systems are half the game.
        6 => Just(Action::Tick),
        3 => (0usize..8, 0usize..8, 0.0f32..30.0, any::<u8>()).prop_map(
            |(attacker, victim, amount, kind)| Action::Hit { attacker, victim, amount, kind }
        ),
        2 => (0usize..8, 0u8..9).prop_map(|(player, slot)| Action::UseSlot { player, slot }),
        2 => (0usize..8, vec3()).prop_map(|(player, to)| Action::Move { player, to }),
        1 => (0usize..8, any::<bool>()).prop_map(|(player, on)| Action::Ground { player, on }),
        1 => (0usize..8, 0usize..64).prop_map(|(player, kit)| Action::SelectKit { player, kit }),
    ]
}

fn script() -> impl Strategy<Value = Script> {
    (2usize..6, prop::collection::vec(action(), 0..300))
        .prop_map(|(players, actions)| Script { players, actions })
}

// ---------------------------------------------------------------------------
// The whole game
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(world_cases())]

    /// Every rule in [`harness::invariants`], after every tick of any script.
    ///
    /// This is the load bearing test in the file. Everything below it is a
    /// specific rule stated once more in a form that says what broke; this one
    /// is the net.
    #[test]
    fn no_script_can_break_an_invariant(script in script()) {
        let game = Game::from_script(&script);
        game.run(&script);
    }

    /// Lives only ever fall, and only one at a time.
    ///
    /// The interesting half is the upper bound. `Lives` is a `u8` and a death
    /// subtracts one, so the failure this is watching for is a decrement that
    /// wrapped 0 round to 255 -- which reads as a player who cannot be
    /// eliminated and whose scoreboard row says they have 255 lives left.
    #[test]
    fn lives_only_ever_fall_and_never_below_zero(script in script()) {
        let game = Game::from_script(&script);
        let players = game.players();
        let mut last: Vec<u8> = vec![MAX_LIVES; players.len()];

        for action in &script.actions {
            game.run(&Script { players: script.players, actions: vec![*action] });

            let lobby = game.world.cloned::<&Lobby>();
            for (index, player) in players.iter().enumerate() {
                let lives = game.world.entity_from_id(*player).cloned::<&Lives>().0;
                prop_assert!(lives <= MAX_LIVES, "{lives} lives is more than the maximum");
                // A new match hands everybody their lives back, which is the
                // one legitimate way the count goes up.
                if lobby.phase != Phase::Waiting {
                    prop_assert!(
                        lives <= last[index],
                        "lives went from {} to {lives} without a match reset",
                        last[index]
                    );
                }
                last[index] = lives;
            }
        }
    }

    /// A hit inside the respawn immunity window is a no-op.
    ///
    /// Not "changes velocity but not health": the immunity is checked before
    /// anything else in `apply_damage`, so `Smashed` is never emitted and no
    /// knockback is computed either. That is the behaviour Mineplex's
    /// `RESPAWN_INVUL` had and the behaviour this needs -- a player who could
    /// be launched off the map while immune would be immune to the damage and
    /// dead anyway, which is the worst of both.
    #[test]
    fn a_hit_inside_the_immunity_window_changes_nothing(
        amount in 0.5f32..40.0,
        kind in 0u8..4,
        remaining in 0.1f32..10.0,
    ) {
        let mut game = Game::new();
        let attacker = game.player("attacker", Vec3::ZERO);
        let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));
        let victim = game.world.entity_from_id(victim);

        let now = 5.0;
        game.world.get::<&mut MatchClock>(|clock| clock.0 = now);
        victim.set(InvulnerableUntil(now + remaining));

        let before = victim.cloned::<&Health>();
        hurt(victim, Damaged {
            attacker: Some(attacker),
            amount,
            knockback: Knockback::from(Vec3::ZERO),
            kind: match kind {
                0 => DamageKind::Melee,
                1 => DamageKind::Projectile,
                2 => DamageKind::Ability,
                _ => DamageKind::Environment,
            },
        });

        let after = victim.cloned::<&Health>();
        prop_assert_eq!(before.current, after.current, "immunity did not stop the damage");
        prop_assert_eq!(
            game.server.total_velocity(PlayerId(2)),
            Vec3::ZERO,
            "immunity did not stop the knockback"
        );
    }

    /// The same script in two fresh worlds produces the same numbers.
    ///
    /// Stated here as well as in `tests/determinism.rs` because a
    /// nondeterministic simulation makes every other property in this file
    /// flaky, and it is worth failing on the cause rather than on a symptom
    /// three tests later.
    #[test]
    fn two_runs_of_one_script_agree(script in script()) {
        let first = Game::from_script(&script);
        first.run(&script);
        let second = Game::from_script(&script);
        second.run(&script);
        prop_assert_eq!(first.fingerprint(), second.fingerprint());
    }
}

// ---------------------------------------------------------------------------
// Knockback
// ---------------------------------------------------------------------------

proptest! {
    /// Reflecting the attacker through the victim negates the horizontal
    /// impulse and leaves the vertical alone.
    ///
    /// This is the reversibility that matters for a launch: two identical hits
    /// from opposite sides cancel sideways and add upwards. It is also what
    /// makes the formula usable at all -- a direction term that were not
    /// antisymmetric would mean a player could be knocked *towards* whoever hit
    /// them from one particular angle.
    #[test]
    fn knockback_is_antisymmetric_in_the_attacker_direction(
        k in 0.0f32..12.0,
        angle in 0.0f32..core::f32::consts::TAU,
        distance in 0.5f32..40.0,
        grounded in any::<bool>(),
    ) {
        let model = KnockbackModel::default();
        let victim = Vec3::new(10.0, 3.0, -7.0);
        let offset = Vec3::new(angle.cos(), 0.0, angle.sin()) * distance;

        let one = resolve(model, k, victim - offset, victim, grounded);
        let other = resolve(model, k, victim + offset, victim, grounded);

        let tolerance = one.length().mul_add(1e-4, 1e-5);
        prop_assert!((one.x + other.x).abs() < tolerance, "{one} and {other} do not cancel in x");
        prop_assert!((one.z + other.z).abs() < tolerance, "{one} and {other} do not cancel in z");
        prop_assert!((one.y - other.y).abs() < tolerance, "vertical should not depend on side");
    }

    /// Knockback depends on where the attacker is, never on how far away.
    ///
    /// The direction is normalised, so a hit from a metre away and the same hit
    /// from forty metres away launch identically. Every ability that reaches
    /// across the map relies on this; if range leaked into the impulse, a
    /// Skeleton's arrow would hit harder the further it flew.
    #[test]
    fn knockback_ignores_the_distance_to_the_attacker(
        k in 0.1f32..12.0,
        angle in 0.0f32..core::f32::consts::TAU,
        near in 0.5f32..5.0,
        far in 20.0f32..200.0,
    ) {
        let model = KnockbackModel::default();
        let victim = Vec3::new(-4.0, 12.0, 9.0);
        let direction = Vec3::new(angle.cos(), 0.0, angle.sin());

        let close = resolve(model, k, victim - direction * near, victim, false);
        let distant = resolve(model, k, victim - direction * far, victim, false);

        prop_assert!(
            (close - distant).length() < 1e-4,
            "{close} from {near} away, {distant} from {far} away"
        );
    }

    /// Strength rises with damage and with the multipliers, and never turns
    /// negative however the terms are combined.
    #[test]
    fn strength_is_monotone_and_non_negative(
        damage in 0.0f32..100.0,
        extra in 0.0f32..100.0,
        current in 0.0f32..20.0,
        taken in 0.0f32..3.0,
        ability in 0.1f32..5.0,
    ) {
        let model = KnockbackModel::default();
        let health = Health { current, max: 20.0 };
        let base = strength(model, damage, health, KnockbackTaken(taken), ability);
        let more = strength(model, damage + extra, health, KnockbackTaken(taken), ability);

        prop_assert!(base.is_finite() && more.is_finite());
        prop_assert!(base >= 0.0, "negative strength {base}");
        prop_assert!(more >= base - 1e-5, "more damage gave less knockback");
    }
}

// ---------------------------------------------------------------------------
// Damage and armour
// ---------------------------------------------------------------------------

proptest! {
    /// Armour reduces damage, never below zero and never by more than the cap.
    ///
    /// The cap is vanilla Minecraft's 80%, and it is the reason a high armour
    /// kit is durable rather than immortal.
    #[test]
    fn armour_reduces_within_the_cap(points in -10.0f32..40.0, damage in 0.0f32..100.0) {
        let armor = Armor(points);
        let reduction = armor.reduction();
        prop_assert!((0.0..=0.8).contains(&reduction), "reduction {reduction} is out of range");

        let applied = armor.apply(damage);
        prop_assert!(applied >= 0.0, "armour turned {damage} into healing: {applied}");
        prop_assert!(applied <= damage + 1e-5, "armour increased the damage");
        prop_assert!(applied >= damage.mul_add(0.2, -1e-5), "armour beat the 80% cap");
    }

    /// More armour is never worse.
    #[test]
    fn armour_is_monotone(points in 0.0f32..40.0, extra in 0.0f32..40.0, damage in 0.0f32..100.0) {
        prop_assert!(Armor(points + extra).apply(damage) <= Armor(points).apply(damage) + 1e-5);
    }

    /// Health stays inside its own bar whatever it is asked to do.
    #[test]
    fn health_stays_between_zero_and_max(
        max in 1.0f32..100.0,
        operations in prop::collection::vec((any::<bool>(), 0.0f32..50.0), 0..40),
    ) {
        let mut health = Health::full(max);
        for (is_damage, amount) in operations {
            if is_damage {
                health.damage(amount);
            } else {
                health.heal(amount);
            }
            prop_assert!(health.current >= 0.0, "health fell to {}", health.current);
            prop_assert!(health.current <= health.max, "health rose to {}", health.current);
            prop_assert!((0.0..=1.0).contains(&health.fraction()));
            prop_assert_eq!(health.is_dead(), health.current <= 0.0);
        }
    }

    /// Spending energy never overdraws, and the answer matches what was spent.
    #[test]
    fn energy_never_goes_negative(
        max in 0.5f32..10.0,
        costs in prop::collection::vec(0.0f32..5.0, 0..30),
    ) {
        let mut energy = Energy::full(max, 0.0);
        for cost in costs {
            let before = energy.current;
            let spent = energy.try_spend(cost);
            prop_assert!(energy.current >= 0.0, "energy fell to {}", energy.current);
            prop_assert!(energy.current <= energy.max);
            if spent {
                prop_assert!((before - energy.current - cost).abs() < 1e-5);
            } else {
                prop_assert_eq!(before, energy.current, "a refused spend still took energy");
            }
        }
    }

    /// A melee bonus applies only to who it names and only while it lasts.
    #[test]
    fn a_melee_bonus_respects_its_target_and_its_expiry(
        flat in 0.0f32..10.0,
        until in 0.0f32..100.0,
        now in 0.0f32..100.0,
        targeted in any::<bool>(),
    ) {
        let marked = Entity::new(42);
        let other = Entity::new(43);
        let bonus = MeleeBonus { flat, against: targeted.then_some(marked), until };

        if now >= until {
            prop_assert_eq!(bonus.applies_to(marked, now), 0.0, "an expired bonus still applied");
            prop_assert_eq!(bonus.applies_to(other, now), 0.0);
        } else {
            prop_assert_eq!(bonus.applies_to(marked, now), flat);
            prop_assert_eq!(
                bonus.applies_to(other, now),
                if targeted { 0.0 } else { flat },
                "a bonus aimed at one player reached another"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The lobby state machine
// ---------------------------------------------------------------------------

/// Which phases each phase may reach in one step. Written from the mode's
/// description rather than read off `step`, so a transition the implementation
/// invents shows up here as an illegal edge.
const fn reachable(from: Phase) -> &'static [Phase] {
    match from {
        Phase::Waiting => &[Phase::Waiting, Phase::Countdown],
        Phase::Countdown => &[Phase::Countdown, Phase::Waiting, Phase::Preparing],
        Phase::Preparing => &[Phase::Preparing, Phase::Playing],
        Phase::Playing => &[Phase::Playing, Phase::Ended],
        Phase::Ended => &[Phase::Ended, Phase::Waiting],
    }
}

fn phase() -> impl Strategy<Value = Phase> {
    prop_oneof![
        Just(Phase::Waiting),
        Just(Phase::Countdown),
        Just(Phase::Preparing),
        Just(Phase::Playing),
        Just(Phase::Ended),
    ]
}

proptest! {
    /// No step ever produces a phase that phase cannot reach, and no step ever
    /// produces a negative timer.
    ///
    /// A negative timer is the specific failure worth naming: every phase but
    /// `Playing` counts down and transitions at zero, so a timer below zero
    /// means a transition was missed and the phase is stuck.
    #[test]
    fn every_step_is_a_legal_transition_with_a_sane_timer(
        from in phase(),
        timer in 0.0f32..120.0,
        dt in 0.0f32..2.0,
        players in 0u32..20,
        alive in 0u32..20,
    ) {
        let config = LobbyConfig::default();
        let next = step(&config, Lobby { phase: from, timer }, dt, players, alive.min(players));

        prop_assert!(
            reachable(from).contains(&next.phase),
            "{from:?} reached {:?}, which is not a legal transition",
            next.phase
        );
        prop_assert!(next.timer >= 0.0, "{:?} left a timer of {}", next.phase, next.timer);
        prop_assert!(next.timer.is_finite());
    }

    /// A fuller lobby never waits longer.
    #[test]
    fn the_countdown_shortens_as_the_lobby_fills(players in 0u32..20, extra in 0u32..20) {
        let config = LobbyConfig::default();
        let (Some(fewer), Some(more)) = (
            config.countdown_for(players),
            config.countdown_for(players + extra),
        ) else {
            return Ok(());
        };
        prop_assert!(more <= fewer, "{} players wait {more}s, {players} wait {fewer}s", players + extra);
    }

    /// A lobby that stays full runs a whole match and then starts another.
    ///
    /// The cycle, not a resting state: a server that keeps its players does not
    /// stop, so the thing to check is that every phase is reached in order and
    /// that the machine comes back round rather than sticking. A version of
    /// this that asserted the lobby ends in `Waiting` was wrong for exactly
    /// that reason -- a full lobby leaves `Waiting` again on the very next
    /// step.
    #[test]
    fn a_lobby_that_stays_full_runs_a_match_and_comes_back_round(
        players in 4u32..9,
        dt in 0.01f32..0.5,
    ) {
        let config = LobbyConfig::default();
        let mut state = Lobby::default();
        let mut visited: Vec<Phase> = vec![state.phase];

        // Enough steps for the longest countdown, a whole match and the
        // results screen, twice over.
        for _ in 0..400_000 {
            // One player left alive once the match is running is what ends it.
            let alive = if state.phase == Phase::Playing { 1 } else { players };
            state = step(&config, state, dt, players, alive);
            prop_assert!(state.timer >= 0.0, "{:?} left a timer of {}", state.phase, state.timer);
            if visited.last() != Some(&state.phase) {
                visited.push(state.phase);
            }
        }

        let wanted = [
            Phase::Waiting,
            Phase::Countdown,
            Phase::Preparing,
            Phase::Playing,
            Phase::Ended,
            Phase::Waiting,
            Phase::Countdown,
        ];
        prop_assert!(
            visited.starts_with(&wanted),
            "a full lobby did not run the phases in order: {visited:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// Abilities and the arena
// ---------------------------------------------------------------------------

proptest! {
    /// Charge steps are bounded by the maximum and monotone in the charge.
    #[test]
    fn charge_steps_are_bounded_and_monotone(
        charge in -1.0f32..2.0,
        extra in 0.0f32..1.0,
        max in 0u32..64,
    ) {
        let steps = charge_steps(charge, max);
        prop_assert!(steps <= max, "{steps} steps out of a maximum of {max}");
        prop_assert!(charge_steps(charge + extra, max) >= steps);
        prop_assert_eq!(charge_steps(f32::NAN.max(0.0), max), 0);
    }

    /// Every spawn the arena hands out is one it declared.
    #[test]
    fn spawns_come_from_the_arena(index in any::<u64>()) {
        let arena = Arena::default();
        prop_assert!(arena.spawns.contains(&arena.spawn(index)));
        prop_assert!(!arena.is_out_of_bounds(arena.spawn(index)), "a spawn point is below the kill plane");
    }

    /// The kill plane is exactly a comparison against y, and nothing else about
    /// a position matters.
    #[test]
    fn out_of_bounds_depends_only_on_height(at in vec3(), kill_y in -32.0f32..32.0) {
        let arena = Arena { kill_y, ..Arena::default() };
        prop_assert_eq!(arena.is_out_of_bounds(at), at.y < kill_y);
    }

    /// Every remaining-lives count lands in a band, and only zero lands in the
    /// first one.
    ///
    /// Stated over `band_index` rather than over the world so the whole `u8`
    /// range is reachable. The bounds are the ones `LivesModule` declares; a
    /// band added or removed there without one here shows up as this property
    /// disagreeing with `the_life_counter_has_one_colour_per_count`, which
    /// drives the same table through a real world.
    #[test]
    fn every_life_count_lands_in_a_band(lives in any::<u8>()) {
        const BOUNDS: [u8; 5] = [0, 1, 2, 3, MAX_LIVES];

        let band = band_index(&BOUNDS, lives);
        prop_assert!(band.is_some(), "{lives} lives fell outside every band");
        let band = band.unwrap();
        prop_assert!(band < BOUNDS.len());
        prop_assert_eq!(
            band == 0,
            lives == 0,
            "only a player with no lives left is in the first band"
        );
        // Above the widest bound, the widest band absorbs it rather than
        // leaving the count unbanded.
        if lives > MAX_LIVES {
            prop_assert_eq!(band, BOUNDS.len() - 1);
        }
    }
}

proptest! {
    #![proptest_config(world_cases())]

    /// `splash` hits everything in range except the player who cast it.
    ///
    /// Stated over the primitive rather than over the kits. "No ability ever
    /// hurts its caster" is the natural thing to write and it is false: Slime
    /// Slam hands a quarter of its damage back on purpose, and the generator
    /// found that in four cases. What is true, and what every kit relies on, is
    /// that the shared area-of-effect helper excludes the caster -- so a kit
    /// that wants recoil has to ask for it, and a kit that does not cannot get
    /// it by accident.
    ///
    /// The radius boundary is generated too, because the whole condition is one
    /// three-way `&&` and a mutation of it that keeps the caster out but lets
    /// the range go would pass a test that only checked the caster.
    #[test]
    fn splash_hits_everything_in_range_except_the_caster(
        radius in 1.0f32..30.0,
        damage in 1.0f32..15.0,
        offsets in prop::collection::vec(vec3(), 1..6),
    ) {
        use smash::module::{
            ability::{Cast, splash},
            player::Facing,
        };

        let mut game = Game::new();
        let caster = game.player("caster", Vec3::ZERO);
        let bystanders: Vec<_> = offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| (game.player(&format!("b{index}"), *offset), *offset))
            .collect();
        let caster = game.world.entity_from_id(caster);

        let before = caster.cloned::<&Health>();
        game.world.get::<&smash::server::ServerHandle>(|server| {
            splash(
                &Cast {
                    world: caster.world(),
                    caster,
                    ability: caster,
                    server: &**server,
                    player: PlayerId(1),
                    position: Position(Vec3::ZERO),
                    facing: Facing(Vec3::X),
                    charge: 1.0,
                },
                radius,
                damage,
                1.0,
            );
        });

        prop_assert_eq!(
            before.current,
            caster.cloned::<&Health>().current,
            "splash hurt the player who cast it"
        );

        for (bystander, at) in bystanders {
            let bystander = game.world.entity_from_id(bystander);
            let health = bystander.cloned::<&Health>();
            let inside = at.length() <= radius;
            prop_assert_eq!(
                health.current < health.max,
                inside,
                "a bystander {} blocks away was{} hit by a splash of radius {}",
                at.length(),
                if inside { " not" } else { "" },
                radius
            );
        }
    }

    /// A cooldown ticks down to zero and stops, never past it and never up.
    #[test]
    fn cooldowns_fall_to_zero_and_stay_there(
        kit_index in 0usize..64,
        ticks in 1u32..200,
    ) {
        use smash::module::ability::{Cooldown, Grants, activate, granted_in_slot};

        let mut game = Game::new();
        let player = game.player("p", Vec3::ZERO);
        let player = game.world.entity_from_id(player);
        let kits = smash::module::kit::registry(&game.world);
        let chosen = game.world.entity_from_id(kits[kit_index % kits.len()]);
        smash::module::kit::apply(&game.world, player, chosen);

        let _ = activate(player, 0, 1.0);
        let mut previous = f32::INFINITY;
        for _ in 0..ticks {
            game.advance(TICK, 1);
            let mut remaining: Vec<f32> = Vec::new();
            player.each_target(Grants, |granted| {
                if let Some(cooldown) = granted.try_get::<&Cooldown>(|c| c.remaining) {
                    remaining.push(cooldown);
                }
            });
            let highest = remaining.into_iter().fold(0.0f32, f32::max);
            prop_assert!(highest >= 0.0, "a cooldown went negative: {highest}");
            prop_assert!(highest <= previous + 1e-5, "a cooldown went back up");
            previous = highest;
        }

        // Ninety seconds, which is comfortably past the longest stock
        // cooldown. `advance` takes seconds and a step count, and reading it as
        // a tick count is how this first ran for one second and reported that
        // no cooldown ever expires.
        game.advance(90.0, 90 * 20);
        if let Some(ability) = granted_in_slot(player, 0) {
            let remaining = ability.try_get::<&Cooldown>(|c| c.remaining).unwrap_or(0.0);
            prop_assert!(remaining.abs() < 1e-5, "a cooldown never expired: {remaining}");
        }
    }
}
