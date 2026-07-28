//! `(OnInstantiate, Inherit)` on an ability's static data, and `(Grants,
//! OnDeleteTarget, Remove)` on the grant edge.
//!
//! Inherit means a player's ability instance reads the kit prefab's `Named`,
//! `Item`, ... through its `IsA` edge rather than carrying a copy; only the
//! per-player `Cooldown` is owned. The grant-edge trait means destroying an
//! ability entity removes the `(Grants, ability)` edge from whoever held it,
//! which is what lets `expire` and `kit::revoke` destroy an ability without
//! unlinking it by hand first.

mod harness;

use flecs_ecs::prelude::*;
use glam::Vec3;
use harness::Game;
use smash::module::{
    ability::{Cooldown, Grants, Named},
    kit,
};

/// The ten static components inherit; the per-player `Cooldown` does not.
#[test]
fn static_ability_data_is_declared_inheritable() {
    let game = Game::new();
    let inherit = (id::<flecs::OnInstantiate>(), id::<flecs::Inherit>());
    for name in [
        "Slot",
        "Item",
        "Named",
        "Description",
        "CooldownSpec",
        "EnergyCost",
        "OnActivate",
        "OnRelease",
        "ChargeTime",
        "Proves",
    ] {
        let component = game
            .world
            .try_lookup(&format!("smash::Ability::{name}"))
            .unwrap_or_else(|| panic!("{name} is registered"));
        assert!(
            component.has(inherit),
            "{name} is not (OnInstantiate, Inherit)"
        );
    }
    assert!(
        !game.world.component::<Cooldown>().has(inherit),
        "Cooldown must stay per-player (Override), not inherited"
    );
}

/// The grant edge is declared `(OnDeleteTarget, Remove)`. This is flecs'
/// default, but declaring it is what documents the reliance -- and this
/// structural check fails if someone changes it to a harmful policy like
/// `Delete` (which would kill the holder) or drops it.
#[test]
fn grants_edge_is_declared_remove_on_delete() {
    let game = Game::new();
    assert!(
        game.world
            .component::<Grants>()
            .has((id::<flecs::OnDeleteTarget>(), id::<flecs::Remove>())),
        "Grants lost its explicit (OnDeleteTarget, Remove) declaration"
    );
}

/// A player's ability instance inherits its declaration and owns only its
/// cooldown. `owns` is the direct question: is the component on this entity, or
/// resolved through `IsA`?
#[test]
fn an_instance_inherits_its_declaration_and_owns_its_cooldown() {
    let mut game = Game::new();
    let player = game.player("player", Vec3::ZERO);
    let blaze = kit::by_name(&game.world, "Blaze").expect("the registry has Blaze");
    kit::apply(&game.world, game.world.entity_from_id(player), blaze);

    let instance = game
        .world
        .entity_from_id(player)
        .target(Grants, 0)
        .expect("the kit granted an ability");

    assert!(
        !instance.owns(id::<Named>()),
        "the instance carries its own Named copy instead of inheriting it"
    );
    assert!(
        instance.try_get::<&Named>(|n| !n.0.is_empty()) == Some(true),
        "the instance cannot read its inherited Named"
    );
    assert!(
        instance.owns(id::<Cooldown>()),
        "the instance does not own its per-player Cooldown"
    );
}

/// Destroying an ability removes the `(Grants, ability)` edge from its holder,
/// so `expire`/`revoke` can drop the manual unlink. The player survives.
#[test]
fn destroying_an_ability_removes_its_grant_edge() {
    let mut game = Game::new();
    let player = game.player("player", Vec3::ZERO);
    let blaze = kit::by_name(&game.world, "Blaze").expect("the registry has Blaze");
    kit::apply(&game.world, game.world.entity_from_id(player), blaze);

    let count = |g: &Game| {
        let mut n = 0;
        g.world
            .entity_from_id(player)
            .each_target(Grants, |_| n += 1);
        n
    };
    let before = count(&game);
    assert!(before > 0, "the kit granted no abilities");

    let victim = game
        .world
        .entity_from_id(player)
        .target(Grants, 0)
        .expect("an ability to destroy")
        .id();
    game.world.entity_from_id(victim).destruct();

    assert!(
        game.world.entity_from_id(player).is_alive(),
        "the player died with the ability"
    );
    assert_eq!(
        count(&game),
        before - 1,
        "the grant edge to the destroyed ability was not removed"
    );
}
