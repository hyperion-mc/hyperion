//! Every game system sits directly under its module, with no phantom node.
//!
//! `world.module::<Self>("smash::Ability")` sets the module scope to
//! `smash.Ability`; a `system_named("smash::tick_cooldowns")` then resolves the
//! `smash::` prefix *relative to that scope* and creates a
//! `smash.Ability.smash.tick_cooldowns` path -- a phantom `smash` node the flecs
//! explorer draws as a real tree row that corresponds to nothing. The fix is to
//! name a system by its leaf alone and let the module scope supply the rest.
//!
//! This test is the guard. It fails if any module grows a child literally named
//! `smash`, which is what a re-introduced prefix produces, and it pins the
//! handful of clean paths so a reader can see the intended shape.

use flecs_ecs::prelude::*;
use smash::SmashModule;

/// No module in the game tree has a child named `smash`.
///
/// The only legitimate `smash` entity is the game module itself, at `::smash`.
/// Anything else called `smash` is a redundant-prefix phantom.
#[test]
fn no_phantom_smash_scope() {
    let world = World::new();
    world.import::<SmashModule>();

    let mut phantoms = Vec::new();
    world
        .query::<()>()
        .with((id::<flecs::ChildOf>(), id::<flecs::Wildcard>()))
        .build()
        .each_entity(|entity, ()| {
            if entity.name() == "smash" && entity.path().as_deref() != Some("::smash") {
                phantoms.push(entity.path().unwrap_or_default());
            }
        });

    assert!(
        phantoms.is_empty(),
        "a redundant `smash::` prefix put a phantom node in the module tree: {phantoms:?}"
    );
}

/// A representative system from each layer resolves at the clean path, and the
/// phantom spelling it used to have resolves to nothing.
#[test]
fn systems_sit_directly_under_their_module() {
    let world = World::new();
    world.import::<SmashModule>();

    for clean in [
        "smash::Ability::tick_cooldowns",
        "smash::Effect::tick_effects",
        "smash::Damage::apply_damage",
        "smash::Lives::respawn",
        "smash::Knockback::apply_knockback",
        "smash::Scoreboard::spectate_on_elimination",
        "smash::kits::Guardian::laser_expiry",
        "smash::kits::Creeper::arm",
        "smash::kits::Wolf::ravage",
    ] {
        assert!(
            world.try_lookup(clean).is_some(),
            "expected a system at `{clean}`, found none"
        );
    }

    for phantom in [
        "smash::Ability::smash::tick_cooldowns",
        "smash::Effect::smash::tick_effects",
    ] {
        assert!(
            world.try_lookup(phantom).is_none(),
            "the phantom path `{phantom}` still resolves"
        );
    }
}
