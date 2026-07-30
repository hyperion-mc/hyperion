//! Scratch measurement: what does each ability draw? Not for merge.
mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::{Game, TICK};
use smash::{
    module::{
        ability::{self, Declared},
        kit::{self, Playing},
        player::{Energy, Health, OnGround, Position},
        projectile::Visual,
    },
    server::mock::Call,
};

fn arm(game: &Game, caster: Entity, entry: &Declared) {
    if !entry.ultimate {
        return;
    }
    let _ = kit::grant_ultimate(&game.world, game.world.entity_from_id(caster), 600.0);
}

#[test]
fn measure_visibility() {
    let manifest = ability::manifest(&Game::new().world);
    let mut rows = Vec::new();

    for entry in &manifest {
        let mut game = Game::new();
        let caster = game.player("caster", Vec3::ZERO);
        let near = game.player("near", Vec3::new(3.5, 0.0, 0.0));
        let world = &game.world;
        let chosen = kit::by_name(world, entry.kit).unwrap();
        for e in [caster, near] {
            kit::apply(world, world.entity_from_id(e), chosen);
        }
        for (e, at) in [(caster, Vec3::ZERO), (near, Vec3::new(3.5, 0.0, 0.0))] {
            let p = world.entity_from_id(e);
            p.set(Position(at));
            p.set(OnGround(true));
            p.get::<&mut Health>(|h| h.current = h.max);
            if let Some(mut en) = p.try_get::<&Energy>(|x| *x) {
                en.current = en.max;
                p.set(en);
            }
        }
        arm(&game, caster, entry);
        game.server.take();

        let cv = game.world.entity_from_id(caster);
        match entry.charge_time {
            Some(s) => {
                ability::use_slot(cv, entry.slot);
                game.advance(s, 8);
                ability::release_slot(cv, entry.slot);
            }
            None => ability::use_slot(cv, entry.slot),
        }
        game.advance(TICK, 1);

        // projectiles alive right after the press
        let mut projectiles = 0usize;
        game.world
            .query::<&Visual>()
            .build()
            .each(|_| projectiles += 1);

        game.advance(1.5, 30);

        let calls = game.server.calls();
        let particles = calls
            .iter()
            .filter(|c| matches!(c, Call::Particles(..)))
            .count();
        let status = calls.iter().filter(|c| matches!(c, Call::Status(..))).count();
        let teleport = calls
            .iter()
            .filter(|c| matches!(c, Call::Teleport(..)))
            .count();

        rows.push(format!(
            "VIS\t{}\t{}\tparticles={particles}\tprojectiles={projectiles}\tstatus={status}\tteleport={teleport}\tult={}",
            entry.kit, entry.name, entry.ultimate
        ));
    }

    for row in &rows {
        println!("{row}");
    }
    println!("VIS-TOTAL\t{}", rows.len());
}
