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
//! which Minecraft particle and item each one becomes is a hosting decision, so
//! the mapping lives here and nowhere under `src/module/`. [`Sound`] is the
//! exception and arrives already naming a vanilla sound event, because which
//! noise an ability makes is part of what the ability *is* and belongs with the
//! kit that declares it; all that is left here is the encoding.

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
            chunk::{LevelParticles, Particle},
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
        PendingTeleportation, Velocity,
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
        BarColour, BossBar, Channel, Cue, Experience, HotbarItem, PlayerId, Server, SidebarLine,
        Sound, SoundCategory, Text, Title,
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
    Play {
        at: Vec3,
        cue: Cue,
    },
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

    fn cue(&self, at: Vec3, cue: Cue) {
        self.push(Op::Play { at, cue });
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

    fn set_boss_bar(&self, player: PlayerId, bar: BossBar) {
        self.push(Op::SetBossBar {
            player: entity_of(player),
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
        world.component::<HudBar>();

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
            .observer::<flecs::OnAdd, ()>()
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
            .system_named::<(&OpQueue, &Compose)>("smash::apply_server_ops")
            .kind(id::<flecs::pipeline::PostUpdate>())
            .each_iter(|it, _, (queue, compose)| {
                let world = it.world();
                let drained =
                    std::mem::take(&mut *queue.0.lock().expect("server op queue poisoned"));
                // Every player's bar, resolved before any op is applied.
                //
                // `world.entity()` hands back its id at once but the
                // `set(HudBar)` that records it is deferred to the merge, so
                // two `SetBossBar` ops for one player in one drain would each
                // see no bar and mint one. Resolving them up front, in one
                // map, is what makes a second bar impossible rather than
                // merely unlikely.
                let mut bars = HashMap::new();
                for op in &drained {
                    if let Op::SetBossBar { player, .. } = op
                        && let Entry::Vacant(slot) = bars.entry(*player)
                        && let Some(bar) = hud_bar(world, *player)
                    {
                        slot.insert(bar);
                    }
                }
                for op in drained {
                    apply(world, compose, op, &bars);
                }
            });
    }
}

fn apply(world: WorldRef<'_>, compose: &Compose, op: Op, bars: &HashMap<Entity, Entity>) {
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
        Op::Play { at, cue } => play_cue(compose, at, cue),
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
        Op::SetBossBar { player, bar } => {
            let Some(entity) = bars.get(&player) else {
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

/// Which boss bar entity is this player's, making it if they have none.
///
/// The bar this game draws is per player -- it carries a percentage that is
/// only true of the person reading it -- so its audience is one edge,
/// `(ShownTo, player)`. Everything after that is `egress::boss_bar`'s: the
/// `Add` on the first push, one operation per field that moves after it, and
/// the `Remove` when the player leaves.
///
/// A child of the player, so it dies with them under flecs's own
/// `(ChildOf, OnDeleteTarget, Delete)`. Without that the bar entity would
/// outlive its only viewer with an empty audience forever, which on a server
/// nobody restarts is a leak measured in players seen.
///
/// No fog, no darkened sky and no boss music, which is the default `Effects`:
/// each of them changes how the arena looks or sounds, and this bar is a
/// readout rather than an event.
fn hud_bar(world: WorldRef<'_>, player: Entity) -> Option<Entity> {
    let player = world.entity_from_id(player);
    if !player.is_alive() {
        return None;
    }
    if let Some(bar) = player.try_get::<&HudBar>(|bar| bar.0)
        && world.entity_from_id(bar).is_alive()
    {
        return Some(bar);
    }
    let bar = world
        .entity()
        .child_of(player)
        .add(id::<boss_bar::BossBar>())
        .add((id::<boss_bar::ShownTo>(), player))
        .id();
    player.set(HudBar(bar));
    Some(bar)
}

/// The boss bar entity a player's HUD writes to.
#[derive(Component, Debug, Copy, Clone)]
struct HudBar(Entity);

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
        id: DisplaySlot::Sidebar.to_id(),
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

/// The game's three cues, as Minecraft particles.
///
/// Sound used to be decided here as well, from a six-variant enum, which is why
/// every ability in the game shared four noises between them. Audio now arrives
/// already named: the game picks the vanilla sound event, because which sound
/// an ability makes is a design decision belonging to the kit that declares it,
/// and only the encoding is a hosting one. What is still a hosting decision,
/// and so still here, is which particle each cue draws.
///
/// `[INFERRED]` throughout: Mineplex's own choices are not in the leaked source,
/// which loaded them from the same spreadsheet as everything else.
/// Half-width of the box a cue's particles are scattered through, in blocks.
const CUE_SPREAD: f32 = 0.4;

fn play_cue(compose: &Compose, at: Vec3, cue: Cue) {
    let particle = match cue {
        Cue::Explosion => Particle::Explosion,
        Cue::Teleport => Particle::Portal,
        Cue::Death => Particle::Cloud,
        // `[PLACEHOLDER]` both. Vanilla draws a burn with `minecraft:flame` and
        // a poison with `minecraft:entity_effect`, and
        // `hyperion_minecraft_proto::packets::play::chunk::Particle` carries
        // neither yet, so each is pinned to the nearest thing it does carry:
        // `crit`'s orange sparks read as "this is hurting you", and dragon
        // breath is already vanilla's own lingering harmful cloud. Swap both
        // the moment the proto grows the real ones; this comment is the fossil
        // to remove, not a design decision to keep.
        Cue::Burn => Particle::Crit,
        // Half power, because a full-strength breath puff is a wall of purple
        // and this marks one point of damage.
        Cue::Venom => Particle::DragonBreath { power: 0.5 },
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
