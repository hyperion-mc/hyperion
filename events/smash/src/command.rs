//! Chat commands, which are the only way to pick a kit until there is a menu.
//!
//! Mineplex used a compass and an inventory GUI in the hub. `/kit <name>` is the
//! same choice made through the one input surface that already works.

use std::fmt::Write;

use clap::Parser;
use flecs_ecs::core::{
    ComponentId, Entity, EntityView, EntityViewGet, World, WorldGet, WorldProvider,
};
use hyperion::net::{Compose, ConnectionId, agnostic};
use hyperion_clap::{CommandPermission, MinecraftCommand, hyperion_command::CommandRegistry};

use crate::module::{
    ability,
    kit::{self, Kit, KitBlurb, KitName},
    lobby,
};

pub fn register(registry: &mut CommandRegistry, world: &World) {
    // The kit names a client offers on tab are a query over the kit prefabs,
    // taken when the player presses tab. Adding a kit changes what `/kit `
    // completes to and there is no list here to update, which is the same claim
    // the rest of this crate makes about kits and the same way it is kept
    // honest: `tests/modularity.rs` adds one from outside the crate.
    KitCommand::register(registry, world).completes("name", Kit::id());
    KitsCommand::register(registry, world);
    AbilitiesCommand::register(registry, world);
    CrystalCommand::register(registry, world);
}

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "kit")]
#[command_permission(group = "Normal")]
pub struct KitCommand {
    /// The kit to play, as shown by `/kits`.
    ///
    /// Several words rather than one, because more than half the roster has a
    /// space in its name and a player who types the name they can see on the
    /// screen should not be told `unexpected argument 'Golem'`. The words are
    /// squashed back together before matching, so `/kit Iron Golem`,
    /// `/kit irongolem` and `/kit IRON GOLEM` are all the same request.
    #[arg(trailing_var_arg = true, num_args = 1..)]
    name: Vec<String>,
}

impl MinecraftCommand for KitCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let player = caller.entity_view(world);

        // A Minecraft command argument is one whitespace-delimited token, and
        // half the kit names in Super Smash Mobs have a space in them. Matching
        // on the squashed name is what lets `/kit irongolem` mean "Iron Golem"
        // without the game's own registry having to care.
        let Some(canonical) = resolve(&world, &self.name.join(" ")) else {
            tell(world, caller, "§cNo such kit. Try /kits.");
            return;
        };

        if let Err(reason) = lobby::select_kit(&world, player, canonical) {
            tell(world, caller, &format!("§c{reason}"));
        }
        // On success select_kit has already sent the confirmation and the new
        // hotbar through the seam.
    }
}

/// The registered kit whose name matches `requested` once case and punctuation
/// are discarded.
fn resolve(world: &flecs_ecs::core::WorldRef<'_>, requested: &str) -> Option<&'static str> {
    let wanted = squash(requested);
    kit::registry(world).into_iter().find_map(|id| {
        world
            .entity_from_id(id)
            .try_get::<&KitName>(|name| name.0)
            .filter(|name| squash(name) == wanted)
    })
}

fn squash(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[derive(Parser, CommandPermission, Debug)]
#[command(name = "kits")]
#[command_permission(group = "Normal")]
pub struct KitsCommand;

impl MinecraftCommand for KitsCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();

        let mut listing = String::from("§6Kits:");
        for id in kit::registry(&world) {
            let entry = world.entity_from_id(id);
            let Some(name) = entry.try_get::<&KitName>(|name| name.0) else {
                continue;
            };
            let blurb = entry.try_get::<&KitBlurb>(|blurb| blurb.0).unwrap_or("");
            let _unused = write!(listing, "\n§e{name}§7 — {blurb}");
        }

        tell(world, caller, &listing);
    }
}

/// The line `/abilities` prefixes each ability with.
///
/// A marker rather than a bare object per line because chat is a shared
/// channel: a reader has to be able to tell the manifest apart from a death
/// message that happens to start with a brace.
pub const MANIFEST_PREFIX: &str = "smash-ability ";

/// The line that closes the manifest, with the count that should have preceded
/// it. A reader that sees fewer has lost some to a truncated chat packet, and
/// silently testing a short roster is exactly the failure this whole mechanism
/// exists to prevent.
pub const MANIFEST_END_PREFIX: &str = "smash-abilities-end ";

/// Dump the ability registry, one JSON object per chat line.
///
/// The registry lives in the world as components on ability prefabs, and this is
/// its only view over the wire. The end to end gate reads it and drives every
/// ability it names, so an ability that exists is an ability the gate tests: no
/// list of kits is written down anywhere on the client side, and nothing has to
/// be edited when a kit is added.
#[derive(Parser, CommandPermission, Debug)]
#[command(name = "abilities")]
#[command_permission(group = "Normal")]
pub struct AbilitiesCommand;

impl MinecraftCommand for AbilitiesCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let declared = ability::manifest(&world);
        for entry in &declared {
            tell(world, caller, &format!("{MANIFEST_PREFIX}{}", json(entry)));
        }
        tell(
            world,
            caller,
            &format!("{MANIFEST_END_PREFIX}{}", declared.len()),
        );
    }
}

/// One ability as a JSON object.
///
/// Hand-rolled rather than pulled in behind serde: this is four scalar kinds and
/// a string list, the crate has no JSON dependency, and a schema a reader has to
/// match exactly is easier to check when it is written out in one place.
fn json(entry: &ability::Declared) -> String {
    let mut out = String::new();
    let _unused = write!(
        out,
        r#"{{"kit":"{}","name":"{}","slot":{},"item":"{}","cooldown":{},"#,
        escape(entry.kit),
        escape(entry.name),
        entry.slot,
        escape(entry.item),
        entry.cooldown,
    );
    match entry.charge_time {
        Some(seconds) => {
            let _unused = write!(out, r#""charge_time":{seconds},"#);
        }
        None => out.push_str(r#""charge_time":null,"#),
    }
    match entry.energy_cost {
        Some(cost) => {
            let _unused = write!(out, r#""energy_cost":{cost},"#);
        }
        None => out.push_str(r#""energy_cost":null,"#),
    }
    let _unused = write!(
        out,
        r#""requires_ground":{},"refunds_on_hit":{},"ultimate":{},"proves":["#,
        entry.requires_ground, entry.refunds_on_hit, entry.ultimate,
    );
    for (index, observable) in entry.proves.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let _unused = write!(out, r#""{}""#, observable.as_str());
    }
    out.push_str("]}");
    out
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Pick up a Smash Crystal.
///
/// Mineplex spawned the crystal in the arena and you walked into it. Nothing
/// spawns one here yet -- that is arena work, and the arena is somebody else's
/// file -- so this is the entry point to the same mechanic: the ultimate is
/// granted for [`ability::ULTIMATE_SECONDS`] and taken back when it lapses,
/// through exactly the code path a real pickup would use.
#[derive(Parser, CommandPermission, Debug)]
#[command(name = "crystal")]
#[command_permission(group = "Normal")]
pub struct CrystalCommand;

impl MinecraftCommand for CrystalCommand {
    fn execute(self, system: EntityView<'_>, caller: Entity) {
        let world = system.world();
        let player = caller.entity_view(world);

        let Some(name) = kit::ultimate_name(player) else {
            tell(world, caller, "§cPick a kit first. Try /kits.");
            return;
        };
        if kit::grant_ultimate(&world, player, ability::ULTIMATE_SECONDS) {
            tell(
                world,
                caller,
                &format!(
                    "§bSmash Crystal: {name} for {:.0} seconds.",
                    ability::ULTIMATE_SECONDS
                ),
            );
        } else {
            tell(world, caller, "§cYou are already holding one.");
        }
    }
}

fn tell(world: flecs_ecs::core::WorldRef<'_>, caller: Entity, message: &str) {
    let chat = agnostic::chat(message);
    world.get::<&Compose>(|compose| {
        caller.entity_view(world).get::<&ConnectionId>(|stream| {
            if let Err(error) = compose.unicast(&chat, *stream) {
                tracing::warn!("dropping a smash command reply: {error}");
            }
        });
    });
}
