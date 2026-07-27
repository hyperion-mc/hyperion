//! The tab list.
//!
//! `PlayerInfoUpdate` is the one player-facing packet whose codec the generator
//! declined: one action bitmask for the whole packet, then entries carrying
//! exactly the fields those actions name and nothing to say how long an entry
//! is. [`hyperion_minecraft_proto::packets::play::player`] has the codec; what
//! is here is the owned shape hyperion builds one from, because the names and
//! skin properties come out of the ECS rather than out of a buffer that
//! outlives the send.

use std::io::Write;

// Re-exported so a caller building a tab list update names the action set
// without depending on the proto crate directly.
pub use hyperion_minecraft_proto::packets::play::player::PlayerInfoActions;
use hyperion_minecraft_proto::{
    Uuid as ProtoUuid,
    generated::packet_id::play::clientbound::PacketId,
    packets::{
        play::player::{PlayerInfoEntry, PlayerInfoUpdate, PlayerProfile},
        play_login::GameType,
    },
    text::Component,
    types::game_profile::Property,
};
use uuid::Uuid;

use crate::{PacketBundle, net::protocol::Clientbound};

/// One signed profile property, in practice only ever `textures`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkinProperty {
    /// Property name.
    pub name: String,
    /// Property value, base64 for `textures`.
    pub value: String,
    /// Mojang's signature over the value, absent on an offline-mode profile.
    pub signature: Option<String>,
}

/// One player in the tab list.
///
/// A field only reaches the wire when the packet's actions name it, so a value
/// here that [`PlayerList::actions`] does not select is ignored rather than
/// sent as a default.
#[derive(Clone, Debug, Default)]
pub struct PlayerListEntry {
    /// The player this entry is about.
    pub uuid: Uuid,
    /// Profile name, for [`PlayerInfoActions::ADD_PLAYER`].
    pub username: String,
    /// Skin and cape, for [`PlayerInfoActions::ADD_PLAYER`].
    pub properties: Vec<SkinProperty>,
    /// Whether the player appears in the list at all, for
    /// [`PlayerInfoActions::UPDATE_LISTED`].
    pub listed: bool,
    /// Round trip time in milliseconds, for
    /// [`PlayerInfoActions::UPDATE_LATENCY`].
    pub ping: i32,
    /// Game mode, for [`PlayerInfoActions::UPDATE_GAME_MODE`].
    pub game_mode: GameType,
    /// Name shown in the list, for [`PlayerInfoActions::UPDATE_DISPLAY_NAME`].
    /// `None` falls back to [`username`](Self::username).
    pub display_name: Option<String>,
    /// Sort key, for [`PlayerInfoActions::UPDATE_LIST_ORDER`].
    pub list_order: i32,
    /// Whether the hat model part is drawn, for
    /// [`PlayerInfoActions::UPDATE_HAT`].
    pub show_hat: bool,
}

/// A tab list update: which fields to send, and for whom.
#[derive(Clone, Debug, Default)]
pub struct PlayerList {
    /// Which fields every entry carries.
    pub actions: PlayerInfoActions,
    /// The players this update is about.
    pub entries: Vec<PlayerListEntry>,
}

impl PlayerList {
    /// Everything a client needs to show a player it has never seen, which is
    /// the action set `createPlayerInitializing` uses minus the chat session
    /// hyperion does not sign.
    #[must_use]
    pub const fn initialize(entries: Vec<PlayerListEntry>) -> Self {
        Self {
            actions: PlayerInfoActions::ADD_PLAYER
                .union(PlayerInfoActions::UPDATE_GAME_MODE)
                .union(PlayerInfoActions::UPDATE_LISTED)
                .union(PlayerInfoActions::UPDATE_LATENCY)
                .union(PlayerInfoActions::UPDATE_DISPLAY_NAME)
                .union(PlayerInfoActions::UPDATE_LIST_ORDER)
                .union(PlayerInfoActions::UPDATE_HAT),
            entries,
        }
    }
}

impl PacketBundle for &PlayerList {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()> {
        // The display names are components only for as long as this call
        // takes, because `Component` borrows the string it renders.
        let display_names: Vec<Option<Component<'_>>> = self
            .entries
            .iter()
            .map(|entry| entry.display_name.as_deref().map(Component::text))
            .collect();

        let entries = self
            .entries
            .iter()
            .zip(&display_names)
            .map(|(entry, display_name)| PlayerInfoEntry {
                profile_id: ProtoUuid(entry.uuid.as_u128()),
                profile: Some(PlayerProfile {
                    name: entry.username.as_str(),
                    properties: entry
                        .properties
                        .iter()
                        .map(|property| Property {
                            name: property.name.as_str(),
                            value: property.value.as_str(),
                            signature: property.signature.as_deref(),
                        })
                        .collect(),
                }),
                // hyperion does not sign chat, so it never has a session to
                // send; `INITIALIZE_CHAT` would write a bare `false` here.
                chat_session: None,
                game_mode: entry.game_mode,
                listed: entry.listed,
                latency: entry.ping,
                display_name: display_name.clone(),
                list_order: entry.list_order,
                show_hat: entry.show_hat,
            })
            .collect();

        let body = PlayerInfoUpdate {
            actions: self.actions,
            entries,
        };
        Clientbound::new(PacketId::PlayerInfoUpdate.to_raw(), &body).encode_including_ids(w)
    }
}
