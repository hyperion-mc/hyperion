//! The one file that imports both hyperion and the game.
//!
//! Writes cross the seam as a queue rather than as immediate world edits.
//! [`Server`] is called from inside flecs observers -- ability activation, the
//! damage pipeline, the lobby state machine -- where the world is mid-iteration
//! and taking a second mutable borrow of a component a running query is reading
//! aborts at runtime. Draining once per tick in `PostUpdate` costs knockback one
//! tick of latency and buys immunity from that whole class of bug.
//!
//! [`HotbarItem`] is the game's own closed vocabulary, and choosing which
//! Minecraft item each one becomes is a hosting decision, so that mapping
//! lives here and nowhere under `src/module/`. [`Sound`] and [`Particles`] are
//! the exceptions and arrive already naming a vanilla sound event and a
//! vanilla particle, because what an ability sounds and looks like is part of
//! what the ability *is* and belongs with the kit that declares it; all that
//! is left here is handing them to the engine.

use std::{
    collections::{HashMap, hash_map::Entry},
    sync::{Arc, Mutex},
};

use flecs_ecs::prelude::*;
use glam::Vec3;
use hyperion::{
    egress::{boss_bar, player_join::roster},
    hyperion_minecraft_proto::{
        generated::packet_id::play::clientbound::PacketId,
        packets::play::{
            clientbound::{
                SetActionBarText, SetDisplayObjective, SetExperience, SetHealth, SetSubtitleText,
                SetTitleText, SetTitlesAnimation,
            },
            player::{
                BossBarColor, BossBarOverlay, DisplaySlot, NumberFormat, ObjectiveDisplay,
                ObjectiveRenderType, SetObjective, SetScore,
            },
        },
    },
    net::{Compose, ConnectionId, agnostic, protocol, protocol::Clientbound},
    simulation::{
        Flight as HostFlight, PendingTeleportation, Velocity,
        gamemode::{DefaultGamemode, Gamemode},
        metadata::living_entity::Health,
        skin::PlayerSkin,
    },
    valence_protocol::{ItemKind, ItemStack},
};
use hyperion_inventory::PlayerInventory;
use hyperion_utils::EntityExt;
use valence_nbt::{Compound, List, Value};

use crate::{
    module::kit::{self, Playing},
    server::{
        BarColour, BarSlot, BossBar, Channel, Experience, Flight, HotbarItem, Particles, PlayerId,
        Server, SidebarLine, Sound, SoundCategory, Status, Text, Title,
    },
};

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
    SetFlight {
        player: Entity,
        flight: Flight,
    },
    SetHealth {
        player: Entity,
        health: f32,
        max: f32,
    },
    SetFood {
        player: Entity,
        food: u8,
    },
    Status {
        player: Entity,
        status: Status,
    },
    SetHotbar {
        player: Entity,
        items: Vec<HotbarItem>,
    },
    Message {
        player: Entity,
        channel: Channel,
        text: Text,
    },
    Broadcast {
        channel: Channel,
        text: Text,
    },
    Sidebar {
        player: Entity,
        title: Text,
        lines: Vec<SidebarLine>,
    },
    Spectating {
        player: Entity,
        spectating: bool,
    },
    Particles(Particles),
    PlaySound {
        at: Vec3,
        sound: Sound,
    },
    PlaySoundTo {
        player: Entity,
        sound: Sound,
    },
    SetExperience {
        player: Entity,
        experience: Experience,
    },
    SetBossBar {
        player: Entity,
        slot: BarSlot,
        bar: BossBar,
    },
    ShowTitle {
        player: Entity,
        title: Title,
    },
    BroadcastTitle {
        title: Title,
    },
}

/// The queue both halves share, as a singleton so the drain system can name it
/// as an ordinary query term.
#[derive(Component)]
pub struct OpQueue(Arc<Mutex<Vec<Op>>>);

/// Both halves of the client's `SetHealth` packet, as last sent.
///
/// Vanilla carries health, food and saturation in one message, and the game
/// moves health and food on unrelated clocks -- health on every hit, food once
/// every seven seconds -- so whichever half is not changing has to be
/// remembered to be re-sent alongside the half that is. On the player entity
/// because that is where the rest of a player's mirrored state already lives.
///
/// The defaults are what a client that has never been told anything is already
/// drawing, so the first push of either half sends the other one unchanged
/// rather than blanking it.
#[derive(Component, Debug, Copy, Clone)]
struct Vitals {
    /// Health on vanilla's twenty-point bar, which is the only scale the
    /// packet has. The game's own maximum is a kit stat and can be anything.
    scaled_health: f32,
    /// Food points, `0..=20`.
    food: u8,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            scaled_health: VANILLA_HEALTH,
            food: VANILLA_FOOD,
        }
    }
}

/// A full vanilla health bar, and the scale everything is sent on.
const VANILLA_HEALTH: f32 = 20.0;

/// A full vanilla food bar.
const VANILLA_FOOD: u8 = 20;

/// Saturation, which this game has no mechanic for.
///
/// Sent as a constant because the field is not optional and a client uses it
/// only to decide how fast the food bar wobbles. Below the drain rate of any
/// kit, so it never delays a hunger tick.
const VANILLA_SATURATION: f32 = 5.0;

/// What was last sent to `entity`, or the defaults if nothing has been.
fn vitals_of(entity: EntityView<'_>) -> Vitals {
    entity
        .try_get::<&Vitals>(|vitals| *vitals)
        .unwrap_or_default()
}

/// Send the whole packet after one half of it changed.
fn send_vitals(entity: EntityView<'_>, compose: &Compose, vitals: Vitals) {
    let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
        return;
    };
    let packet = SetHealth {
        health: vitals.scaled_health,
        food: i32::from(vitals.food),
        saturation: VANILLA_SATURATION,
    };
    let _unused = protocol::send(compose, connection, PacketId::SetHealth.to_raw(), &packet);
}

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

    fn set_flight(&self, player: PlayerId, flight: Flight) {
        self.push(Op::SetFlight {
            player: entity_of(player),
            flight,
        });
    }

    fn set_health(&self, player: PlayerId, health: f32, max: f32) {
        self.push(Op::SetHealth {
            player: entity_of(player),
            health,
            max,
        });
    }

    fn set_food(&self, player: PlayerId, food: u8) {
        self.push(Op::SetFood {
            player: entity_of(player),
            food,
        });
    }

    fn status(&self, player: PlayerId, status: Status) {
        self.push(Op::Status {
            player: entity_of(player),
            status,
        });
    }

    fn set_hotbar(&self, player: PlayerId, items: &[HotbarItem]) {
        self.push(Op::SetHotbar {
            player: entity_of(player),
            items: items.to_vec(),
        });
    }

    fn send_message(&self, player: PlayerId, channel: Channel, text: Text) {
        self.push(Op::Message {
            player: entity_of(player),
            channel,
            text,
        });
    }

    fn broadcast(&self, channel: Channel, text: Text) {
        self.push(Op::Broadcast { channel, text });
    }

    fn set_sidebar(&self, player: PlayerId, title: Text, lines: &[SidebarLine]) {
        self.push(Op::Sidebar {
            player: entity_of(player),
            title,
            lines: lines.to_vec(),
        });
    }

    fn set_spectating(&self, player: PlayerId, spectating: bool) {
        self.push(Op::Spectating {
            player: entity_of(player),
            spectating,
        });
    }

    fn particles(&self, effect: Particles) {
        self.push(Op::Particles(effect));
    }

    fn play_sound(&self, at: Vec3, sound: Sound) {
        self.push(Op::PlaySound { at, sound });
    }

    fn play_sound_to(&self, player: PlayerId, sound: Sound) {
        self.push(Op::PlaySoundTo {
            player: entity_of(player),
            sound,
        });
    }

    fn set_experience(&self, player: PlayerId, experience: Experience) {
        self.push(Op::SetExperience {
            player: entity_of(player),
            experience,
        });
    }

    fn set_boss_bar(&self, player: PlayerId, slot: BarSlot, bar: BossBar) {
        self.push(Op::SetBossBar {
            player: entity_of(player),
            slot,
            bar,
        });
    }

    fn show_title(&self, player: PlayerId, title: Title) {
        self.push(Op::ShowTitle {
            player: entity_of(player),
            title,
        });
    }

    fn broadcast_title(&self, title: Title) {
        self.push(Op::BroadcastTitle { title });
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
        world.component::<PlayerBars>();
        world.component::<Vitals>();

        let ops = Arc::new(Mutex::new(Vec::new()));
        world.set(OpQueue(Arc::clone(&ops)));
        world.set(crate::server::ServerHandle::new(HyperionServer { ops }));

        world.import::<crate::mirror::MirrorModule>();
        world.import::<crate::input::InputModule>();

        // Every player is in adventure, so an arena cannot be dug up. Set here
        // rather than per player: hyperion puts a joining player into whatever
        // this singleton says, and the one thing smash wants to say about
        // gamemode is the default.
        world.set(DefaultGamemode(Gamemode::Adventure));

        // The one place a kit's look crosses into the host. `KitSkin` lives on
        // the kit prefab and is reached through `(Playing, kit)`; hyperion
        // speaks profiles and wants a `PlayerSkin` on the player, so this is
        // the translation and not a second copy of the truth. Reacting to the
        // relation rather than to a call inside `kit::apply` means anything
        // that changes which mob you are dresses you correctly, including a
        // path nobody has written yet.
        world
            .observer_named::<flecs::OnAdd, ()>("wear_kit_skin")
            .with((Playing, id::<flecs::Wildcard>()))
            .each_entity(|player, ()| {
                let Some(skin) = kit::skin_of(player) else {
                    return;
                };
                // `wear` and not `set`: re-picking the mob you are already
                // playing publishes nothing, so a player leaning on a podium
                // does not send every other client a stream of profile
                // rewrites and entity respawns.
                roster::wear(
                    player,
                    PlayerSkin::new(
                        skin.textures.trim().to_owned(),
                        skin.signature.trim().to_owned(),
                    ),
                );
            });

        // PostUpdate rather than OnStore: hyperion's own inventory and entity
        // state sync run in OnStore, so the writes have to already be on the
        // components by then or they wait a further tick.
        world
            .system_named::<(&OpQueue, &Compose)>("apply_server_ops")
            .kind(id::<flecs::pipeline::PostUpdate>())
            .each_iter(|it, _, (queue, compose)| {
                let world = it.world();
                let drained =
                    std::mem::take(&mut *queue.0.lock().expect("server op queue poisoned"));
                // Every player's bars, resolved before any op is applied.
                //
                // `world.entity()` hands back its id at once but the
                // `set(PlayerBars)` that records it is deferred to the merge,
                // so a second `SetBossBar` for the same player in one drain
                // still reads the component as it stood at the start of the
                // tick. Resolving every slot into one map first, and writing
                // the component back once per player, is what makes a
                // duplicate bar impossible rather than merely unlikely. Doing
                // it per slot instead loses the other slot's entity to the
                // last write, and the tick after that mints a second bar for
                // it -- two match bars stacked on one screen.
                let mut bars: HashMap<Entity, PlayerBars> = HashMap::new();
                for op in &drained {
                    let Op::SetBossBar { player, slot, .. } = op else {
                        continue;
                    };
                    let resolved = match bars.entry(*player) {
                        Entry::Occupied(held) => held.into_mut(),
                        Entry::Vacant(empty) => match bars_of(world, *player) {
                            Some(held) => empty.insert(held),
                            None => continue,
                        },
                    };
                    resolved.0[slot.index()] = Some(match resolved.0[slot.index()] {
                        Some(bar) if world.entity_from_id(bar).is_alive() => bar,
                        _ => mint_bar(world, *player),
                    });
                }
                for (player, resolved) in &bars {
                    world.entity_from_id(*player).set(*resolved);
                }
                // The same hazard as `bars` above, one type along, and the
                // reason this is a map rather than a component read per op:
                // `set(Vitals)` is deferred too, so a `SetHealth` and a
                // `SetFood` for one player in one drain -- a player who traded
                // hits, which is a tick that happens constantly -- would have
                // the second read the component as it stood before the first.
                // It would send the *default* full health beside the new food
                // level and snap the client's health bar to full for a frame.
                // Carried across the loop and written back once per player.
                let mut vitals: HashMap<Entity, Vitals> = HashMap::new();
                for op in drained {
                    apply(world, compose, op, &bars, &mut vitals);
                }
                for (player, held) in &vitals {
                    world.entity_from_id(*player).set(*held);
                }
            });
    }
}

fn apply(
    world: WorldRef<'_>,
    compose: &Compose,
    op: Op,
    bars: &HashMap<Entity, PlayerBars>,
    vitals: &mut HashMap<Entity, Vitals>,
) {
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
        Op::SetFlight { player, flight } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            // `set` and not `get::<&mut _>`: hyperion sends the abilities
            // packet from an `OnSet` observer on this component, so a mutation
            // that skipped the hook would change what the server believes and
            // tell the client nothing.
            //
            // `is_flying` is false in both branches. The serverbound half of
            // this exchange is the only thing that ever turns it on, and
            // clearing it here is what ends a take-off the tick it starts:
            // without it the client stays in creative flight and the double
            // tap that starts the next jump never happens, because it is
            // already flying.
            entity.set(HostFlight {
                allow: flight.is_armed(),
                is_flying: false,
            });
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
            let held = vitals.entry(player).or_insert_with(|| vitals_of(entity));
            held.scaled_health = if max > 0.0 {
                health * VANILLA_HEALTH / max
            } else {
                0.0
            };
            send_vitals(entity, compose, *held);
        }
        Op::SetFood { player, food } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            let held = vitals.entry(player).or_insert_with(|| vitals_of(entity));
            held.food = food;
            send_vitals(entity, compose, *held);
        }
        Op::Status { player, status } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            // `Status::apply` broadcasts locally around the victim with nobody
            // excluded, so the victim's own client -- the one that owns the
            // movement prediction a slow has to change -- is in the broadcast.
            status.apply(entity);
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
            send(compose, channel, text, Some(connection));
        }
        Op::Broadcast { channel, text } => send(compose, channel, text, None),
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
            // Set the component and let hyperion's roster module publish it.
            // Writing the `GameEvent` here was the bug this replaces: it told
            // the client one thing and left the server believing another, so a
            // dead player was a spectator to their own client, a survival
            // player to the tab list, and whatever they started as to every
            // rule the server enforces. Coming back is a return to the
            // server's default rather than to a hardcoded survival, which is
            // what made respawning hand back the ability to break blocks.
            if spectating {
                entity.add_enum(Gamemode::Spectator);
            } else {
                world.get::<&DefaultGamemode>(|default| entity.add_enum(default.0));
            }
        }
        Op::Particles(effect) => effect.emit(world),
        Op::PlaySound { at, sound } => {
            let Some(packet) = encode(at, sound) else {
                return;
            };
            if let Err(error) = packet.broadcast_near(compose) {
                tracing::warn!("dropping a smash sound: {error}");
            }
        }
        Op::PlaySoundTo { player, sound } => {
            let entity = world.entity_from_id(player);
            if !entity.is_alive() {
                return;
            }
            let Some(connection) = entity.try_get::<&ConnectionId>(|id| *id) else {
                return;
            };
            // At the listener's own ears rather than anywhere in the world, so
            // it is the same loudness wherever they are standing.
            let at = entity
                .try_get::<&hyperion::simulation::Position>(|p| **p)
                .unwrap_or(Vec3::ZERO);
            let Some(packet) = encode(at, sound) else {
                return;
            };
            if let Err(error) = packet.play_to(compose, connection) {
                tracing::warn!("dropping a smash sound: {error}");
            }
        }
        Op::SetExperience { player, experience } => {
            let Some(connection) = connection_of(world, player) else {
                return;
            };
            let packet = SetExperience {
                experience_progress: experience.progress,
                experience_level: experience.level,
                // Never read by the client's HUD, which draws the bar from
                // `experience_progress` and the number from `experience_level`.
                // It is the value `/xp query points` would answer with, and
                // this game has no points to answer about.
                total_experience: 0,
            };
            let _unused = protocol::send(
                compose,
                connection,
                PacketId::SetExperience.to_raw(),
                &packet,
            );
        }
        Op::SetBossBar { player, slot, bar } => {
            let Some(entity) = bars
                .get(&player)
                .and_then(|held| held.0[slot.index()].as_ref())
            else {
                return;
            };
            world
                .entity_from_id(*entity)
                .set(boss_bar::Title(bar.title))
                .set(boss_bar::Progress(bar.progress))
                .set(boss_bar::Style {
                    colour: colour(bar.colour),
                    // No notches: the quantity under this bar is health and a
                    // percentage, and neither of them comes in six pieces.
                    overlay: BossBarOverlay::Progress,
                });
        }
        Op::ShowTitle { player, title } => {
            let Some(connection) = connection_of(world, player) else {
                return;
            };
            show_title(compose, &title, Some(connection));
        }
        Op::BroadcastTitle { title } => show_title(compose, &title, None),
    }
}

/// The connection behind a player, or nothing when they have left.
fn connection_of(world: WorldRef<'_>, player: Entity) -> Option<ConnectionId> {
    let entity = world.entity_from_id(player);
    if !entity.is_alive() {
        return None;
    }
    entity.try_get::<&ConnectionId>(|id| *id)
}

/// The bars a player already has, or nothing when they have left.
///
/// An empty set for a player who has never been drawn one, which is the same
/// answer as "every slot is still to be minted".
fn bars_of(world: WorldRef<'_>, player: Entity) -> Option<PlayerBars> {
    let player = world.entity_from_id(player);
    if !player.is_alive() {
        return None;
    }
    Some(
        player
            .try_get::<&PlayerBars>(|bars| *bars)
            .unwrap_or_default(),
    )
}

/// A fresh boss bar entity, shown to `player` alone.
///
/// Every bar this game draws is per player -- the match bar carries a
/// percentage that is only true of the person reading it, and the build stamp
/// has to reach whoever connects next -- so each one's audience is one edge,
/// `(ShownTo, player)`. Everything after that is `egress::boss_bar`'s: the
/// `Add` on the first push, one operation per field that moves after it, and
/// the `Remove` when the player leaves. A slot nobody ever writes to costs no
/// entity, because this is only called for a slot that is being written.
///
/// A child of the player, so it dies with them under flecs's own
/// `(ChildOf, OnDeleteTarget, Delete)`. Without that the bar entity would
/// outlive its only viewer with an empty audience forever, which on a server
/// nobody restarts is a leak measured in players seen.
///
/// No fog, no darkened sky and no boss music, which is the default `Effects`:
/// each of them changes how the arena looks or sounds, and these bars are
/// readouts rather than events.
fn mint_bar(world: WorldRef<'_>, player: Entity) -> Entity {
    world
        .entity()
        .child_of(player)
        .add(id::<boss_bar::BossBar>())
        .add((id::<boss_bar::ShownTo>(), player))
        .id()
}

/// The boss bar entities a player is being drawn, one per [`BarSlot`].
///
/// An array and not a component per slot, so a new slot is an entry in
/// [`BarSlot::ALL`] rather than a new component nobody remembers to register.
/// The width is `BarSlot::COUNT`, which is that list's length, and every index
/// written here comes from [`BarSlot::index`], which is a position in the same
/// list -- so this cannot be indexed out of bounds by a slot the list knows
/// about. A slot the list does not know about panics in `index` by name
/// instead, which is the only remaining way to get this wrong and it says so.
#[derive(Component, Debug, Copy, Clone, Default)]
struct PlayerBars([Option<Entity>; BarSlot::COUNT]);

const fn colour(colour: BarColour) -> BossBarColor {
    match colour {
        BarColour::Green => BossBarColor::Green,
        BarColour::Yellow => BossBarColor::Yellow,
        BarColour::Red => BossBarColor::Red,
        BarColour::Blue => BossBarColor::Blue,
    }
}

/// Put a title on screen: the timing, then the line under it, then the line
/// itself.
///
/// That order is the whole reason [`Title`] is one value. The subtitle packet
/// only stores a line; the title packet is what draws both and restarts the
/// animation, so a subtitle written after its title is a line the player sees
/// under the *next* one. The empty subtitle is sent rather than skipped for
/// the same reason in reverse: the client keeps the last one it was given, so
/// a title with nothing under it would otherwise inherit whatever the previous
/// one said.
fn show_title(compose: &Compose, title: &Title, to: Option<ConnectionId>) {
    let animation = SetTitlesAnimation {
        fade_in: title.times.fade_in,
        stay: title.times.stay,
        fade_out: title.times.fade_out,
    };
    dispatch(
        compose,
        Clientbound::new(PacketId::SetTitlesAnimation.to_raw(), &animation),
        to,
    );

    let blank = Text::text("");
    let subtitle = SetSubtitleText {
        text: title.subtitle.as_ref().unwrap_or(&blank).to_tag(),
    };
    dispatch(
        compose,
        Clientbound::new(PacketId::SetSubtitleText.to_raw(), &subtitle),
        to,
    );

    let text = SetTitleText {
        text: title.title.to_tag(),
    };
    dispatch(
        compose,
        Clientbound::new(PacketId::SetTitleText.to_raw(), &text),
        to,
    );
}

/// Turn the game's sound into hyperion's.
///
/// `None` only for an id that is not a valid identifier at all, which is a bug
/// in a kit declaration rather than a runtime condition; `tests/sound.rs` holds
/// every id in the game to the vanilla registry so one cannot reach here.
fn encode(at: Vec3, sound: Sound) -> Option<agnostic::Sound> {
    let ident = hyperion::valence_ident::Ident::new(sound.id)
        .inspect_err(|error| tracing::warn!("{} is not a sound id: {error}", sound.id))
        .ok()?;
    Some(
        agnostic::sound(ident, at)
            .volume(sound.volume)
            .pitch(sound.pitch)
            .category(category(sound.category))
            .build(),
    )
}

const fn category(category: SoundCategory) -> agnostic::SoundCategory {
    match category {
        SoundCategory::Master => agnostic::SoundCategory::Master,
        SoundCategory::Weather => agnostic::SoundCategory::Weather,
        SoundCategory::Blocks => agnostic::SoundCategory::Blocks,
        SoundCategory::Hostile => agnostic::SoundCategory::Hostile,
        SoundCategory::Neutral => agnostic::SoundCategory::Neutral,
        SoundCategory::Players => agnostic::SoundCategory::Players,
        SoundCategory::Ambient => agnostic::SoundCategory::Ambient,
        SoundCategory::Ui => agnostic::SoundCategory::Ui,
    }
}

fn send(compose: &Compose, channel: Channel, text: Text, to: Option<ConnectionId>) {
    match channel {
        Channel::Chat => {
            // `agnostic::chat` builds its own component from a string, so this
            // is the one place in smash where a style cannot survive. Giving
            // that helper a component-taking sibling is a change to hyperion's
            // own API rather than to the game: ENG-10796.
            //
            // Until then the drop is loud. Not a `debug_assert`: the servers
            // that would hit it are release builds, so an assertion is
            // compiled out exactly where it would have been read, and the mock
            // seam the tests use never reaches this function at all.
            if !text.style.is_empty() || !text.extra.is_empty() {
                tracing::warn!(
                    "dropping the style off a chat line, which Channel::Chat cannot carry: \
                     {text:?}"
                );
            }
            let chat = agnostic::chat(text.plain());
            dispatch(compose, &chat, to);
        }
        Channel::ActionBar => {
            let packet = SetActionBarText {
                text: text.to_tag(),
            };
            dispatch(
                compose,
                Clientbound::new(PacketId::SetActionBarText.to_raw(), &packet),
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
fn sidebar(compose: &Compose, to: ConnectionId, title: &Text, lines: &[SidebarLine]) {
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

    let create = SetObjective {
        objective_name: OBJECTIVE,
        display: Some(ObjectiveDisplay {
            display_name: title.clone(),
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
        id: DisplaySlot::Sidebar.id(),
        objective_name: OBJECTIVE,
    };
    let _unused = compose.unicast(
        Clientbound::new(PacketId::SetDisplayObjective.to_raw(), &display),
        to,
    );

    // A sidebar row is keyed by its score holder, and two rows with the same
    // holder collapse into one. The holder used to be the row's own text,
    // padded with a run of legacy reset codes to keep duplicates apart, which
    // meant the key and the visible text were the same string and neither
    // could be chosen freely. `SetScore.display` separates them: the holder is
    // a positional key nobody sees, and the row on screen is a component with
    // a real style.
    //
    // The score used to be `lines.len() - index`, so the column the client
    // draws in red carried this loop's counter. It is now the row's own score,
    // which is what `render` decided it means, and a row that means nothing by
    // it says so with `NumberFormat::Blank` rather than by hoping nobody
    // reads it.
    //
    // The order is the score's, not the packet's: the client sorts by score
    // descending and breaks ties on the holder, case-insensitively ascending.
    // So the key is zero-padded, which makes its lexicographic order the same
    // as this loop's and the same as the order `render` chose. Unpadded,
    // `row10` would sort between `row1` and `row2` and a full lobby would come
    // out shuffled.
    for (index, line) in lines.iter().enumerate() {
        let owner = format!("row{index:02}");
        let packet = SetScore {
            owner: owner.as_str(),
            objective_name: OBJECTIVE,
            score: line.score.value(),
            display: Some(line.text.clone()),
            // A rank-only row still needs a score to sort on, so the number is
            // suppressed at the client rather than left out of the packet.
            number_format: line.score.drawn().is_none().then_some(NumberFormat::Blank),
        };
        let _unused = compose.unicast(Clientbound::new(PacketId::SetScore.to_raw(), &packet), to);
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
///
/// JSON and not [`Text`], because this tag predates the move to NBT
/// components and `valence_nbt` is what carries it. Rewriting the item path
/// onto the component codec is worth doing and is not this change.
fn display_tag(name: &str, lore: &[String]) -> Compound {
    let mut display = Compound::new();
    display.insert("Name", Value::String(json_text(name, None)));
    if !lore.is_empty() {
        let rendered: Vec<_> = lore
            .iter()
            .map(|line| json_text(line, Some("gray")))
            .collect();
        display.insert("Lore", Value::List(List::String(rendered)));
    }
    let mut tag = Compound::new();
    tag.insert("display", Value::Compound(display));
    tag
}

/// One JSON text component.
///
/// The colour is a field and not a legacy section-sign prefix on the text. A
/// formatting code inside the literal only renders at all because the client
/// still runs the pre-1.16 formatter over component text, it cannot say
/// anything outside the sixteen named colours, and it is the exact shape of
/// the bug that put `[green]` on the smash sidebar.
fn json_text(text: &str, color: Option<&str>) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    color.map_or_else(
        || format!(r#"{{"text":"{escaped}","italic":false}}"#),
        |color| format!(r#"{{"text":"{escaped}","italic":false,"color":"{color}"}}"#),
    )
}

/// The numeric id the client knows an entity by. Re-exported so the input layer
/// can turn an `Interact` packet's target back into an entity.
#[must_use]
pub fn minecraft_id(entity: EntityView<'_>) -> i32 {
    entity.minecraft_id()
}
