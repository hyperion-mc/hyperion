//! The one file that imports both hyperion and the game.
//!
//! Writes cross the seam as a queue rather than as immediate world edits.
//! [`Server`] is called from inside flecs observers -- ability activation, the
//! damage pipeline, the lobby state machine -- where the world is mid-iteration
//! and taking a second mutable borrow of a component a running query is reading
//! aborts at runtime. Draining once per tick in `PostUpdate` costs knockback one
//! tick of latency and buys immunity from that whole class of bug.
//!
//! [`Cue`] and [`HotbarItem`] are the game's own closed vocabulary. Choosing
//! which Minecraft sound, particle and item each one becomes is a hosting
//! decision, so the mapping lives here and nowhere under `src/module/`.

use std::sync::{Arc, Mutex};

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::{
    hyperion_minecraft_proto::{
        generated::packet_id::play::clientbound::PacketId,
        packets::{
            play::{
                chunk::{LevelParticles, Particle},
                clientbound::{SetActionBarText, SetDisplayObjective, SetHealth, SetTitleText},
                player::{
                    DisplaySlot, ObjectiveDisplay, ObjectiveRenderType, SetObjective, SetScore,
                },
            },
            play_login::{GameEvent, GameType},
        },
        text::Component,
    },
    net::{Compose, ConnectionId, agnostic, protocol, protocol::Clientbound},
    simulation::{PendingTeleportation, Velocity, metadata::living_entity::Health},
    valence_protocol::{ItemKind, ItemStack},
};
use hyperion_inventory::PlayerInventory;
use hyperion_utils::EntityExt;
use valence_nbt::{Compound, List, Value};

use crate::server::{Channel, Cue, HotbarItem, PlayerId, Server};

/// One deferred write.
///
/// Owned data rather than borrows, because the queue outlives the call that
/// filled it by up to a tick.
enum Op {
    AddVelocity {
        player: Entity,
        delta: Vec3,
    },
    Teleport {
        player: Entity,
        to: Vec3,
    },
    SetHealth {
        player: Entity,
        health: f32,
        max: f32,
    },
    SetHotbar {
        player: Entity,
        items: Vec<HotbarItem>,
    },
    Message {
        player: Entity,
        channel: Channel,
        text: String,
    },
    Broadcast {
        channel: Channel,
        text: String,
    },
    Sidebar {
        player: Entity,
        title: String,
        lines: Vec<String>,
    },
    Spectating {
        player: Entity,
        spectating: bool,
    },
    Play {
        at: Vec3,
        cue: Cue,
    },
}

/// The queue both halves share, as a singleton so the drain system can name it
/// as an ordinary query term.
#[derive(Component)]
pub struct OpQueue(Arc<Mutex<Vec<Op>>>);

/// hyperion's implementation of the game's seam.
pub struct HyperionServer {
    ops: Arc<Mutex<Vec<Op>>>,
}

impl HyperionServer {
    fn push(&self, op: Op) {
        // A poisoned queue means a previous drain panicked; the game is already
        // over, and swallowing the write here would only hide the first panic.
        self.ops.lock().expect("server op queue poisoned").push(op);
    }
}

/// A player as hyperion knows them. The game's opaque [`PlayerId`] is the flecs
/// entity id, so no side table is needed to get back.
const fn entity_of(player: PlayerId) -> Entity {
    Entity(player.0)
}

#[must_use]
pub const fn player_id(entity: Entity) -> PlayerId {
    PlayerId(entity.0)
}

impl Server for HyperionServer {
    fn add_velocity(&self, player: PlayerId, delta: Vec3) {
        self.push(Op::AddVelocity {
            player: entity_of(player),
            delta,
        });
    }

    fn teleport(&self, player: PlayerId, to: Vec3) {
        self.push(Op::Teleport {
            player: entity_of(player),
            to,
        });
    }

    fn set_health(&self, player: PlayerId, health: f32, max: f32) {
        self.push(Op::SetHealth {
            player: entity_of(player),
            health,
            max,
        });
    }

    fn set_hotbar(&self, player: PlayerId, items: &[HotbarItem]) {
        self.push(Op::SetHotbar {
            player: entity_of(player),
            items: items.to_vec(),
        });
    }

    fn send_message(&self, player: PlayerId, channel: Channel, text: &str) {
        self.push(Op::Message {
            player: entity_of(player),
            channel,
            text: text.to_owned(),
        });
    }

    fn broadcast(&self, channel: Channel, text: &str) {
        self.push(Op::Broadcast {
            channel,
            text: text.to_owned(),
        });
    }

    fn set_sidebar(&self, player: PlayerId, title: &str, lines: &[String]) {
        self.push(Op::Sidebar {
            player: entity_of(player),
            title: title.to_owned(),
            lines: lines.to_vec(),
        });
    }

    fn set_spectating(&self, player: PlayerId, spectating: bool) {
        self.push(Op::Spectating {
            player: entity_of(player),
            spectating,
        });
    }

    fn cue(&self, at: Vec3, cue: Cue) {
        self.push(Op::Play { at, cue });
    }
}

/// Installs the seam. Imported before [`crate::SmashModule`] so the game finds
/// its [`crate::server::ServerHandle`] already set.
#[derive(Component)]
pub struct SmashAdapterModule;

impl Module for SmashAdapterModule {
    fn module(world: &World) {
        world.import::<crate::SmashModule>();

        world.component::<OpQueue>().add_trait::<flecs::Singleton>();

        let ops = Arc::new(Mutex::new(Vec::new()));
        world.set(OpQueue(Arc::clone(&ops)));
        world.set(crate::server::ServerHandle::new(HyperionServer { ops }));

        world.import::<crate::mirror::MirrorModule>();
        world.import::<crate::input::InputModule>();

        // PostUpdate rather than OnStore: hyperion's own inventory and entity
        // state sync run in OnStore, so the writes have to already be on the
        // components by then or they wait a further tick.
        world
            .system_named::<(&OpQueue, &Compose)>("smash::apply_server_ops")
            .kind(id::<flecs::pipeline::PostUpdate>())
            .each_iter(|it, _, (queue, compose)| {
                let world = it.world();
                let drained =
                    std::mem::take(&mut *queue.0.lock().expect("server op queue poisoned"));
                for op in drained {
                    apply(world, compose, op);
                }
            });
    }
}

fn apply(world: WorldRef<'_>, compose: &Compose, op: Op) {
    match op {
        Op::AddVelocity { player, delta } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            // hyperion's `sync_player_entity` turns a non-zero Velocity into one
            // SetEntityMotion and zeroes it again, which is exactly the
            // "send one velocity packet" contract that file asks for.
            entity.get::<&mut Velocity>(|velocity| velocity.0 += delta);
        }
        Op::Teleport { player, to } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            entity.set(PendingTeleportation::new(to));
        }
        Op::SetHealth {
            player,
            health,
            max,
        } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            // Two writes, because they reach different audiences: the metadata
            // component is what other players see over the victim's head, and
            // SetHealth is the only thing that moves the victim's own bar.
            entity.set(Health::new(health));
            let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
                return;
            };
            let scaled = if max > 0.0 { health * 20.0 / max } else { 0.0 };
            let packet = SetHealth {
                health: scaled,
                food: 20,
                saturation: 5.0,
            };
            let _unused =
                protocol::send(compose, connection, PacketId::SetHealth.to_raw(), &packet);
        }
        Op::SetHotbar { player, items } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            entity.get::<&mut PlayerInventory>(|inventory| {
                inventory.clear();
                for item in &items {
                    let _unused = inventory.set_hotbar(u16::from(item.slot), stack_for(item));
                }
            });
        }
        Op::Message {
            player,
            channel,
            text,
        } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
                return;
            };
            send(compose, channel, &text, Some(connection));
        }
        Op::Broadcast { channel, text } => send(compose, channel, &text, None),
        Op::Sidebar {
            player,
            title,
            lines,
        } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
                return;
            };
            sidebar(compose, connection, &title, &lines);
        }
        Op::Spectating { player, spectating } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
                return;
            };
            let mode = if spectating {
                GameType::Spectator
            } else {
                GameType::Survival
            };
            let packet = GameEvent {
                event: GameEvent::CHANGE_GAME_MODE,
                param: f32::from(mode.to_id()),
            };
            let _unused = compose.unicast(
                Clientbound::new(PacketId::GameEvent.to_raw(), &packet),
                connection,
            );
        }
        Op::Play { at, cue } => play_cue(compose, at, cue),
    }
}

fn send(compose: &Compose, channel: Channel, text: &str, to: Option<ConnectionId>) {
    match channel {
        Channel::Chat => {
            let chat = agnostic::chat(text);
            dispatch(compose, &chat, to);
        }
        Channel::ActionBar => {
            let component = Component::text(text);
            let packet = SetActionBarText {
                text: component.to_tag(),
            };
            dispatch(
                compose,
                Clientbound::new(PacketId::SetActionBarText.to_raw(), &packet),
                to,
            );
        }
        Channel::Title => {
            let component = Component::text(text);
            let packet = SetTitleText {
                text: component.to_tag(),
            };
            dispatch(
                compose,
                Clientbound::new(PacketId::SetTitleText.to_raw(), &packet),
                to,
            );
        }
    }
}

fn dispatch<P>(compose: &Compose, packet: P, to: Option<ConnectionId>)
where
    P: hyperion::PacketBundle,
{
    let result = match to {
        Some(connection) => compose.unicast(packet, connection),
        None => compose.broadcast(packet).send(),
    };
    if let Err(error) = result {
        tracing::warn!("dropping a smash packet: {error}");
    }
}

/// Rebuild a player's sidebar from nothing.
///
/// Remove-then-create rather than diffing against what was there last tick: the
/// objective is per player and the scoreboard is redrawn once a second at most,
/// so keeping a shadow copy of every line to compute a delta would be more state
/// than the whole feature is worth.
fn sidebar(compose: &Compose, to: ConnectionId, title: &str, lines: &[String]) {
    const OBJECTIVE: &str = "smash";

    // `METHOD_REMOVE`, which takes every score with it, so the rows below are
    // written into an objective the client has just been told is empty.
    let remove = SetObjective {
        objective_name: OBJECTIVE,
        display: None,
        change: false,
    };
    let _unused = compose.unicast(
        Clientbound::new(PacketId::SetObjective.to_raw(), &remove),
        to,
    );

    let title_text = Component::text(title);
    let create = SetObjective {
        objective_name: OBJECTIVE,
        display: Some(ObjectiveDisplay {
            display_name: title_text,
            render_type: ObjectiveRenderType::Integer,
            number_format: None,
        }),
        change: false,
    };
    let _unused = compose.unicast(
        Clientbound::new(PacketId::SetObjective.to_raw(), &create),
        to,
    );

    let display = SetDisplayObjective {
        id: DisplaySlot::Sidebar.to_id(),
        objective_name: OBJECTIVE,
    };
    let _unused = compose.unicast(
        Clientbound::new(PacketId::SetDisplayObjective.to_raw(), &display),
        to,
    );

    // A sidebar row is keyed by its own text, so two identical rows collapse
    // into one. Padding with a run of colour codes keeps them distinct and
    // renders as nothing.
    for (index, line) in lines.iter().enumerate() {
        let Ok(score) = i32::try_from(lines.len().saturating_sub(index)) else {
            continue;
        };
        let unique = format!("{line}{}", "§r".repeat(index));
        let packet = SetScore {
            owner: unique.as_str(),
            objective_name: OBJECTIVE,
            score,
            display: None,
            number_format: None,
        };
        let _unused = compose.unicast(Clientbound::new(PacketId::SetScore.to_raw(), &packet), to);
    }
}

/// The game's six cues, as Minecraft sounds and particles.
///
/// `[INFERRED]` throughout: Mineplex's own choices are not in the leaked source,
/// which loaded them from the same spreadsheet as everything else.
/// Half-width of the box a cue's particles are scattered through, in blocks.
const CUE_SPREAD: f32 = 0.4;

fn play_cue(compose: &Compose, at: Vec3, cue: Cue) {
    let sound = match cue {
        Cue::Explosion => "minecraft:entity.generic.explode",
        Cue::Teleport => "minecraft:entity.enderman.teleport",
        Cue::Hurt => "minecraft:entity.player.hurt",
        Cue::Death => "minecraft:entity.player.big_fall",
        Cue::AbilityReady => "minecraft:block.note_block.pling",
        Cue::Charge => "minecraft:block.note_block.hat",
    };

    if let Ok(ident) = hyperion::valence_ident::Ident::new(sound) {
        let packet = agnostic::sound(ident, at).volume(1.0).pitch(1.0).build();
        if let Err(error) = compose.broadcast(&packet).send() {
            tracing::warn!("dropping a smash sound: {error}");
        }
    }

    let particle = match cue {
        Cue::Explosion => Some(Particle::Explosion),
        Cue::Teleport => Some(Particle::Portal),
        Cue::Death => Some(Particle::Cloud),
        Cue::Hurt | Cue::AbilityReady | Cue::Charge => None,
    };
    let Some(particle) = particle else {
        return;
    };
    let packet = LevelParticles {
        // A cue marks something that just happened to a player, so it is worth
        // seeing from further out than the client's normal particle radius.
        override_limiter: true,
        // The client's particle setting is the player's own choice.
        always_show: false,
        x: f64::from(at.x),
        y: f64::from(at.y),
        z: f64::from(at.z),
        x_dist: CUE_SPREAD,
        y_dist: CUE_SPREAD,
        z_dist: CUE_SPREAD,
        max_speed: 0.5,
        count: 40,
        particle,
    };
    if let Err(error) = compose
        .broadcast(Clientbound::new(PacketId::LevelParticles.to_raw(), &packet))
        .send()
    {
        tracing::warn!("dropping a smash particle: {error}");
    }
}

/// A kit's hotbar entry as a real item stack.
///
/// The game names items as full `minecraft:` identifiers because that is how
/// Mineplex's kit data did; valence's table is keyed on the bare path.
fn stack_for(item: &HotbarItem) -> ItemStack {
    let bare = item.item.strip_prefix("minecraft:").unwrap_or(item.item);
    let kind = ItemKind::from_str(bare).unwrap_or(ItemKind::Stick);
    ItemStack::new(kind, 1, Some(display_tag(&item.name, &item.lore)))
}

/// The 1.20.1 `display` tag: a name and lore, each one a JSON chat component.
fn display_tag(name: &str, lore: &[String]) -> Compound {
    let mut display = Compound::new();
    display.insert("Name", Value::String(json_text(name)));
    if !lore.is_empty() {
        let rendered: Vec<_> = lore
            .iter()
            .map(|line| json_text(&format!("§7{line}")))
            .collect();
        display.insert("Lore", Value::List(List::String(rendered)));
    }
    let mut tag = Compound::new();
    tag.insert("display", Value::Compound(display));
    tag
}

fn json_text(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!(r#"{{"text":"{escaped}","italic":false}}"#)
}

/// The numeric id the client knows an entity by. Re-exported so the input layer
/// can turn an `Interact` packet's target back into an entity.
#[must_use]
pub fn minecraft_id(entity: EntityView<'_>) -> i32 {
    entity.minecraft_id()
}
