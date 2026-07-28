//! Who the client thinks is online, and what they look like.
//!
//! A client will not render a player it has no profile for. `createEntityFromPacket`
//! in `ClientPacketListener` refuses an `AddEntity` of kind `player` whose
//! profile id is not already in `playerInfoMap`, logs "Server attempted to add
//! player prior to sending player info" and returns null, so before this module
//! existed no player could see another player at all: hyperion sent the entity
//! and never sent the tab list entry that gives it a face.
//!
//! Three things about the client's bookkeeping shape everything here.
//!
//! - `handlePlayerInfoUpdate` adds new entries with `putIfAbsent`, so re-sending
//!   `ADD_PLAYER` for a profile id it already knows is a no-op. Changing a
//!   profile means removing it first.
//! - `AbstractClientPlayer` caches the `PlayerInfo` it was built with, so a
//!   remote player also has to be dropped and re-added for a new profile to
//!   reach the renderer.
//! - `PlayerInfo.createSkinLookup` passes `requireSecure = !isLocalPlayer`, and
//!   `SkinManager.createLookup` then drops any skin whose
//!   `MinecraftProfileTextures.signatureState` is not `SIGNED`. An unsigned
//!   `textures` property is enough for a player to see their own skin and is
//!   silently ignored by everyone else.
//!
//! Verified against the 26.2 client jar (sha1 2dc72797acbc1b63fc16a11c4ac393605f453754)
//! and authlib 9.0.75, which ships inside the server bundle this repository
//! already pins.
//!
//! # A texture change never costs a player their world
//!
//! This module used to hand the wearer a `Respawn`, on the reasoning that
//! rebuilding `LocalPlayer` is the only thing that clears the cached
//! `PlayerInfo` in the third bullet above. It does, and it also throws away
//! everything else the client had: `handleRespawn` replaces `ClientLevel`
//! wholesale, so the player loses every chunk and every entity in one packet.
//!
//! Giving those back is not a matter of re-sending terrain. Chunks could be
//! restreamed from here, but entities could not: which entities a client is
//! subscribed to is the proxy's bookkeeping, keyed on position, and it has no
//! reason to re-offer a subscription the client already holds. So a respawn
//! left the wearer standing in an empty hub with no podiums, no mobs and
//! nobody else in it, and the operator saw the terrain half of that as
//! "Loading terrain..." forever.
//!
//! So [`refresh`] does not respawn. The wearer's profile entry is replaced,
//! which is what the tab list reads, and every *other* client drops and
//! re-adds their entity, which is what puts the new skin on the model that
//! matters. What is deliberately not fixed is the wearer's own first-person
//! view of themselves, which keeps the texture their `LocalPlayer` was built
//! with until something rebuilds it. That is one pair of arms against every
//! client's whole world, and it is the trade this file makes on purpose.

use flecs_ecs::{macros::system, prelude::*};
use hyperion_minecraft_proto::{
    Uuid as ProtoUuid,
    generated::packet_id::play::clientbound::PacketId,
    packets::{
        play::clientbound::{PlayerInfoRemove, RemoveEntities},
        play_login::GameEvent,
    },
};
use hyperion_utils::EntityExt;
use tracing::error;

use crate::{
    egress::{
        metadata::show_all,
        player_join::{PlayerInfoActions, PlayerList, PlayerListEntry, SkinProperty},
    },
    net::{Channel, Compose, ConnectionId, DataBundle, protocol::Clientbound},
    simulation::{
        Name, Pitch, Position, Uuid, Velocity, Yaw, add_entity,
        entity_kind::EntityKind,
        event,
        gamemode::{self, Gamemode},
        metadata::{MetadataChanges, get_and_clear_metadata},
        skin::PlayerSkin,
    },
    storage::EventQueue,
};

/// The tab list entry for one player, or `None` if the entity is not one.
///
/// The skin is optional and an absent one is not an error: an offline-mode
/// player has no Mojang profile to fetch, and a game that dresses its players
/// up sets [`PlayerSkin`] when it decides what they should look like rather
/// than at login.
#[must_use]
pub fn entry_of(entity: EntityView<'_>) -> Option<PlayerListEntry> {
    let (uuid, username) =
        entity.try_get::<(&Uuid, &Name)>(|(uuid, name)| (uuid.0, name.to_string()))?;

    let properties = entity
        .try_get::<&PlayerSkin>(skin_property)
        .flatten()
        .into_iter()
        .collect();

    Some(PlayerListEntry {
        uuid,
        username,
        properties,
        listed: true,
        ping: 0,
        game_mode: gamemode::of(entity).to_game_type(),
        // `None` makes the client fall back to the profile name, which is what
        // the player typed. Two players may share it; nothing on the wire is
        // keyed on a name, so that is a display question and not a protocol
        // one.
        display_name: None,
        list_order: 0,
        show_hat: true,
    })
}

/// A [`PlayerSkin`] as the profile property the client reads it from.
///
/// An empty signature becomes `None` rather than `Some("")`: authlib's
/// `Property.hasSignature` is a null check, so an empty string is a *present*
/// signature that fails validation, which is strictly worse than sending none.
fn skin_property(skin: &PlayerSkin) -> Option<SkinProperty> {
    if skin.textures.is_empty() {
        return None;
    }
    Some(SkinProperty {
        name: "textures".to_owned(),
        value: skin.textures.clone(),
        signature: (!skin.signature.is_empty()).then(|| skin.signature.clone()),
    })
}

/// Tell `entity` who is online, and tell everyone else about `entity`.
///
/// Called from the join sequence rather than from an observer because the
/// ordering is load bearing: the profile has to be on the client before the
/// first `AddEntity` naming it, and the joining player's own subscription to
/// other players' channels starts the moment [`Channel`] is added at the end of
/// [`crate::net::protocol::join::enter_world`].
///
/// # Errors
/// Returns an error when a packet fails to encode.
pub fn announce(
    world: &WorldRef<'_>,
    entity: EntityView<'_>,
    compose: &Compose,
    connection_id: ConnectionId,
) -> anyhow::Result<()> {
    let Some(joining) = entry_of(entity) else {
        anyhow::bail!("a player reached play state without a uuid and a name");
    };

    // Everyone already in the world, plus the joining player, in one packet.
    let mut roster = vec![joining.clone()];
    world
        .query::<()>()
        .with(id::<Channel>())
        .build()
        .each_entity(|other, ()| {
            if other.id() == entity.id() {
                return;
            }
            if let Some(entry) = entry_of(other) {
                roster.push(entry);
            }
        });

    let full = PlayerList::initialize(roster);
    compose.unicast(&full, connection_id)?;

    let one = PlayerList::initialize(vec![joining]);
    compose.broadcast(&one).exclude(connection_id).send()
}

/// Drop `uuid` from every client's tab list.
fn retire(compose: &Compose, uuid: uuid::Uuid) -> anyhow::Result<()> {
    let packet = PlayerInfoRemove(vec![ProtoUuid(uuid.as_u128())]);
    let clientbound = Clientbound::new(PacketId::PlayerInfoRemove.to_raw(), &packet);
    compose.broadcast(clientbound).send()
}

/// Dress `entity` in `skin`, unless it is already wearing it.
///
/// The comparison is on the profile property the client would receive and not
/// on the component, because those are not the same question. A player who
/// joins with no [`PlayerSkin`] at all and a player wearing [`PlayerSkin::EMPTY`]
/// publish exactly the same profile -- no `textures` property either way -- so
/// writing the second over the first changes nothing a client can observe and
/// must not be published as a change.
///
/// That distinction is the whole reason this function exists. `apply_skins`
/// hands every real client an empty skin a moment after it joins, because a
/// vanilla client sends its profile id and hyperion answers by asking Mojang
/// for a skin that an offline uuid does not have. Left as a plain `set`, that
/// no-op tripped [`refresh`] on every single join.
pub fn wear(entity: EntityView<'_>, skin: PlayerSkin) {
    let published = entity.try_get::<&PlayerSkin>(skin_property).flatten();
    if published == skin_property(&skin) {
        return;
    }
    entity.set(skin);
}

/// Re-send `entity`'s profile so a changed skin takes effect.
///
/// Two different sequences, because the client treats itself differently from
/// everyone else. Others are told to forget the profile and the entity and are
/// handed both again, which is what puts the new texture on the model. The
/// wearer is told to forget the profile and is handed it back, and that is all:
/// see this module's own documentation for why they are deliberately not
/// respawned and what that costs.
///
/// # Errors
/// Returns an error when a packet fails to encode.
fn refresh(entity: EntityView<'_>, compose: &Compose) -> anyhow::Result<()> {
    let Some(entry) = entry_of(entity) else {
        return Ok(());
    };
    let Some(connection_id) = entity.try_get::<&ConnectionId>(|id| *id) else {
        return Ok(());
    };
    let uuid = entry.uuid;
    let minecraft_id = entity.minecraft_id();

    let remove_info = PlayerInfoRemove(vec![ProtoUuid(uuid.as_u128())]);
    let add_info = PlayerList::initialize(vec![entry]);

    let mut mine = DataBundle::new(compose);
    mine.add_packet(Clientbound::new(
        PacketId::PlayerInfoRemove.to_raw(),
        &remove_info,
    ))?;
    mine.add_packet(&add_info)?;
    mine.unicast(connection_id)?;

    let mut theirs = DataBundle::new(compose);
    theirs.add_packet(Clientbound::new(
        PacketId::PlayerInfoRemove.to_raw(),
        &remove_info,
    ))?;
    theirs.add_packet(&add_info)?;
    theirs.add_packet(Clientbound::new(
        PacketId::RemoveEntities.to_raw(),
        &RemoveEntities(vec![minecraft_id]),
    ))?;

    let found = entity.try_get::<(&Position, &Pitch, &Yaw, &Velocity)>(
        |(position, pitch, yaw, velocity)| -> anyhow::Result<()> {
            let spawn = add_entity(
                minecraft_id,
                EntityKind::Player,
                &Uuid::from(uuid),
                position,
                *pitch,
                *yaw,
                velocity,
            );
            theirs.add_packet(Clientbound::new(PacketId::AddEntity.to_raw(), &spawn))?;
            theirs.add_packet(Clientbound::new(
                PacketId::SetEntityData.to_raw(),
                &show_all(minecraft_id),
            ))
        },
    );
    match found {
        Some(result) => result?,
        None => anyhow::bail!("a visible player is missing its position components"),
    }

    let mut metadata = MetadataChanges::default();
    metadata.encode_non_default_components(entity);
    if let Some(view) = get_and_clear_metadata(&mut metadata) {
        theirs.add_packet(Clientbound::new(
            PacketId::SetEntityData.to_raw(),
            &hyperion_minecraft_proto::packets::play::entity::SetEntityData {
                id: minecraft_id,
                packed_items: &view,
            },
        ))?;
    }

    theirs.broadcast(Some(connection_id))
}

/// Tell `entity` and everyone watching them that their gamemode changed.
///
/// The `GameEvent` is what actually flips the client's own abilities; the tab
/// list entry is what keeps the little icon next to their name honest. Both,
/// because either alone leaves the client and the server disagreeing about
/// something the server enforces.
fn publish_gamemode(entity: EntityView<'_>, compose: &Compose) -> anyhow::Result<()> {
    let Some(entry) = entry_of(entity) else {
        return Ok(());
    };
    let mode = gamemode::of(entity);

    if let Some(connection_id) = entity.try_get::<&ConnectionId>(|id| *id) {
        compose.unicast(
            Clientbound::new(PacketId::GameEvent.to_raw(), &GameEvent {
                event: GameEvent::CHANGE_GAME_MODE,
                param: f32::from(mode.to_game_type().to_id()),
            }),
            connection_id,
        )?;
    }

    let update = PlayerList {
        actions: PlayerInfoActions::UPDATE_GAME_MODE,
        entries: vec![entry],
    };
    compose.broadcast(&update).send()
}

/// Keeps the tab list and the gamemode in step with the ECS.
#[derive(Component)]
pub struct RosterModule;

impl Module for RosterModule {
    fn module(world: &World) {
        // Only players already visible are refreshed. A skin that lands during
        // login is picked up by [`announce`] instead, which is both cheaper and
        // the only order that works: there is nothing to remove yet.
        world
            .observer::<flecs::OnSet, &PlayerSkin>()
            .with(id::<Channel>())
            .each_entity(|entity, _| {
                entity.world().get::<&Compose>(|compose| {
                    if let Err(error) = refresh(entity, compose) {
                        error!("failed to re-send a player's profile: {error}");
                    }
                });
            });

        // A flecs enum is an exclusive pair, so a change arrives as an add of
        // the new constant rather than as a set of a value.
        world
            .observer::<flecs::OnAdd, ()>()
            .with((id::<Gamemode>(), id::<flecs::Wildcard>()))
            .with(id::<Channel>())
            .each_entity(|entity, ()| {
                entity.world().get::<&Compose>(|compose| {
                    if let Err(error) = publish_gamemode(entity, compose) {
                        error!("failed to publish a gamemode change: {error}");
                    }
                });
            });

        // `SetSkin` used to be handled by a copy of the refresh sequence living
        // in the bedwars crate, which is how the two drifted: that copy sent a
        // profile with the literal username "Player" and no gamemode. Setting
        // the component instead means one refresh path, and it is the same one
        // a game that assigns skins directly already goes through.
        system!("apply_set_skin", world, &mut EventQueue<event::SetSkin>)
            .kind(id::<flecs::pipeline::PreUpdate>())
            .each_iter(|it, _, queue| {
                let world = it.world();
                for event::SetSkin { skin, by } in queue.drain() {
                    let Some(entity) = world.try_get_alive(by) else {
                        continue;
                    };
                    wear(entity, skin);
                }
            });

        world
            .observer::<flecs::OnRemove, &Uuid>()
            .with_enum(crate::simulation::PacketState::Play)
            .each_entity(|entity, uuid| {
                let uuid = uuid.0;
                entity.world().get::<&Compose>(|compose| {
                    if let Err(error) = retire(compose, uuid) {
                        error!("failed to remove a player from the tab list: {error}");
                    }
                });
            });
    }
}
