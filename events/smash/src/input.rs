//! hyperion's packet events, turned into the game's entry points.
//!
//! Everything here is a translation. No rule about what an ability does or how
//! much a swing hurts lives in this file; it decides only which of the game's
//! public functions a given packet means.

use flecs_ecs::prelude::*;
use hyperion::{
    simulation::{Name, event, metadata::living_entity::Health as HyperionHealth},
    storage::EventQueue,
};
use hyperion_inventory::PlayerInventory;

use crate::{
    adapter::player_id,
    flecs_ext::EntityViewExt,
    module::{
        ability,
        damage::{self, DamageKind, Damaged, MeleeBonus},
        kit::{self, KitStats, Playing},
        knockback::Knockback,
        player::{Player, Position},
        selector,
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
        // A hyperion player becomes a smash player. `Player` carries `With`
        // traits for every mirrored component, so this one add is what gives the
        // entity its Position, Facing, Health and the rest.
        //
        // Keyed on reaching play and not on `simulation::Player`, which hyperion
        // adds while the connection is still in login. Everything the game does
        // to a player is a packet at a play-state id, so a player counted any
        // earlier is one the lobby tallies before they can see anything and one
        // the scoreboard writes a sidebar to during the configuration state,
        // where a real 26.2 client reads the id against the configuration
        // table and drops the connection.
        world
            .observer::<flecs::OnAdd, ()>()
            .with_enum(hyperion::simulation::PacketState::Play)
            .each_entity(|entity, ()| {
                entity.set(player_id(entity.id())).add(Player::id());
            });

        // The scoreboard and the death messages are built from `EntityView::name`,
        // and an unnamed entity renders as an empty string, so a whole sidebar
        // reads as bare life counts. hyperion learns the username during login;
        // copying it onto the flecs name is what makes those lines legible.
        world
            .observer::<flecs::OnSet, &Name>()
            .with(Player::id())
            .each_entity(|entity, name| {
                entity.set_name(name);
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

        // Right-clicking a mob in the lobby, which is how a kit is picked.
        // Every entity in the world reaches this and `selector::click_mob`
        // answers for the fifteen that are standing on podiums.
        world
            .system_named::<&mut EventQueue<event::EntityInteract>>("smash::on_entity_interact")
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event in queue.drain() {
                    let player = world.entity_from_id(event.from);
                    if player.has(Player::id()) {
                        let _unused = selector::click_mob(&world, player, event.target);
                    }
                }
            });

        // Right-clicking the wool a mob is standing on, which does the same
        // thing. Every block in the world reaches this, and `selector::click`
        // answers for the handful that are podiums.
        world
            .system_named::<&mut EventQueue<event::BlockInteract>>("smash::on_block_interact")
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event in queue.drain() {
                    let player = world.entity_from_id(event.from);
                    if player.has(Player::id()) {
                        let _unused = selector::click(&world, player, event.position);
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
                    let clock = world.get::<&crate::module::damage::MatchClock>(|clock| clock.0);
                    let amount = melee_damage(attacker, victim.id(), clock);

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

        // The same deferral applies to abilities arriving and leaving one at a
        // time rather than a whole kit at once, which is what the Smash Crystal
        // does: without this a granted ultimate never reaches slot 8 and an
        // expired one stays there forever.
        //
        // One term, and the `Player` check moved into the body. A second filter
        // term turns this into a query-match observer, which fires on any table
        // transition that keeps the query satisfied -- so every ability use,
        // which adds and removes an invulnerability marker, rewrote the whole
        // inventory in the same tick.
        let mark_stale = |entity: EntityView<'_>, ()| {
            if entity.has(Player::id()) {
                entity.add(HotbarStale::id());
            }
        };
        world
            .observer::<flecs::OnAdd, ()>()
            .with((ability::Grants, id::<flecs::Wildcard>()))
            .each_entity(mark_stale);
        world
            .observer::<flecs::OnRemove, ()>()
            .with((ability::Grants, id::<flecs::Wildcard>()))
            .each_entity(mark_stale);

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
                // Deliberately a full replace and not a diff: the seam's only
                // hotbar verb is "here is the whole bar", and a kit change, a
                // respawn and a crystal expiring all want the same answer.
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
                entity
                    .world()
                    .get::<&crate::server::ServerHandle>(|server| {
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

/// The attacker's kit's melee damage, plus whatever their kit has stacked onto
/// it, or a bare-handed default.
fn melee_damage(attacker: EntityView<'_>, victim: Entity, now: f32) -> f32 {
    let base = attacker
        .find_target(Playing, |_| true)
        .and_then(|kit| kit.try_get::<&KitStats>(|stats| stats.melee_damage))
        .unwrap_or(1.0);
    let bonus = attacker
        .try_get::<&MeleeBonus>(|bonus| bonus.applies_to(victim, now))
        .unwrap_or(0.0);
    base + bonus
}
