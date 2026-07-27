//! Lives, death, respawn, elimination, and the scoreboard that reports them.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::{
    module::{
        arena::Arena,
        damage::{DamageKind, Damaged, LastHitAt, MatchClock, hurt},
        kit,
        knockback::Knockback,
        lives::{DeathCause, Eliminated, Lives, MAX_LIVES, Placement, RespawnAt, kill, killer_of},
        lobby::Phase,
        player::{Health, Position},
        scoreboard::{COLLAPSE_ABOVE, Row, render},
    },
    server::{PlayerId, mock::Call},
};

#[test]
fn everyone_starts_with_four_lives() {
    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    assert_eq!(
        game.world.entity_from_id(player).cloned::<&Lives>().0,
        MAX_LIVES
    );
    assert_eq!(MAX_LIVES, 4, "Mineplex's MAX_LIVES");
}

#[test]
fn falling_out_of_bounds_costs_a_life_and_schedules_a_respawn() {
    use smash::module::lobby::Lobby;

    let mut game = Game::new();
    let player = game.player("faller", Vec3::new(0.0, 40.0, 0.0));
    let player = game.world.entity_from_id(player);

    // The void check only runs during a match. In the hub a player standing
    // below a kill plane would otherwise die on the tick they connect.
    game.world.set(Lobby {
        phase: Phase::Playing,
        timer: 1.0,
    });

    let kill_y = game.world.cloned::<&Arena>().kill_y;
    player.set(Position(Vec3::new(0.0, kill_y - 5.0, 0.0)));
    game.advance(0.05, 1);

    assert_eq!(player.cloned::<&Lives>().0, MAX_LIVES - 1);
    assert!(player.try_get::<&RespawnAt>(|r| r.0).is_some());
    assert!(
        game.server
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Spectating(_, true))),
        "a dead player should be spectating while they wait"
    );
    assert!(
        game.server
            .broadcasts()
            .iter()
            .any(|line| line.contains("out of bounds")),
        "nobody was told: {:?}",
        game.server.broadcasts()
    );
}

#[test]
fn a_respawn_restores_health_and_puts_you_back_on_a_spawn_point() {
    let mut game = Game::new();
    let player = game.player("faller", Vec3::new(0.0, 40.0, 0.0));
    let player = game.world.entity_from_id(player);
    let spawns = game.world.cloned::<&Arena>().spawns;

    kill(player, DeathCause::Void);
    assert!(player.cloned::<&Health>().current.abs() < 1e-6);

    // The respawn is gated on the match clock, which only advances while the
    // lobby says the game is running.
    game.world.get::<&mut MatchClock>(|clock| clock.0 = 10.0);
    game.advance(0.05, 1);

    let health = player.cloned::<&Health>();
    assert!(
        (health.current - health.max).abs() < 1e-6,
        "respawned at full health"
    );
    assert!(player.try_get::<&RespawnAt>(|r| r.0).is_none());
    assert!(
        spawns.contains(&player.cloned::<&Position>().0),
        "respawned somewhere that is not a spawn point"
    );
    assert!(
        game.server
            .calls()
            .iter()
            .any(|call| matches!(call, Call::Spectating(_, false))),
        "never taken out of spectator"
    );
}

#[test]
fn losing_the_last_life_eliminates_you_permanently() {
    let mut game = Game::new();
    let player = game.player("doomed", Vec3::ZERO);
    let other = game.player("survivor", Vec3::new(30.0, 0.0, 0.0));
    let player = game.world.entity_from_id(player);
    let _ = other;

    for remaining in (0..MAX_LIVES).rev() {
        kill(player, DeathCause::Void);
        assert_eq!(player.cloned::<&Lives>().0, remaining);
        player.remove(RespawnAt::id());
        player.get::<&mut Health>(|health| health.current = health.max);
    }

    assert!(player.has(Eliminated::id()));
    assert!(player.try_get::<&Placement>(|p| p.0).is_some());
    assert!(
        game.server
            .messages_to(PlayerId(1))
            .iter()
            .any(|line| line.contains("GAME OVER")),
        "{:?}",
        game.server.messages_to(PlayerId(1))
    );

    // Further deaths do nothing.
    let placement = player.cloned::<&Placement>();
    kill(player, DeathCause::Void);
    assert_eq!(player.cloned::<&Lives>().0, 0);
    assert_eq!(player.cloned::<&Placement>(), placement);
}

#[test]
fn a_void_death_is_credited_to_whoever_hit_you_last() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));

    hurt(game.world.entity_from_id(victim), Damaged {
        attacker: Some(attacker),
        amount: 5.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });

    kill(game.world.entity_from_id(victim), DeathCause::Void);

    assert!(
        game.server
            .broadcasts()
            .iter()
            .any(|line| line.contains("smashed by attacker")),
        "{:?}",
        game.server.broadcasts()
    );
}

#[test]
fn kill_credit_expires() {
    use smash::module::damage::KILL_CREDIT_WINDOW;

    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));
    let victim = game.world.entity_from_id(victim);

    hurt(victim, Damaged {
        attacker: Some(attacker),
        amount: 5.0,
        knockback: Knockback::from(Vec3::ZERO),
        kind: DamageKind::Melee,
    });

    let hit_at = victim.cloned::<&LastHitAt>().0;
    assert!(killer_of(victim, hit_at + KILL_CREDIT_WINDOW - 0.1).is_some());
    assert!(
        killer_of(victim, hit_at + KILL_CREDIT_WINDOW + 0.1).is_none(),
        "an old hit should not still be worth a kill"
    );
}

#[test]
fn the_last_hit_is_the_one_that_counts() {
    let mut game = Game::new();
    let first = game.player("first", Vec3::ZERO);
    let second = game.player("second", Vec3::new(-4.0, 0.0, 0.0));
    let victim = game.player("victim", Vec3::new(4.0, 0.0, 0.0));
    let victim = game.world.entity_from_id(victim);

    for attacker in [first, second] {
        hurt(victim, Damaged {
            attacker: Some(attacker),
            amount: 2.0,
            knockback: Knockback::from(Vec3::ZERO),
            kind: DamageKind::Melee,
        });
    }

    assert_eq!(
        killer_of(victim, victim.cloned::<&LastHitAt>().0),
        Some(second),
        "LastHitBy is exclusive, so the newer hit replaced the older"
    );
}

#[test]
fn the_scoreboard_sorts_by_lives_and_colours_the_last_one_red() {
    let rows = vec![
        Row {
            name: "low".to_owned(),
            lives: 1,
            colour: Lives(1).colour(),
        },
        Row {
            name: "high".to_owned(),
            lives: 4,
            colour: Lives(4).colour(),
        },
    ];
    let lines = render(Phase::Playing, rows);
    assert!(lines[0].contains("high"), "{lines:?}");
    assert!(
        lines[1].contains("red"),
        "a last life must read as red: {lines:?}"
    );
}

#[test]
fn a_big_lobby_collapses_to_two_counters() {
    let rows: Vec<Row> = (0..=COLLAPSE_ABOVE)
        .map(|index| Row {
            name: format!("p{index}"),
            lives: u8::from(index % 2 == 0),
            colour: "green",
        })
        .collect();

    let lines = render(Phase::Playing, rows);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(lines[0].starts_with("Players Alive: "));
    assert!(lines[1].starts_with("Players Dead: "));
}

#[test]
fn picking_a_kit_in_the_hub_pushes_a_hotbar_and_picking_one_mid_match_is_refused() {
    use smash::module::lobby::{Lobby, select_kit};

    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    assert!(select_kit(&game.world, player, "Skeleton").is_ok());
    assert!(
        game.server
            .calls()
            .iter()
            .any(|call| matches!(call, Call::SetHotbar(_, items) if !items.is_empty())),
        "no hotbar was sent"
    );
    assert!(select_kit(&game.world, player, "Nonexistent").is_err());

    game.world.set(Lobby {
        phase: Phase::Playing,
        timer: 1.0,
    });
    assert!(
        select_kit(&game.world, player, "Slime").is_err(),
        "kits must be locked once the match starts"
    );
}

#[test]
fn switching_kit_replaces_the_old_abilities_rather_than_stacking_them() {
    use smash::module::ability::Grants;

    let mut game = Game::new();
    let player = game.player("p", Vec3::ZERO);
    let player = game.world.entity_from_id(player);

    let count_granted = || {
        let mut n = 0;
        player.each_target(Grants, |_| n += 1);
        n
    };

    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Skeleton").unwrap(),
    );
    let skeleton_abilities = count_granted();
    assert!(skeleton_abilities > 0);

    kit::apply(
        &game.world,
        player,
        kit::by_name(&game.world, "Iron Golem").unwrap(),
    );
    assert_eq!(
        count_granted(),
        3,
        "Iron Golem has three starting abilities and no leftovers from Skeleton"
    );
}
