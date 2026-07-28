//! `effect::on`/`from_source` return the effects on *one* victim.
//!
//! The lookup is a direct `(Upon, victim)` query rather than a scan of every
//! effect, so this pins the property that would break if the victim term were
//! ever dropped: effects on one player must not show up in another's lookup.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::module::effect::{self, Affliction, Blame};

#[test]
fn effects_are_looked_up_per_victim() {
    let mut game = Game::new();
    let attacker = game.player("attacker", Vec3::ZERO);
    let v1 = game.player("v1", Vec3::new(1.0, 0.0, 0.0));
    let v2 = game.player("v2", Vec3::new(2.0, 0.0, 0.0));
    let world = (&game.world).into();

    // Two distinct-source afflictions on v1 (distinct sources so they coexist
    // rather than replacing one another), one on v2.
    for _ in 0..2 {
        let source = game.world.entity().id();
        effect::afflict(
            world,
            game.world.entity_from_id(v1),
            Blame { source, attacker },
            Affliction::shield(9.0),
        );
    }
    effect::afflict(
        world,
        game.world.entity_from_id(v2),
        Blame {
            source: attacker,
            attacker,
        },
        Affliction::shield(9.0),
    );

    assert_eq!(
        effect::on(world, v1).len(),
        2,
        "v1's afflictions were miscounted"
    );
    assert_eq!(
        effect::on(world, v2).len(),
        1,
        "v2's lookup returned effects that are not on v2 -- the victim term was lost"
    );
}
