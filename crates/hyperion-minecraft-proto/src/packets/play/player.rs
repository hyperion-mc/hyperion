//! The player list, chat, abilities and status.
//!
//! Everything here branches on a runtime value somewhere in its codec, which
//! is why the generator declined it. Three shapes recur:
//!
//! - a bitmask that selects which fields follow ([`PlayerInfoUpdate`],
//!   [`PlayerAbilities`], [`Commands`]),
//! - a method byte that selects a whole tail ([`SetObjective`],
//!   [`SetPlayerTeam`]),
//! - a leading discriminant that selects a variant ([`BossEvent`],
//!   [`NumberFormat`]).
//!
//! The packets whose layout *is* mechanical are generated, not repeated here:
//! `PlayerInfoRemove`, `TabList`, `SystemChat`, `Sound`, `SoundEntity`,
//! `SetHealth`, `SetExperience`, `SetTitleText`, `SetSubtitleText`,
//! `SetActionBarText`, `SetTitlesAnimation` and `SetDisplayObjective` all live
//! in [`crate::packets::play::clientbound`]. One name appears in both places
//! and only one of them is real: the generated `clientbound::BossEvent` is an
//! empty struct, because the extractor found no composed fields under
//! `Packet.codec(write, new)` and emitted a unit. [`BossEvent`] here is the
//! packet.
//!
//! # Styles ride as raw NBT
//!
//! `Style.Serializer.TRUSTED_STREAM_CODEC` is `fromCodecWithRegistriesTrusted`,
//! so a `Style` on the wire is a network NBT compound and nothing else.
//! [`crate::text::Style`] has no public `Encode` yet, so the two places a bare
//! style appears -- [`ChatTypeDecoration::style`] and [`NumberFormat::Styled`]
//! -- carry an [`nbt::Tag`]. `Tag::Compound(Compound::new())` is `Style.EMPTY`.
//! Giving `Style` its own codec would replace both, and is the follow-up.

use crate::{
    Decode, Encode, Error, Holder, Identifier, Reader, RegistryId, Result, Uuid, Writer,
    codec::{read_count, write_count},
    generated::registry,
    nbt,
    packets::play_login::GameType,
    text::Component,
    types::game_profile::Property,
};

// --- player list ----------------------------------------------------------

/// Which fields every entry of a [`PlayerInfoUpdate`] carries
/// (`ClientboundPlayerInfoUpdatePacket.Action`), as a bitmask.
///
/// `FriendlyByteBuf.writeEnumSet` writes one bit per enum constant, least
/// significant bit first, rounded up to whole bytes. `Action` has eight
/// constants, so the mask is exactly one byte and [`PlayerInfoUpdate`] writes
/// it as one; a ninth action would silently widen it to two and shift every
/// entry, which the empty-packet wire test is there to catch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct PlayerInfoActions(u8);

impl PlayerInfoActions {
    /// The entry carries a name and profile properties.
    pub const ADD_PLAYER: Self = Self(1 << 0);
    /// Every defined bit.
    pub const ALL: Self = Self(0xff);
    /// The entry carries a chat session, which is what signed chat is keyed on.
    pub const INITIALIZE_CHAT: Self = Self(1 << 1);
    /// No actions, i.e. entries that are nothing but a uuid.
    pub const NONE: Self = Self(0);
    /// The entry carries a display name.
    pub const UPDATE_DISPLAY_NAME: Self = Self(1 << 5);
    /// The entry carries a game mode.
    pub const UPDATE_GAME_MODE: Self = Self(1 << 2);
    /// The entry carries whether the hat model part is shown.
    pub const UPDATE_HAT: Self = Self(1 << 7);
    /// The entry carries a ping in milliseconds.
    pub const UPDATE_LATENCY: Self = Self(1 << 4);
    /// The entry carries whether the player appears in the tab list at all.
    pub const UPDATE_LISTED: Self = Self(1 << 3);
    /// The entry carries a sort key for the tab list.
    pub const UPDATE_LIST_ORDER: Self = Self(1 << 6);

    /// The mask with `other`'s bits also set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        self.0
    }

    /// Read a wire byte.
    ///
    /// Every bit is defined in 776, so nothing is dropped here; the constructor
    /// exists so that a future version with fewer actions has one place to
    /// mask.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }
}

/// The profile fields `ADD_PLAYER` writes.
///
/// Not a [`crate::types::GameProfile`]: the profile id is the entry's own
/// [`PlayerInfoEntry::profile_id`], already on the wire ahead of the actions,
/// so `ADD_PLAYER` writes the name and the properties alone.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerProfile<'a> {
    /// Player name, at most 16 UTF-16 code units (`ByteBufCodecs.PLAYER_NAME`).
    pub name: &'a str,
    /// Signed profile properties, chiefly `textures`.
    pub properties: Vec<Property<'a>>,
}

impl PlayerProfile<'_> {
    /// `ByteBufCodecs.PLAYER_NAME` is `stringUtf8(16)`.
    pub const MAX_NAME_LEN: usize = 16;
    /// `ByteBufCodecs.GAME_PROFILE_PROPERTIES`' own count limit.
    pub const MAX_PROPERTIES: usize = 16;
}

impl Encode for PlayerProfile<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string_with_limit(self.name, Self::MAX_NAME_LEN)?;
        write_count(writer, self.properties.len(), Some(Self::MAX_PROPERTIES))?;
        for property in &self.properties {
            property.encode(writer)?;
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for PlayerProfile<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let name = reader.string_with_limit(Self::MAX_NAME_LEN)?;
        let count = read_count(reader, Some(Self::MAX_PROPERTIES))?;
        let mut properties = Vec::with_capacity(count.min(reader.remaining_len()));
        for _ in 0..count {
            properties.push(Property::decode(reader)?);
        }
        Ok(Self { name, properties })
    }
}

/// A player's signing key and the session it belongs to
/// (`ProfilePublicKey.Data`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfilePublicKeyData<'a> {
    /// Expiry as Unix milliseconds (`FriendlyByteBuf.writeInstant`).
    pub expires_at: i64,
    /// X.509 encoding of the RSA public key, at most 512 bytes.
    pub key: &'a [u8],
    /// Mojang's signature over the key, at most 4096 bytes.
    pub key_signature: &'a [u8],
}

impl ProfilePublicKeyData<'_> {
    /// `FriendlyByteBuf.readPublicKey`'s limit.
    pub const MAX_KEY_LEN: usize = 512;
    /// `ProfilePublicKey.Data.MAX_KEY_SIGNATURE_SIZE`.
    pub const MAX_SIGNATURE_LEN: usize = 4096;
}

impl Encode for ProfilePublicKeyData<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.expires_at);
        writer.byte_array_with_limit(self.key, Self::MAX_KEY_LEN)?;
        writer.byte_array_with_limit(self.key_signature, Self::MAX_SIGNATURE_LEN)
    }
}

impl<'a> Decode<'a> for ProfilePublicKeyData<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            expires_at: reader.i64()?,
            key: reader.byte_array_with_limit(Self::MAX_KEY_LEN)?,
            key_signature: reader.byte_array_with_limit(Self::MAX_SIGNATURE_LEN)?,
        })
    }
}

/// A chat session, which is what a signed message chain is validated against
/// (`RemoteChatSession.Data`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChatSession<'a> {
    /// Session id, which changes every time the player reconnects.
    pub session_id: Uuid,
    /// The key messages in this session are signed with.
    pub public_key: ProfilePublicKeyData<'a>,
}

impl Encode for ChatSession<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.session_id.encode(writer)?;
        self.public_key.encode(writer)
    }
}

impl<'a> Decode<'a> for ChatSession<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            session_id: Uuid::decode(reader)?,
            public_key: ProfilePublicKeyData::decode(reader)?,
        })
    }
}

/// One player in a [`PlayerInfoUpdate`]
/// (`ClientboundPlayerInfoUpdatePacket.Entry`).
///
/// Every field but [`profile_id`](Self::profile_id) is written only when the
/// packet's action bit for it is set, so a value here that no action selects is
/// never sent. The three `Option` fields are the ones that are nullable on the
/// wire *as well*: their action being set still permits an absent value, except
/// for [`profile`](Self::profile), which `Action.ADD_PLAYER` requires.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerInfoEntry<'a> {
    /// The player this entry is about.
    pub profile_id: Uuid,
    /// Name and skin, for [`PlayerInfoActions::ADD_PLAYER`].
    pub profile: Option<PlayerProfile<'a>>,
    /// Signing key, for [`PlayerInfoActions::INITIALIZE_CHAT`].
    pub chat_session: Option<ChatSession<'a>>,
    /// Game mode, for [`PlayerInfoActions::UPDATE_GAME_MODE`].
    pub game_mode: GameType,
    /// Whether the player is in the tab list, for
    /// [`PlayerInfoActions::UPDATE_LISTED`].
    pub listed: bool,
    /// Ping in milliseconds, for [`PlayerInfoActions::UPDATE_LATENCY`].
    pub latency: i32,
    /// Tab list name, for [`PlayerInfoActions::UPDATE_DISPLAY_NAME`]. `None`
    /// means the client falls back to the profile name.
    pub display_name: Option<Component<'a>>,
    /// Tab list sort key, for [`PlayerInfoActions::UPDATE_LIST_ORDER`].
    pub list_order: i32,
    /// Whether the hat model part is shown, for
    /// [`PlayerInfoActions::UPDATE_HAT`].
    pub show_hat: bool,
}

impl PlayerInfoEntry<'_> {
    /// Write the uuid and then each field the actions select, in action order.
    ///
    /// The order is `Action`'s declaration order and not the order the actions
    /// were named in, because Java iterates an `EnumSet`, which is ordinal
    /// ordered. Writing them in any other order desynchronises the reader
    /// without any length to catch it.
    fn encode_with(&self, actions: PlayerInfoActions, writer: &mut Writer) -> Result<()> {
        self.profile_id.encode(writer)?;
        if actions.contains(PlayerInfoActions::ADD_PLAYER) {
            // `Objects.requireNonNull(entry.profile())`: a packet whose actions
            // promise a profile and whose entry has none cannot be written at
            // all, so this is an error rather than an empty profile.
            let profile = self
                .profile
                .as_ref()
                .ok_or(Error::MissingField("player_info_update entry profile"))?;
            profile.encode(writer)?;
        }
        if actions.contains(PlayerInfoActions::INITIALIZE_CHAT) {
            self.chat_session.encode(writer)?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_GAME_MODE) {
            writer.var_int(i32::from(self.game_mode.to_id()));
        }
        if actions.contains(PlayerInfoActions::UPDATE_LISTED) {
            writer.bool(self.listed);
        }
        if actions.contains(PlayerInfoActions::UPDATE_LATENCY) {
            writer.var_int(self.latency);
        }
        if actions.contains(PlayerInfoActions::UPDATE_DISPLAY_NAME) {
            self.display_name.encode(writer)?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_LIST_ORDER) {
            writer.var_int(self.list_order);
        }
        if actions.contains(PlayerInfoActions::UPDATE_HAT) {
            writer.bool(self.show_hat);
        }
        Ok(())
    }
}

impl<'a> PlayerInfoEntry<'a> {
    /// Read one entry, taking the same fields [`Self::encode_with`] wrote.
    fn decode_with(actions: PlayerInfoActions, reader: &mut Reader<'a>) -> Result<Self> {
        let mut entry = Self {
            profile_id: Uuid::decode(reader)?,
            ..Self::default()
        };
        if actions.contains(PlayerInfoActions::ADD_PLAYER) {
            entry.profile = Some(PlayerProfile::decode(reader)?);
        }
        if actions.contains(PlayerInfoActions::INITIALIZE_CHAT) {
            entry.chat_session = Option::decode(reader)?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_GAME_MODE) {
            let id = reader.var_int()?;
            entry.game_mode = GameType::from_id(i8::try_from(id).unwrap_or(0));
        }
        if actions.contains(PlayerInfoActions::UPDATE_LISTED) {
            entry.listed = reader.bool()?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_LATENCY) {
            entry.latency = reader.var_int()?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_DISPLAY_NAME) {
            entry.display_name = Option::decode(reader)?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_LIST_ORDER) {
            entry.list_order = reader.var_int()?;
        }
        if actions.contains(PlayerInfoActions::UPDATE_HAT) {
            entry.show_hat = reader.bool()?;
        }
        Ok(entry)
    }
}

/// `minecraft:player_info_update`, clientbound
/// (`ClientboundPlayerInfoUpdatePacket`).
///
/// One action set for the whole packet, then a list of entries each of which
/// carries exactly the fields those actions name. Nothing in the body says how
/// long an entry is, so a reader that disagrees with the writer about the
/// action order loses the rest of the packet rather than one field.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlayerInfoUpdate<'a> {
    /// Which fields every entry carries.
    pub actions: PlayerInfoActions,
    /// The players this packet is about.
    pub entries: Vec<PlayerInfoEntry<'a>>,
}

impl Encode for PlayerInfoUpdate<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(self.actions.to_bits());
        write_count(writer, self.entries.len(), None)?;
        for entry in &self.entries {
            entry.encode_with(self.actions, writer)?;
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for PlayerInfoUpdate<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let actions = PlayerInfoActions::from_bits(reader.u8()?);
        let count = read_count(reader, None)?;
        // An entry is at least a uuid, so a count needing more bytes than
        // remain cannot be honest and must not drive a reservation.
        let mut entries = Vec::with_capacity(count.min(reader.remaining_len() / 16));
        for _ in 0..count {
            entries.push(PlayerInfoEntry::decode_with(actions, reader)?);
        }
        Ok(Self { actions, entries })
    }
}

// --- disguised chat -------------------------------------------------------

/// Which value a chat decoration substitutes into its translation
/// (`ChatTypeDecoration.Parameter`), as a `VarInt` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum ChatTypeParameter {
    /// The sender's display name.
    Sender = 0,
    /// The recipient's display name, for direct and team messages.
    Target = 1,
    /// The message itself.
    Content = 2,
}

/// How one chat type renders (`ChatTypeDecoration`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTypeDecoration<'a> {
    /// Translation key, e.g. `chat.type.text`.
    pub translation_key: &'a str,
    /// Which values fill the translation's slots, in order.
    pub parameters: Vec<ChatTypeParameter>,
    /// Style applied to the whole decorated message, as network NBT.
    /// `Tag::Compound(Compound::new())` is `Style.EMPTY`.
    pub style: nbt::Tag<'a>,
}

impl Encode for ChatTypeDecoration<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.translation_key)?;
        self.parameters.encode(writer)?;
        self.style.encode(writer)
    }
}

impl<'a> Decode<'a> for ChatTypeDecoration<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            translation_key: reader.string()?,
            parameters: Vec::decode(reader)?,
            style: nbt::Tag::decode(reader)?,
        })
    }
}

/// A chat type written out in full rather than referenced by registry id
/// (`ChatType`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTypeDirect<'a> {
    /// How the message appears in the chat box.
    pub chat: ChatTypeDecoration<'a>,
    /// How the message is read out by the narrator.
    pub narration: ChatTypeDecoration<'a>,
}

impl Encode for ChatTypeDirect<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.chat.encode(writer)?;
        self.narration.encode(writer)
    }
}

impl<'a> Decode<'a> for ChatTypeDirect<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            chat: ChatTypeDecoration::decode(reader)?,
            narration: ChatTypeDecoration::decode(reader)?,
        })
    }
}

/// A chat type together with the names it decorates with (`ChatType.Bound`).
#[derive(Debug, Clone, PartialEq)]
pub struct ChatTypeBound<'a> {
    /// A `minecraft:chat_type` entry, by id or written out in full.
    pub chat_type: Holder<ChatTypeDirect<'a>>,
    /// The sender's display name.
    pub name: Component<'a>,
    /// The recipient's display name, for the decorations that use one.
    pub target_name: Option<Component<'a>>,
}

impl Encode for ChatTypeBound<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.chat_type.encode(writer)?;
        self.name.encode(writer)?;
        self.target_name.encode(writer)
    }
}

impl<'a> Decode<'a> for ChatTypeBound<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            chat_type: Holder::decode(reader)?,
            name: Component::decode(reader)?,
            target_name: Option::decode(reader)?,
        })
    }
}

/// `minecraft:disguised_chat`, clientbound (`ClientboundDisguisedChatPacket`).
///
/// A chat message the server vouches for rather than one the sender signed, so
/// it carries no signature and no message index. The generator declined it
/// because `ChatType.Bound.STREAM_CODEC` names `ChatType.STREAM_CODEC`, which
/// is a `Holder` over the codec containing it.
#[derive(Debug, Clone, PartialEq)]
pub struct DisguisedChat<'a> {
    /// The message body, before decoration.
    pub message: Component<'a>,
    /// Which chat type decorates it, and with whose names.
    pub chat_type: ChatTypeBound<'a>,
}

impl Encode for DisguisedChat<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.message.encode(writer)?;
        self.chat_type.encode(writer)
    }
}

impl<'a> Decode<'a> for DisguisedChat<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            message: Component::decode(reader)?,
            chat_type: ChatTypeBound::decode(reader)?,
        })
    }
}

// --- abilities ------------------------------------------------------------

/// What a player is allowed to do (`ClientboundPlayerAbilitiesPacket`'s
/// bitfield), as a bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct AbilityFlags(i8);

impl AbilityFlags {
    /// Every defined bit.
    pub const ALL: Self = Self(0b1111);
    /// The player may fly (`FLAG_CAN_FLY`).
    pub const CAN_FLY: Self = Self(4);
    /// The player is flying right now (`FLAG_FLYING`).
    pub const FLYING: Self = Self(2);
    /// Blocks break instantly (`FLAG_INSTABUILD`).
    pub const INSTABUILD: Self = Self(8);
    /// Damage is ignored (`FLAG_INVULNERABLE`).
    pub const INVULNERABLE: Self = Self(1);
    /// No abilities, i.e. plain survival.
    pub const NONE: Self = Self(0);

    /// The mask with `other`'s bits also set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_bits(self) -> i8 {
        self.0
    }

    /// Read a wire byte, dropping the four undefined bits the way the client's
    /// own `(bitfield & FLAG) != 0` tests do.
    #[must_use]
    pub const fn from_bits(bits: i8) -> Self {
        Self(bits & Self::ALL.0)
    }
}

/// `minecraft:player_abilities`, clientbound
/// (`ClientboundPlayerAbilitiesPacket`).
///
/// The speeds are the ones the client applies directly; they are not
/// multipliers over an attribute. Vanilla sends `0.05` and `0.1`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PlayerAbilities {
    /// Invulnerability, flight permission, flight state and instant mining.
    pub flags: AbilityFlags,
    /// Flying speed in blocks per tick.
    pub flying_speed: f32,
    /// Walking speed in blocks per tick.
    pub walking_speed: f32,
}

impl Encode for PlayerAbilities {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i8(self.flags.to_bits());
        writer.f32(self.flying_speed);
        writer.f32(self.walking_speed);
        Ok(())
    }
}

impl Decode<'_> for PlayerAbilities {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            flags: AbilityFlags::from_bits(reader.i8()?),
            flying_speed: reader.f32()?,
            walking_speed: reader.f32()?,
        })
    }
}

// --- command tree ---------------------------------------------------------

/// How much of a string one `brigadier:string` argument consumes
/// (`StringArgumentType.StringType`), as a `VarInt` ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Encode, Decode)]
pub enum StringArgumentKind {
    /// One whitespace-delimited word.
    SingleWord = 0,
    /// One word, or everything inside a pair of quotes.
    QuotablePhrase = 1,
    /// The rest of the command line.
    GreedyPhrase = 2,
}

/// An argument type and the properties the client needs to build it
/// (`ArgumentTypeInfo`).
///
/// Forty-four of the fifty-seven registered types are a `SingletonArgumentInfo`
/// and write nothing after their id; they are [`Self::Empty`]. The thirteen
/// that do carry properties have a variant each, because a codec that skipped
/// them would lose the rest of the command tree: nothing here is
/// length-prefixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ArgumentType<'a> {
    /// A type whose `ArgumentTypeInfo` writes no properties, held by its
    /// network id in `minecraft:command_argument_type`.
    Empty(RegistryId),
    /// `brigadier:double`, with optional bounds.
    Double {
        /// Lower bound; `None` is `Double.MIN_VALUE`.
        min: Option<f64>,
        /// Upper bound; `None` is `Double.MAX_VALUE`.
        max: Option<f64>,
    },
    /// `minecraft:entity`, a target selector.
    Entity {
        /// Whether the selector must resolve to exactly one entity.
        single: bool,
        /// Whether it may only match players.
        players_only: bool,
    },
    /// `brigadier:float`, with optional bounds.
    Float {
        /// Lower bound; `None` is `-Float.MAX_VALUE`.
        min: Option<f32>,
        /// Upper bound; `None` is `Float.MAX_VALUE`.
        max: Option<f32>,
    },
    /// `brigadier:integer`, with optional bounds.
    Integer {
        /// Lower bound; `None` is `Integer.MIN_VALUE`.
        min: Option<i32>,
        /// Upper bound; `None` is `Integer.MAX_VALUE`.
        max: Option<i32>,
    },
    /// `brigadier:long`, with optional bounds.
    Long {
        /// Lower bound; `None` is `Long.MIN_VALUE`.
        min: Option<i64>,
        /// Upper bound; `None` is `Long.MAX_VALUE`.
        max: Option<i64>,
    },
    /// One of the five types scoped to a registry: `minecraft:resource`,
    /// `resource_key`, `resource_or_tag`, `resource_or_tag_key` and
    /// `resource_selector`. Which one is the id; the payload is the registry.
    Registry {
        /// Network id in `minecraft:command_argument_type`.
        id: RegistryId,
        /// Registry the argument names an element of, e.g. `minecraft:item`.
        registry: Identifier<'a>,
    },
    /// `minecraft:score_holder`.
    ScoreHolder {
        /// Whether the selector may match more than one holder.
        multiple: bool,
    },
    /// `brigadier:string`.
    String(StringArgumentKind),
    /// `minecraft:time`, a duration in ticks.
    Time {
        /// Smallest duration the argument accepts, in ticks.
        min: i32,
    },
}

/// `ArgumentUtils.createNumberFlags`: bit 0 is a lower bound, bit 1 an upper.
const NUMBER_HAS_MIN: u8 = 1;
/// See [`NUMBER_HAS_MIN`].
const NUMBER_HAS_MAX: u8 = 2;

/// The `minecraft:command_argument_type` network id of `name`.
///
/// # Errors
/// Returns [`Error::InvalidIdentifier`] when the registry has no such entry,
/// which means this crate's registry table and its argument-type list have
/// drifted apart.
fn argument_type_id(name: &str) -> Result<i32> {
    let id = registry::COMMAND_ARGUMENT_TYPE
        .id_of(name)
        .ok_or_else(|| Error::InvalidIdentifier(name.to_owned()))?;
    i32::try_from(id).map_err(|_| Error::InvalidIdentifier(name.to_owned()))
}

impl ArgumentType<'_> {
    /// The network id this argument type is written under.
    ///
    /// # Errors
    /// See [`argument_type_id`].
    pub fn to_id(&self) -> Result<i32> {
        match self {
            Self::Empty(id) | Self::Registry { id, .. } => Ok(id.0),
            Self::Double { .. } => argument_type_id("brigadier:double"),
            Self::Entity { .. } => argument_type_id("minecraft:entity"),
            Self::Float { .. } => argument_type_id("brigadier:float"),
            Self::Integer { .. } => argument_type_id("brigadier:integer"),
            Self::Long { .. } => argument_type_id("brigadier:long"),
            Self::ScoreHolder { .. } => argument_type_id("minecraft:score_holder"),
            Self::String(_) => argument_type_id("brigadier:string"),
            Self::Time { .. } => argument_type_id("minecraft:time"),
        }
    }

    /// Write the id and then the properties, as `ArgumentNodeStub.serializeCap`
    /// does.
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.to_id()?);
        match self {
            Self::Empty(_) => {}
            Self::Float { min, max } => {
                writer.u8(number_flags(min.is_some(), max.is_some()));
                if let Some(min) = min {
                    writer.f32(*min);
                }
                if let Some(max) = max {
                    writer.f32(*max);
                }
            }
            Self::Double { min, max } => {
                writer.u8(number_flags(min.is_some(), max.is_some()));
                if let Some(min) = min {
                    writer.f64(*min);
                }
                if let Some(max) = max {
                    writer.f64(*max);
                }
            }
            Self::Integer { min, max } => {
                writer.u8(number_flags(min.is_some(), max.is_some()));
                if let Some(min) = min {
                    writer.i32(*min);
                }
                if let Some(max) = max {
                    writer.i32(*max);
                }
            }
            Self::Long { min, max } => {
                writer.u8(number_flags(min.is_some(), max.is_some()));
                if let Some(min) = min {
                    writer.i64(*min);
                }
                if let Some(max) = max {
                    writer.i64(*max);
                }
            }
            Self::String(kind) => kind.encode(writer)?,
            Self::Entity {
                single,
                players_only,
            } => writer.u8(u8::from(*single) | (u8::from(*players_only) << 1)),
            Self::ScoreHolder { multiple } => writer.u8(u8::from(*multiple)),
            Self::Time { min } => writer.i32(*min),
            Self::Registry { registry, .. } => registry.encode(writer)?,
        }
        Ok(())
    }
}

/// `ArgumentUtils.createNumberFlags`.
const fn number_flags(has_min: bool, has_max: bool) -> u8 {
    (if has_min { NUMBER_HAS_MIN } else { 0 }) | (if has_max { NUMBER_HAS_MAX } else { 0 })
}

impl<'a> ArgumentType<'a> {
    /// Read an id and the properties its type implies.
    ///
    /// An id naming no entry in this version's registry is
    /// [`Error::InvalidEnum`] rather than a silently empty argument: the
    /// properties would be read as the next node's fields.
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let raw = reader.var_int()?;
        let name = usize::try_from(raw)
            .ok()
            .and_then(|id| registry::COMMAND_ARGUMENT_TYPE.get(id))
            .ok_or(Error::InvalidEnum {
                name: "minecraft:command_argument_type",
                value: raw,
            })?;
        let id = RegistryId(raw);
        Ok(match name {
            "brigadier:float" => {
                let flags = reader.u8()?;
                Self::Float {
                    min: read_if(flags & NUMBER_HAS_MIN != 0, reader, Reader::f32)?,
                    max: read_if(flags & NUMBER_HAS_MAX != 0, reader, Reader::f32)?,
                }
            }
            "brigadier:double" => {
                let flags = reader.u8()?;
                Self::Double {
                    min: read_if(flags & NUMBER_HAS_MIN != 0, reader, Reader::f64)?,
                    max: read_if(flags & NUMBER_HAS_MAX != 0, reader, Reader::f64)?,
                }
            }
            "brigadier:integer" => {
                let flags = reader.u8()?;
                Self::Integer {
                    min: read_if(flags & NUMBER_HAS_MIN != 0, reader, Reader::i32)?,
                    max: read_if(flags & NUMBER_HAS_MAX != 0, reader, Reader::i32)?,
                }
            }
            "brigadier:long" => {
                let flags = reader.u8()?;
                Self::Long {
                    min: read_if(flags & NUMBER_HAS_MIN != 0, reader, Reader::i64)?,
                    max: read_if(flags & NUMBER_HAS_MAX != 0, reader, Reader::i64)?,
                }
            }
            "brigadier:string" => Self::String(StringArgumentKind::decode(reader)?),
            "minecraft:entity" => {
                let flags = reader.u8()?;
                Self::Entity {
                    single: flags & 1 != 0,
                    players_only: flags & 2 != 0,
                }
            }
            "minecraft:score_holder" => Self::ScoreHolder {
                multiple: reader.u8()? & 1 != 0,
            },
            "minecraft:time" => Self::Time { min: reader.i32()? },
            "minecraft:resource_or_tag"
            | "minecraft:resource_or_tag_key"
            | "minecraft:resource"
            | "minecraft:resource_key"
            | "minecraft:resource_selector" => Self::Registry {
                id,
                registry: Identifier::decode(reader)?,
            },
            _ => Self::Empty(id),
        })
    }
}

/// Read one value only when `present`, so an optional field is one expression.
fn read_if<'a, T>(
    present: bool,
    reader: &mut Reader<'a>,
    read: fn(&mut Reader<'a>) -> Result<T>,
) -> Result<Option<T>> {
    if present {
        Ok(Some(read(reader)?))
    } else {
        Ok(None)
    }
}

/// What kind of node this is, and what it needs beyond its flags
/// (`ClientboundCommandsPacket.NodeStub`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum CommandNodeStub<'a> {
    /// The tree's root, which has children and nothing else.
    #[default]
    Root,
    /// A fixed word, e.g. the `team` in `/team add`.
    Literal {
        /// The word itself.
        name: &'a str,
    },
    /// A parsed value.
    Argument {
        /// Argument name, which is how a suggestion request refers to it.
        name: &'a str,
        /// How the client parses the value.
        parser: ArgumentType<'a>,
        /// Which suggestion provider fills in completions, e.g.
        /// `minecraft:ask_server`. `None` means the parser's own suggestions.
        suggestions: Option<Identifier<'a>>,
    },
}

impl CommandNodeStub<'_> {
    /// The two low bits of the node's flags byte (`MASK_TYPE`).
    const fn type_bits(&self) -> u8 {
        match self {
            Self::Root => 0,
            Self::Literal { .. } => 1,
            Self::Argument { .. } => 2,
        }
    }
}

/// One node of the command graph (`ClientboundCommandsPacket.Entry`).
///
/// [`children`](Self::children) and [`redirect`](Self::redirect) are indices
/// into the packet's own node list, so a node is only meaningful inside the
/// [`Commands`] that holds it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CommandNode<'a> {
    /// Indices of the nodes that may follow this one.
    pub children: Vec<i32>,
    /// Index of the node this one aliases, if any (`FLAG_REDIRECT`).
    pub redirect: Option<i32>,
    /// What the node matches.
    pub stub: CommandNodeStub<'a>,
    /// Whether a command may end here (`FLAG_EXECUTABLE`).
    pub executable: bool,
    /// Whether the client greys the node out because the player lacks the
    /// permission for it (`FLAG_RESTRICTED`, new in 1.21.6).
    pub restricted: bool,
}

/// A command may end at this node.
const FLAG_EXECUTABLE: u8 = 4;
/// The node aliases another, whose index follows.
const FLAG_REDIRECT: u8 = 8;
/// An argument node names a suggestion provider.
const FLAG_CUSTOM_SUGGESTIONS: u8 = 16;
/// The player may not run this node.
const FLAG_RESTRICTED: u8 = 32;

impl Encode for CommandNode<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let mut flags = self.stub.type_bits();
        if self.executable {
            flags |= FLAG_EXECUTABLE;
        }
        if self.redirect.is_some() {
            flags |= FLAG_REDIRECT;
        }
        if self.restricted {
            flags |= FLAG_RESTRICTED;
        }
        if let CommandNodeStub::Argument {
            suggestions: Some(_),
            ..
        } = self.stub
        {
            flags |= FLAG_CUSTOM_SUGGESTIONS;
        }
        writer.u8(flags);

        write_count(writer, self.children.len(), None)?;
        for child in &self.children {
            writer.var_int(*child);
        }
        if let Some(redirect) = self.redirect {
            writer.var_int(redirect);
        }
        match &self.stub {
            CommandNodeStub::Root => {}
            CommandNodeStub::Literal { name } => writer.string(name)?,
            CommandNodeStub::Argument {
                name,
                parser,
                suggestions,
            } => {
                writer.string(name)?;
                parser.encode(writer)?;
                // Not an `Option`: the flag bit already said whether this is
                // here, so there is no boolean in front of it.
                if let Some(suggestions) = suggestions {
                    suggestions.encode(writer)?;
                }
            }
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for CommandNode<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let flags = reader.u8()?;
        let count = read_count(reader, None)?;
        let mut children = Vec::with_capacity(count.min(reader.remaining_len()));
        for _ in 0..count {
            children.push(reader.var_int()?);
        }
        let redirect = read_if(flags & FLAG_REDIRECT != 0, reader, Reader::var_int)?;
        let stub = match flags & 3 {
            1 => CommandNodeStub::Literal {
                name: reader.string()?,
            },
            2 => CommandNodeStub::Argument {
                name: reader.string()?,
                parser: ArgumentType::decode(reader)?,
                suggestions: if flags & FLAG_CUSTOM_SUGGESTIONS == 0 {
                    None
                } else {
                    Some(Identifier::decode(reader)?)
                },
            },
            // `ClientboundCommandsPacket.read` returns a null stub for type 0
            // and for the undefined type 3 alike, and a null stub is what a
            // root node is.
            _ => CommandNodeStub::Root,
        };
        Ok(Self {
            children,
            redirect,
            stub,
            executable: flags & FLAG_EXECUTABLE != 0,
            restricted: flags & FLAG_RESTRICTED != 0,
        })
    }
}

/// `minecraft:commands`, clientbound (`ClientboundCommandsPacket`).
///
/// The whole command tree, flattened: nodes in one list, edges as indices into
/// it. The client rebuilds the graph from [`root_index`](Self::root_index) and
/// refuses a tree whose indices do not resolve, so a node list that names an
/// index it does not have disconnects the player.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Commands<'a> {
    /// Every node, in no particular order.
    pub nodes: Vec<CommandNode<'a>>,
    /// Index of the root within [`nodes`](Self::nodes).
    pub root_index: i32,
}

impl Encode for Commands<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.nodes.encode(writer)?;
        writer.var_int(self.root_index);
        Ok(())
    }
}

impl<'a> Decode<'a> for Commands<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            nodes: Vec::decode(reader)?,
            root_index: reader.var_int()?,
        })
    }
}

// --- boss bar -------------------------------------------------------------

/// A boss bar's colour (`BossEvent.BossBarColor`), as a `VarInt` ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum BossBarColor {
    /// Pink.
    #[default]
    Pink = 0,
    /// Blue.
    Blue = 1,
    /// Red.
    Red = 2,
    /// Green.
    Green = 3,
    /// Yellow.
    Yellow = 4,
    /// Purple.
    Purple = 5,
    /// White.
    White = 6,
}

/// How a boss bar is divided (`BossEvent.BossBarOverlay`), as a `VarInt`
/// ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum BossBarOverlay {
    /// One continuous bar.
    #[default]
    Progress = 0,
    /// Six notches.
    Notched6 = 1,
    /// Ten notches.
    Notched10 = 2,
    /// Twelve notches.
    Notched12 = 3,
    /// Twenty notches.
    Notched20 = 4,
}

/// A boss bar's side effects on the world (`FLAG_DARKEN`, `FLAG_MUSIC`,
/// `FLAG_FOG`), as a bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BossBarProperties(u8);

impl BossBarProperties {
    /// Every defined bit.
    pub const ALL: Self = Self(0b111);
    /// Fog closes in (`FLAG_FOG`).
    pub const CREATE_WORLD_FOG: Self = Self(4);
    /// The sky darkens (`FLAG_DARKEN`).
    pub const DARKEN_SCREEN: Self = Self(1);
    /// No side effects.
    pub const NONE: Self = Self(0);
    /// Boss music plays (`FLAG_MUSIC`).
    pub const PLAY_MUSIC: Self = Self(2);

    /// The mask with `other`'s bits also set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_bits(self) -> u8 {
        self.0
    }

    /// Read a wire byte, dropping the undefined bits the way the client's own
    /// `(flags & FLAG) > 0` tests do.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits & Self::ALL.0)
    }
}

/// What a [`BossEvent`] does to the bar it names
/// (`ClientboundBossEventPacket.Operation`).
///
/// The leading `VarInt` is `OperationType`'s ordinal and it selects the whole
/// tail, which is why the generator could not follow this packet.
#[derive(Debug, Clone, PartialEq)]
pub enum BossEventOperation<'a> {
    /// Create the bar.
    Add {
        /// Title shown above the bar.
        name: Component<'a>,
        /// Fullness in `0.0..=1.0`.
        progress: f32,
        /// Bar colour.
        color: BossBarColor,
        /// How the bar is divided.
        overlay: BossBarOverlay,
        /// Sky, music and fog effects.
        properties: BossBarProperties,
    },
    /// Remove the bar.
    Remove,
    /// Move the bar without touching anything else.
    UpdateProgress(f32),
    /// Retitle the bar.
    UpdateName(Component<'a>),
    /// Recolour and re-divide the bar.
    UpdateStyle {
        /// Bar colour.
        color: BossBarColor,
        /// How the bar is divided.
        overlay: BossBarOverlay,
    },
    /// Change the sky, music and fog effects.
    UpdateProperties(BossBarProperties),
}

impl BossEventOperation<'_> {
    /// `OperationType`'s ordinal for this operation.
    const fn type_id(&self) -> i32 {
        match self {
            Self::Add { .. } => 0,
            Self::Remove => 1,
            Self::UpdateProgress(_) => 2,
            Self::UpdateName(_) => 3,
            Self::UpdateStyle { .. } => 4,
            Self::UpdateProperties(_) => 5,
        }
    }
}

/// `minecraft:boss_event`, clientbound (`ClientboundBossEventPacket`).
///
/// The generated `clientbound::BossEvent` is an empty struct: the extractor
/// read `Packet.codec(write, new)` and found no composed fields to follow, so
/// this is the one to use.
#[derive(Debug, Clone, PartialEq)]
pub struct BossEvent<'a> {
    /// Which bar, chosen by the server and reused across updates.
    pub id: Uuid,
    /// What to do to it.
    pub operation: BossEventOperation<'a>,
}

impl Encode for BossEvent<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.id.encode(writer)?;
        writer.var_int(self.operation.type_id());
        match &self.operation {
            BossEventOperation::Add {
                name,
                progress,
                color,
                overlay,
                properties,
            } => {
                name.encode(writer)?;
                writer.f32(*progress);
                color.encode(writer)?;
                overlay.encode(writer)?;
                writer.u8(properties.to_bits());
            }
            BossEventOperation::Remove => {}
            BossEventOperation::UpdateProgress(progress) => writer.f32(*progress),
            BossEventOperation::UpdateName(name) => name.encode(writer)?,
            BossEventOperation::UpdateStyle { color, overlay } => {
                color.encode(writer)?;
                overlay.encode(writer)?;
            }
            BossEventOperation::UpdateProperties(properties) => writer.u8(properties.to_bits()),
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for BossEvent<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let id = Uuid::decode(reader)?;
        let raw = reader.var_int()?;
        let operation = match raw {
            0 => BossEventOperation::Add {
                name: Component::decode(reader)?,
                progress: reader.f32()?,
                color: BossBarColor::decode(reader)?,
                overlay: BossBarOverlay::decode(reader)?,
                properties: BossBarProperties::from_bits(reader.u8()?),
            },
            1 => BossEventOperation::Remove,
            2 => BossEventOperation::UpdateProgress(reader.f32()?),
            3 => BossEventOperation::UpdateName(Component::decode(reader)?),
            4 => BossEventOperation::UpdateStyle {
                color: BossBarColor::decode(reader)?,
                overlay: BossBarOverlay::decode(reader)?,
            },
            5 => BossEventOperation::UpdateProperties(BossBarProperties::from_bits(reader.u8()?)),
            value => {
                return Err(Error::InvalidEnum {
                    name: "ClientboundBossEventPacket.OperationType",
                    value,
                });
            }
        };
        Ok(Self { id, operation })
    }
}

// --- scoreboard -----------------------------------------------------------

/// How a score is rendered where it appears (`NumberFormat`).
///
/// Dispatched on a `VarInt` id into `minecraft:number_format_type`.
#[derive(Debug, Clone, PartialEq)]
pub enum NumberFormat<'a> {
    /// `minecraft:blank`: the number is not drawn at all.
    Blank,
    /// `minecraft:styled`: the number is drawn with this style, as network NBT.
    /// See this module's note on styles.
    Styled(nbt::Tag<'a>),
    /// `minecraft:fixed`: this component is drawn instead of the number.
    ///
    /// Boxed because a [`Component`] is two orders of magnitude larger than
    /// the other two variants, and this one is rare.
    Fixed(Box<Component<'a>>),
}

impl NumberFormat<'_> {
    /// The `minecraft:number_format_type` entry this variant is.
    const fn type_name(&self) -> &'static str {
        match self {
            Self::Blank => "minecraft:blank",
            Self::Styled(_) => "minecraft:styled",
            Self::Fixed(_) => "minecraft:fixed",
        }
    }
}

impl Encode for NumberFormat<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        let name = self.type_name();
        let id = registry::NUMBER_FORMAT_TYPE
            .id_of(name)
            .ok_or_else(|| Error::InvalidIdentifier(name.to_owned()))?;
        writer.var_int(i32::try_from(id).map_err(|_| Error::InvalidIdentifier(name.to_owned()))?);
        match self {
            Self::Blank => Ok(()),
            Self::Styled(style) => style.encode(writer),
            Self::Fixed(value) => value.encode(writer),
        }
    }
}

impl<'a> Decode<'a> for NumberFormat<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let raw = reader.var_int()?;
        let name = usize::try_from(raw)
            .ok()
            .and_then(|id| registry::NUMBER_FORMAT_TYPE.get(id))
            .ok_or(Error::InvalidEnum {
                name: "minecraft:number_format_type",
                value: raw,
            })?;
        match name {
            "minecraft:blank" => Ok(Self::Blank),
            "minecraft:styled" => Ok(Self::Styled(nbt::Tag::decode(reader)?)),
            "minecraft:fixed" => Ok(Self::Fixed(Box::new(Component::decode(reader)?))),
            _ => Err(Error::InvalidEnum {
                name: "minecraft:number_format_type",
                value: raw,
            }),
        }
    }
}

/// How an objective's score is drawn beside a name
/// (`ObjectiveCriteria.RenderType`), as a `VarInt` ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum ObjectiveRenderType {
    /// The number itself.
    #[default]
    Integer = 0,
    /// A row of hearts.
    Hearts = 1,
}

/// The fields `SetObjective` writes only for `METHOD_ADD` and `METHOD_CHANGE`.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectiveDisplay<'a> {
    /// Title shown at the top of the sidebar.
    pub display_name: Component<'a>,
    /// How the score is drawn.
    pub render_type: ObjectiveRenderType,
    /// Score formatting, or `None` for the client's default.
    pub number_format: Option<NumberFormat<'a>>,
}

impl Encode for ObjectiveDisplay<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.display_name.encode(writer)?;
        self.render_type.encode(writer)?;
        self.number_format.encode(writer)
    }
}

impl<'a> Decode<'a> for ObjectiveDisplay<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            display_name: Component::decode(reader)?,
            render_type: ObjectiveRenderType::decode(reader)?,
            number_format: Option::decode(reader)?,
        })
    }
}

/// `minecraft:set_objective`, clientbound (`ClientboundSetObjectivePacket`).
///
/// The method byte decides whether the display fields follow it:
/// [`METHOD_REMOVE`](Self::METHOD_REMOVE) ends the packet, and the other two
/// carry an [`ObjectiveDisplay`]. The two are one field here so that the
/// combination the wire cannot express cannot be built either.
#[derive(Debug, Clone, PartialEq)]
pub struct SetObjective<'a> {
    /// Objective name, which is the key a score refers to.
    pub objective_name: &'a str,
    /// The display fields, present for add and change and absent for remove.
    pub display: Option<ObjectiveDisplay<'a>>,
    /// Whether this creates the objective or changes an existing one.
    ///
    /// Only meaningful when [`display`](Self::display) is `Some`: with no
    /// display the method is [`METHOD_REMOVE`](Self::METHOD_REMOVE) regardless.
    pub change: bool,
}

impl SetObjective<'_> {
    /// Create the objective (`METHOD_ADD`).
    pub const METHOD_ADD: i8 = 0;
    /// Change an objective the client already has (`METHOD_CHANGE`).
    pub const METHOD_CHANGE: i8 = 2;
    /// Delete the objective (`METHOD_REMOVE`).
    pub const METHOD_REMOVE: i8 = 1;
}

impl Encode for SetObjective<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.objective_name)?;
        match &self.display {
            None => writer.i8(Self::METHOD_REMOVE),
            Some(display) => {
                writer.i8(if self.change {
                    Self::METHOD_CHANGE
                } else {
                    Self::METHOD_ADD
                });
                display.encode(writer)?;
            }
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for SetObjective<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let objective_name = reader.string()?;
        let method = reader.i8()?;
        // `ClientboundSetObjectivePacket`'s own read tests for add and change
        // and treats every other byte as remove, so an unknown method is not
        // an error here either.
        let has_display = method == Self::METHOD_ADD || method == Self::METHOD_CHANGE;
        Ok(Self {
            objective_name,
            display: read_if(has_display, reader, ObjectiveDisplay::decode)?,
            change: method == Self::METHOD_CHANGE,
        })
    }
}

/// `minecraft:set_score`, clientbound (`ClientboundSetScorePacket`).
///
/// Mechanical apart from [`number_format`](Self::number_format), whose codec
/// dispatches on a registry id.
#[derive(Debug, Clone, PartialEq)]
pub struct SetScore<'a> {
    /// Score holder, a player name or a plain string for a fake one.
    pub owner: &'a str,
    /// Which objective the score belongs to.
    pub objective_name: &'a str,
    /// The score.
    pub score: i32,
    /// Name to draw instead of `owner`, or `None` for `owner` itself.
    pub display: Option<Component<'a>>,
    /// How to draw the number, or `None` for the objective's own format.
    pub number_format: Option<NumberFormat<'a>>,
}

impl Encode for SetScore<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.owner)?;
        writer.string(self.objective_name)?;
        writer.var_int(self.score);
        self.display.encode(writer)?;
        self.number_format.encode(writer)
    }
}

impl<'a> Decode<'a> for SetScore<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            owner: reader.string()?,
            objective_name: reader.string()?,
            score: reader.var_int()?,
            display: Option::decode(reader)?,
            number_format: Option::decode(reader)?,
        })
    }
}

/// When a team's name tags are visible (`Team.Visibility`), as a `VarInt` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum TeamVisibility {
    /// Always.
    #[default]
    Always = 0,
    /// Never.
    Never = 1,
    /// Hidden from players on other teams.
    HideForOtherTeams = 2,
    /// Hidden from players on this team.
    HideForOwnTeam = 3,
}

/// Who a team's members push (`Team.CollisionRule`), as a `VarInt` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum TeamCollisionRule {
    /// Everyone.
    #[default]
    Always = 0,
    /// Nobody.
    Never = 1,
    /// Only players on other teams.
    PushOtherTeams = 2,
    /// Only players on this team.
    PushOwnTeam = 3,
}

/// A team's colour, which is also the sidebar slot it owns (`TeamColor`), as a
/// `VarInt` id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
pub enum TeamColor {
    /// Black.
    #[default]
    Black = 0,
    /// Dark blue.
    DarkBlue = 1,
    /// Dark green.
    DarkGreen = 2,
    /// Dark aqua.
    DarkAqua = 3,
    /// Dark red.
    DarkRed = 4,
    /// Dark purple.
    DarkPurple = 5,
    /// Gold.
    Gold = 6,
    /// Gray.
    Gray = 7,
    /// Dark gray.
    DarkGray = 8,
    /// Blue.
    Blue = 9,
    /// Green.
    Green = 10,
    /// Aqua.
    Aqua = 11,
    /// Red.
    Red = 12,
    /// Light purple.
    LightPurple = 13,
    /// Yellow.
    Yellow = 14,
    /// White.
    White = 15,
}

/// A team's two boolean settings (`PlayerTeam.packOptions`), as a bitmask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TeamOptions(i8);

impl TeamOptions {
    /// Every defined bit.
    pub const ALL: Self = Self(0b11);
    /// Members may hurt each other.
    pub const ALLOW_FRIENDLY_FIRE: Self = Self(1);
    /// Neither setting.
    pub const NONE: Self = Self(0);
    /// Members see each other while invisible.
    pub const SEE_FRIENDLY_INVISIBLES: Self = Self(2);

    /// The mask with `other`'s bits also set.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Whether every bit of `other` is set.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The wire byte.
    #[must_use]
    pub const fn to_bits(self) -> i8 {
        self.0
    }

    /// Read a wire byte, dropping the bits `unpackOptions` does not test.
    #[must_use]
    pub const fn from_bits(bits: i8) -> Self {
        Self(bits & Self::ALL.0)
    }
}

/// Everything about a team but its members
/// (`ClientboundSetPlayerTeamPacket.Parameters`).
#[derive(Debug, Clone, PartialEq)]
pub struct TeamParameters<'a> {
    /// Team name as shown to players.
    pub display_name: Component<'a>,
    /// Prepended to every member's name.
    pub player_prefix: Component<'a>,
    /// Appended to every member's name.
    pub player_suffix: Component<'a>,
    /// When name tags are drawn.
    pub name_tag_visibility: TeamVisibility,
    /// Who members push.
    pub collision_rule: TeamCollisionRule,
    /// Member name colour, or `None` for no colour.
    pub color: Option<TeamColor>,
    /// Friendly fire and invisibility.
    pub options: TeamOptions,
}

impl Encode for TeamParameters<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.display_name.encode(writer)?;
        self.player_prefix.encode(writer)?;
        self.player_suffix.encode(writer)?;
        self.name_tag_visibility.encode(writer)?;
        self.collision_rule.encode(writer)?;
        self.color.encode(writer)?;
        writer.i8(self.options.to_bits());
        Ok(())
    }
}

impl<'a> Decode<'a> for TeamParameters<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            display_name: Component::decode(reader)?,
            player_prefix: Component::decode(reader)?,
            player_suffix: Component::decode(reader)?,
            name_tag_visibility: TeamVisibility::decode(reader)?,
            collision_rule: TeamCollisionRule::decode(reader)?,
            color: Option::decode(reader)?,
            options: TeamOptions::from_bits(reader.i8()?),
        })
    }
}

/// `minecraft:set_player_team`, clientbound
/// (`ClientboundSetPlayerTeamPacket`).
///
/// The method byte decides which of the two tails follow, and the two
/// predicates do not agree: `shouldHaveParameters` is add and change,
/// `shouldHavePlayerList` is add, join and leave. Only add has both.
#[derive(Debug, Clone, PartialEq)]
pub struct SetPlayerTeam<'a> {
    /// Team name, which is the key everything else refers to.
    pub name: &'a str,
    /// Which of the five operations this is.
    pub method: i8,
    /// Team settings, for add and change.
    pub parameters: Option<TeamParameters<'a>>,
    /// Members, for add, join and leave.
    pub players: Vec<&'a str>,
}

impl SetPlayerTeam<'_> {
    /// Create the team, with parameters and members (`METHOD_ADD`).
    pub const METHOD_ADD: i8 = 0;
    /// Change the team's parameters (`METHOD_CHANGE`).
    pub const METHOD_CHANGE: i8 = 2;
    /// Add the named members (`METHOD_JOIN`).
    pub const METHOD_JOIN: i8 = 3;
    /// Remove the named members (`METHOD_LEAVE`).
    pub const METHOD_LEAVE: i8 = 4;
    /// Delete the team (`METHOD_REMOVE`).
    pub const METHOD_REMOVE: i8 = 1;

    /// `ClientboundSetPlayerTeamPacket.shouldHaveParameters`.
    const fn has_parameters(method: i8) -> bool {
        method == Self::METHOD_ADD || method == Self::METHOD_CHANGE
    }

    /// `ClientboundSetPlayerTeamPacket.shouldHavePlayerList`.
    const fn has_player_list(method: i8) -> bool {
        method == Self::METHOD_ADD || method == Self::METHOD_JOIN || method == Self::METHOD_LEAVE
    }
}

impl Encode for SetPlayerTeam<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.name)?;
        writer.i8(self.method);
        if Self::has_parameters(self.method) {
            // Java throws here rather than writing a shorter packet, because a
            // reader sizes the rest of the body off the method byte alone.
            let parameters = self
                .parameters
                .as_ref()
                .ok_or(Error::MissingField("set_player_team parameters"))?;
            parameters.encode(writer)?;
        }
        if Self::has_player_list(self.method) {
            write_count(writer, self.players.len(), None)?;
            for player in &self.players {
                writer.string(player)?;
            }
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for SetPlayerTeam<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let name = reader.string()?;
        let method = reader.i8()?;
        let parameters = read_if(Self::has_parameters(method), reader, TeamParameters::decode)?;
        let mut players = Vec::new();
        if Self::has_player_list(method) {
            let count = read_count(reader, None)?;
            players.reserve(count.min(reader.remaining_len()));
            for _ in 0..count {
                players.push(reader.string()?);
            }
        }
        Ok(Self {
            name,
            method,
            parameters,
            players,
        })
    }
}
