//! The damage-to-knockback pipeline, end to end through a real world.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        damage::{DamageKind, Damaged, hurt},
        knockback::{Knockback, KnockbackTaken},
        player::Health,
    },
    server::{PlayerId, mock::Call},
};

#[test]
fn a_hit_lowers_health_and_launches_the_victim_away() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: Some(attacker),
        amount: 6.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });

    let health = game.world.entity_from_id(victim).cloned::<&Health>();
    assert!(health.current < health.max, "the hit did not land");

    let impulse = game.server.total_velocity(PlayerId(2));
    assert!(
        impulse.x > 0.0,
        "victim was not launched away from the attacker"
    );
    assert_eq!(impulse.z, 0.0, "no sideways drift from a head-on hit");
    assert!(
        impulse.y > 0.0,
        "a grounded victim should be popped upwards"
    );
}

#[test]
fn the_same_hit_launches_a_hurt_victim_further() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let fresh = game.player("fresh", Vec3::new(4.0, 0.0, 0.0));
    let hurt_player = game.player("hurt", Vec3::new(4.0, 0.0, 0.0));

    game.world.entity_from_id(hurt_player).set(Health {
        current: 4.0,
        max: 20.0,
    });

    let hit = Damaged {
        attacker: Some(attacker),
        amount: 6.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    };

    hurt(game.world.entity_from_id(fresh), hit);
    hurt(game.world.entity_from_id(hurt_player), hit);

    let fresh_impulse = game.server.total_velocity(PlayerId(2)).length();
    let hurt_impulse = game.server.total_velocity(PlayerId(3)).length();

    assert!(
        hurt_impulse > fresh_impulse * 1.5,
        "low health must matter a lot: fresh {fresh_impulse}, hurt {hurt_impulse}"
    );
}

#[test]
fn kill_credit_is_recorded_as_a_relationship() {
    use smash::module::damage::LastHitBy;

    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: Some(attacker),
        amount: 3.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });

    assert!(
        game.world.entity_from_id(victim).has((LastHitBy, attacker)),
        "the victim does not remember who hit them"
    );
}

#[test]
fn a_heavier_kit_takes_less_knockback() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let light = game.player("light", Vec3::new(4.0, 0.0, 0.0));
    let heavy = game.player("heavy", Vec3::new(4.0, 0.0, 0.0));

    game.world.entity_from_id(light).set(KnockbackTaken(1.5));
    game.world.entity_from_id(heavy).set(KnockbackTaken(1.0));

    for victim in [light, heavy] {
        hurt(game.world.entity_from_id(victim), Damaged {
            attacker: Some(attacker),
            amount: 6.0,
            knockback: Knockback::from(Vec3::ZERO),
            kind: DamageKind::Melee,
        });
    }

    assert!(
        game.server.total_velocity(PlayerId(2)).length()
            > game.server.total_velocity(PlayerId(3)).length()
    );
}

#[test]
fn environmental_damage_ignores_armour() {
    use smash::module::damage::Armor;

    let mut game = Game::new();
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));
    game.world.entity_from_id(victim).set(Armor(16.0));

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: None,
        amount: 1.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Environment,
    });

    let health = game.world.entity_from_id(victim).cloned::<&Health>();
    assert_eq!(health.current, 19.0, "hunger must be true damage");

    assert!(
        game.server
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SetHealth(_, _, _))),
        "the client was never told about the new health"
    );
}
