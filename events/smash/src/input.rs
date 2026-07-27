//! hyperion's packet events, turned into the game's entry points.
//!
//! Everything here is a translation. No rule about what an ability does or how
//! much a swing hurts lives in this file; it decides only which of the game's
//! public functions a given packet means.

use flecs_ecs::prelude::*;
use hyperion::{
    simulation::{Uuid, event, metadata::living_entity::Health as HyperionHealth},
    storage::EventQueue,
};
use hyperion_inventory::PlayerInventory;

use crate::{
    adapter::player_id,
    flecs_ext::EntityViewExt,
    module::{
        ability,
        arena::Arena,
        damage::{self, DamageKind, Damaged},
        kit::{self, KitStats, Playing},
        knockback::Knockback,
        player::{Player, Position},
    },
    server::PlayerId,
};

/// Set when a player's kit changes and cleared once their hotbar has been
/// rebuilt from it.
#[derive(Component)]
struct HotbarStale;

#[derive(Component)]
pub struct InputModule;

impl Module for InputModule {
    fn module(world: &World) {
        // hyperion's join path reads `Position` with a hard `get`, so a player
        // with none never reaches the Login packet and sits on "Joining
        // world..." while the proxy happily reports them connected. Handing out
        // an arena spawn point before the join runs is what makes the game
        // joinable at all.
        world
            .observer::<flecs::OnSet, &Arena>()
            .with(id::<Uuid>())
            .without(id::<hyperion::simulation::Position>())
            .each_entity(|entity, arena| {
                entity.set(hyperion::simulation::Position::from(arena.spawn(*entity.id())));
            });

        // A hyperion player becomes a smash player. `Player` carries `With`
        // traits for every mirrored component, so this one add is what gives the
        // entity its Position, Facing, Health and the rest.
        world
            .observer::<flecs::OnAdd, ()>()
            .with(id::<hyperion::simulation::Player>())
            .each_entity(|entity, ()| {
                entity.set(player_id(entity.id())).add(Player::id());
            });

        world
            .system_named::<&mut EventQueue<event::ItemInteract>>("smash::on_item_interact")
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event in queue.drain() {
                    let player = world.entity_from_id(event.entity);
                    if let Some(slot) = held_slot(player) {
                        ability::use_slot(player, slot);
                    }
                }
            });

        world
            .system_named::<&mut EventQueue<event::ReleaseUseItem>>("smash::on_release_use_item")
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event in queue.drain() {
                    let player = world.entity_from_id(event.from);
                    if let Some(slot) = held_slot(player) {
                        ability::release_slot(player, slot);
                    }
                }
            });

        // Mineplex replaced the weapon's damage outright with the kit's, so what
        // the attacker is holding never enters into it -- which is also why
        // hyperion's own `damage` field on the event is ignored here.
        world
            .system_named::<&mut EventQueue<event::AttackEntity>>("smash::on_attack")
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event in queue.drain() {
                    let attacker = world.entity_from_id(event.origin);
                    let victim = world.entity_from_id(event.target);
                    if !attacker.has(Player::id()) || !victim.has(Player::id()) {
                        continue;
                    }
                    let Some(origin) = attacker.try_get::<&Position>(|position| position.0) else {
                        continue;
                    };
                    let amount = melee_damage(attacker);

                    damage::hurt(victim, Damaged {
                        attacker: Some(attacker.id()),
                        amount,
                        knockback: Knockback::from(origin),
                        kind: DamageKind::Melee,
                    });
                }
            });

        // A kit is picked from inside a command, which runs inside a flecs
        // system, so every `add` that `kit::apply` makes is deferred: the
        // ability entities and the `(Grants, ability)` edges do not exist yet
        // when `select_kit` reads them back to build a hotbar, and the player
        // gets an empty one.
        //
        // Marking the player instead and rebuilding on a later tick is what
        // makes the hotbar arrive, because by the time a system runs again
        // every one of those deferred operations has been committed.
        world.component::<HotbarStale>();
        world
            .observer::<flecs::OnAdd, ()>()
            .with((Playing, id::<flecs::Wildcard>()))
            .with(Player::id())
            .each_entity(|player, ()| {
                player.add(HotbarStale::id());
            });

        world
            .system_named::<&PlayerId>("smash::push_stale_hotbars")
            .kind(id::<flecs::pipeline::PostUpdate>())
            .with(Player::id())
            .with(HotbarStale::id())
            .each_entity(|player, id| {
                let hotbar = kit::hotbar(player);
                if hotbar.is_empty() {
                    return;
                }
                let id = *id;
                player
                    .world()
                    .get::<&crate::server::ServerHandle>(|server| server.set_hotbar(id, &hotbar));
                player.remove(HotbarStale::id());
            });

        // The game owns health, so the client's bar has to be told about the
        // game's number rather than hyperion's. Pushing it on change instead of
        // every tick keeps this off the hot path.
        world
            .observer::<flecs::OnSet, &crate::module::player::Health>()
            .with(Player::id())
            .each_entity(|entity, health| {
                let Some(id) = entity.try_get::<&PlayerId>(|id| *id) else {
                    return;
                };
                entity.set(HyperionHealth::new(health.current));
                entity.world().get::<&crate::server::ServerHandle>(|server| {
                    server.set_health(id, health.current, health.max);
                });
            });
    }
}

/// Which of the nine hotbar slots the player is holding.
fn held_slot(player: EntityView<'_>) -> Option<u8> {
    let absolute = player.try_get::<&PlayerInventory>(PlayerInventory::get_cursor_index)?;
    u8::try_from(absolute.checked_sub(PlayerInventory::HOTBAR_START_SLOT)?).ok()
}

/// The attacker's kit's melee damage, or a bare-handed default.
fn melee_damage(attacker: EntityView<'_>) -> f32 {
    attacker
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| stats.melee_damage))
        .unwrap_or(1.0)
}
