//! The knockback formula, checked against Mineplex's own source.
//!
//! The consolidated pipeline being tested, from `SuperSmash.java:533`,
//! `DamageManager.java:518-560` and `UtilAction.velocity`:
//!
//! ```text
//! K              = log10(max(damage, 2))
//!                  * (1 + 0.1 * (maxHealth - health))
//!                  * kitKnockbackTaken
//!                  * abilityMultiplier
//! horizontal     = 0.2 + 0.8 * (0.6 * K)
//! vertical       = min(0.2 * K, 0.4 + 0.04 * K) + (grounded ? 0.2 : 0)
//! ```

use glam::Vec3;
use proptest::prelude::*;
use smash::module::{
    knockback::{KnockbackModel, KnockbackTaken, resolve, strength, vanilla},
    player::Health,
};

const EPS: f32 = 1e-4;

const fn health(current: f32) -> Health {
    Health { current, max: 20.0 }
}

/// The reference implementation, written straight from the Java and kept
/// separate from the one under test so a transcription slip in either shows up.
fn reference_strength(damage: f32, current: f32, taken: f32, ability: f32) -> f32 {
    damage.max(2.0).log10() * 0.1f32.mul_add(20.0 - current, 1.0) * taken * ability
}

#[test]
fn strength_matches_the_java_pipeline() {
    // damage, health, kit knockback taken, ability multiplier
    let cases = [
        (6.0, 20.0, 1.00, 1.0),
        (6.0, 20.0, 1.50, 1.0),
        (6.0, 10.0, 1.50, 1.0),
        (6.0, 1.0, 1.50, 1.0),
        (6.0, 6.0, 1.25, 2.5),  // Skeleton hit by Bone Explosion
        (1.0, 20.0, 1.75, 1.0), // below the damage floor
        (20.0, 3.0, 1.65, 2.5), // Creeper's Explode on a near-dead target
    ];

    for (damage, current, taken, ability) in cases {
        let got = strength(
            KnockbackModel::default(),
            damage,
            health(current),
            KnockbackTaken(taken),
            ability,
        );
        let want = reference_strength(damage, current, taken, ability);
        assert!(
            (got - want).abs() < EPS,
            "damage={damage} health={current} taken={taken} ability={ability}: got {got}, want \
             {want}"
        );
    }
}

/// The worked example from the research: a six-damage melee hit on a kit that
/// takes 150% knockback, at three health levels.
#[test]
fn the_documented_worked_example_reproduces() {
    let model = KnockbackModel::default();
    let cases = [(20.0, 1.17, 0.76), (10.0, 2.33, 1.32), (1.0, 3.38, 1.82)];

    for (current, want_k, want_horizontal) in cases {
        let k = strength(model, 6.0, health(current), KnockbackTaken(1.5), 1.0);
        assert!(
            (k - want_k).abs() < 0.01,
            "health {current}: strength {k}, expected {want_k}"
        );

        let impulse = resolve(model, k, Vec3::ZERO, Vec3::new(4.0, 0.0, 0.0), false);
        let horizontal = Vec3::new(impulse.x, 0.0, impulse.z).length();
        assert!(
            (horizontal - want_horizontal).abs() < 0.01,
            "health {current}: horizontal {horizontal}, expected {want_horizontal}"
        );
    }
}

/// Launch angle rises with strength up to the point the vertical cap binds,
/// then falls away for the rest of the range.
///
/// The cap is `0.4 + 0.04 * K` against an uncapped `0.2 * K`, so it starts
/// binding at `K = 2.5`. Below that a harder hit pops you higher; above it a
/// harder hit only pushes you further sideways, which is what turns a chip
/// damaged player into a player who dies to the void rather than to the
/// ceiling. Both halves are the original's, not a choice made here.
#[test]
fn launch_angle_peaks_where_the_vertical_cap_starts_binding() {
    let model = KnockbackModel::default();
    let ratio = |k: f32| {
        let v = resolve(model, k, Vec3::ZERO, Vec3::X, false);
        v.y / Vec3::new(v.x, 0.0, v.z).length()
    };

    let knee =
        model.vertical_cap_base / (model.vertical_per_strength - model.vertical_cap_per_strength);
    assert!((knee - 2.5).abs() < 1e-4, "the knee should be at K = 2.5");

    for pair in [1.0f32, 1.5, 2.0, knee].windows(2) {
        assert!(
            ratio(pair[1]) > ratio(pair[0]),
            "below the knee a harder hit should launch higher: {} gives {:.3}, {} gives {:.3}",
            pair[0],
            ratio(pair[0]),
            pair[1],
            ratio(pair[1])
        );
    }

    for pair in [knee, 3.0, 5.0, 8.0, 13.0, 21.0].windows(2) {
        assert!(
            ratio(pair[1]) < ratio(pair[0]),
            "above the knee a harder hit should be flatter: {} gives {:.3}, {} gives {:.3}",
            pair[0],
            ratio(pair[0]),
            pair[1],
            ratio(pair[1])
        );
    }

    // Asymptotically the angle tends to 0.04 / 0.48.
    let asymptote =
        model.vertical_cap_per_strength / (model.speed_per_length * model.trajectory_scale);
    assert!(ratio(1e5) > asymptote);
    assert!(ratio(1e5) - asymptote < 0.01);
}

#[test]
fn the_vertical_cap_is_the_binding_constraint_at_high_strength() {
    let model = KnockbackModel::default();
    for k in [3.0f32, 5.0, 10.0] {
        let impulse = resolve(model, k, Vec3::ZERO, Vec3::X, false);
        let cap = model
            .vertical_cap_per_strength
            .mul_add(k, model.vertical_cap_base);
        assert!(
            (impulse.y - cap).abs() < EPS,
            "strength {k}: vertical {} should equal the cap {cap}",
            impulse.y
        );
    }
}

#[test]
fn a_grounded_victim_gets_the_extra_lift() {
    let model = KnockbackModel::default();
    let airborne = resolve(model, 2.0, Vec3::ZERO, Vec3::X, false);
    let grounded = resolve(model, 2.0, Vec3::ZERO, Vec3::X, true);
    assert!((grounded.y - airborne.y - model.ground_boost).abs() < EPS);
    assert!(
        (grounded.x - airborne.x).abs() < EPS,
        "lift is vertical only"
    );
}

/// An attacker directly above a victim still launches them sideways, because
/// the trajectory is flattened before it is used.
#[test]
fn a_hit_from_directly_above_does_not_drive_the_victim_downwards() {
    let model = KnockbackModel::default();
    let impulse = resolve(
        model,
        2.0,
        Vec3::new(0.0, 10.0, 0.0),
        Vec3::new(1.0, 0.0, 0.0),
        false,
    );
    assert!(impulse.y > 0.0, "never launched downwards");
    assert!(impulse.x > 0.0, "still pushed away horizontally");
}

/// Exactly co-located attacker and victim have no direction to launch along.
/// Returning zero beats returning a NaN that silently poisons a velocity.
#[test]
fn a_degenerate_direction_is_zero_not_nan() {
    let impulse = resolve(KnockbackModel::default(), 2.0, Vec3::ZERO, Vec3::ZERO, true);
    assert_eq!(impulse, Vec3::ZERO);
    assert!(impulse.is_finite());
}

/// Vanilla never looks at the victim; Super Smash Mobs does almost nothing
/// else. This pins the difference the design doc claims.
#[test]
fn vanilla_ignores_the_victim_and_smash_does_not() {
    let model = KnockbackModel::default();
    let a = vanilla(Vec3::ZERO, Vec3::X, 0);
    let b = vanilla(Vec3::ZERO, Vec3::X, 0);
    assert_eq!(a, b);

    let fresh = strength(model, 6.0, health(20.0), KnockbackTaken(1.0), 1.0);
    let hurt = strength(model, 6.0, health(1.0), KnockbackTaken(1.0), 1.0);
    assert!(hurt > fresh * 2.5, "fresh {fresh}, hurt {hurt}");
}

proptest! {
    /// Whatever the inputs, a hit never pulls the victim towards the attacker
    /// and never produces a value the physics cannot use.
    #[test]
    fn knockback_is_always_finite_and_away_from_the_attacker(
        damage in 0.0f32..100.0,
        current in 0.0f32..20.0,
        taken in 0.5f32..3.0,
        ability in 0.5f32..5.0,
        angle in 0.0f32..core::f32::consts::TAU,
        grounded in any::<bool>(),
    ) {
        let model = KnockbackModel::default();
        let k = strength(model, damage, health(current), KnockbackTaken(taken), ability);
        let offset = Vec3::new(angle.cos(), 0.0, angle.sin()) * 4.0;
        let impulse = resolve(model, k, Vec3::ZERO, offset, grounded);

        prop_assert!(impulse.is_finite());
        prop_assert!(impulse.y >= 0.0, "never launched downwards");
        let along = Vec3::new(impulse.x, 0.0, impulse.z).dot(offset.normalize());
        prop_assert!(along > 0.0, "horizontal component points away from the attacker");
    }

    /// Losing health can only ever make you easier to launch.
    #[test]
    fn knockback_is_monotone_in_missing_health(
        damage in 2.0f32..40.0,
        hurt_by in 0.1f32..19.0,
        taken in 0.5f32..3.0,
    ) {
        let model = KnockbackModel::default();
        let full = strength(model, damage, health(20.0), KnockbackTaken(taken), 1.0);
        let hurt = strength(model, damage, health(20.0 - hurt_by), KnockbackTaken(taken), 1.0);
        prop_assert!(hurt >= full);
    }

    /// The multipliers compose multiplicatively, which is what lets a 150%
    /// kit and a 2.5x ability stack into the 3.75x that actually kills.
    #[test]
    fn multipliers_compose_multiplicatively(
        damage in 2.0f32..40.0,
        current in 0.0f32..20.0,
        taken in 0.5f32..3.0,
        ability in 0.5f32..5.0,
    ) {
        let model = KnockbackModel::default();
        let base = strength(model, damage, health(current), KnockbackTaken(1.0), 1.0);
        let both = strength(model, damage, health(current), KnockbackTaken(taken), ability);
        let want = base * taken * ability;
        prop_assert!((both - want).abs() < want.abs().mul_add(1e-3, 1e-6));
    }
}
