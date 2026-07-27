//! Configuration state: everything between login and play.
//!
//! Since 1.20.2 a client that has acknowledged login lands here rather than in
//! play. This is where the server hands over the dynamic registries, the tags,
//! the enabled feature flags and its brand, and where the two sides negotiate
//! which data packs the client already has. A client cannot render anything
//! until it has been through it, so it is not optional.
//!
//! Several packets in this state are `net.minecraft.network.protocol.common`
//! classes shared with play, which is why their ids differ between the two
//! states while their bodies do not. Each type below names the class it came
//! from; the ids live in [`crate::generated::packet_id`].
//!
//! # Known packs and `registry_data`
//!
//! [`SelectKnownPacks`] runs before [`RegistryData`] and changes what
//! `registry_data` contains. `RegistrySynchronization.packRegistry` writes one
//! entry per registry element either way, but an element whose defining pack
//! the client reported as known is written with *no* payload -- the client
//! fills it in from its own copy. So [`RegistryEntry::data`] being `None` is
//! not an empty entry, it is a reference to the client's local one.

use crate::{Decode, Encode, Error, Reader, Result, Writer, nbt::Tag, text::Component};

// --- shared list handling -------------------------------------------------

/// Read a `VarInt` element count, refusing one the remaining input cannot
/// supply.
///
/// The count is attacker-controlled and feeds a reservation, so it is checked
/// against the bytes actually present before anything is allocated.
/// `min_element_size` is the smallest encoding one element can have.
///
/// # Errors
/// Returns [`Error::NegativeLength`] on a negative count and
/// [`Error::UnexpectedEof`] when the input is too short to hold that many
/// elements.
pub(super) fn read_count(reader: &mut Reader<'_>, min_element_size: usize) -> Result<usize> {
    let count = reader.var_int()?;
    let count = usize::try_from(count).map_err(|_| Error::NegativeLength(count))?;
    let needed = count.saturating_mul(min_element_size);
    if needed > reader.remaining_len() {
        return Err(Error::UnexpectedEof {
            needed,
            available: reader.remaining_len(),
        });
    }
    Ok(count)
}

/// Write a `VarInt` element count.
///
/// # Errors
/// Returns [`Error::NegativeLength`] for a collection larger than a `VarInt`
/// can describe, which no real packet reaches.
pub(super) fn write_count(writer: &mut Writer, count: usize) -> Result<()> {
    writer.var_int(i32::try_from(count).map_err(|_| Error::NegativeLength(-1))?);
    Ok(())
}

// --- client information ---------------------------------------------------

/// How much chat a client wants to see (`ChatVisiblity`).
///
/// The misspelling is Mojang's; the name is kept as it appears in the jar so
/// it greps. `ClientInformation` reads it with `FriendlyByteBuf.readEnum`,
/// which is a `VarInt` ordinal and throws on anything out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ChatVisiblity {
    /// Show everything.
    Full = 0,
    /// Show system messages only.
    System = 1,
    /// Show nothing.
    Hidden = 2,
}

impl ChatVisiblity {
    /// Resolve a wire ordinal.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] outside 0..=2, matching `readEnum`
    /// indexing past the end of `values()`.
    pub const fn from_raw(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Full),
            1 => Ok(Self::System),
            2 => Ok(Self::Hidden),
            _ => Err(Error::InvalidEnum {
                name: "ChatVisiblity",
                value,
            }),
        }
    }

    /// The wire ordinal.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self as i32
    }
}

/// Which hand a player holds items in (`HumanoidArm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum HumanoidArm {
    /// Left-handed.
    Left = 0,
    /// Right-handed, the vanilla default.
    Right = 1,
}

impl HumanoidArm {
    /// Resolve a wire ordinal.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] outside 0..=1.
    pub const fn from_raw(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::Left),
            1 => Ok(Self::Right),
            _ => Err(Error::InvalidEnum {
                name: "HumanoidArm",
                value,
            }),
        }
    }

    /// The wire ordinal.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self as i32
    }
}

/// How many particles a client wants (`ParticleStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ParticleStatus {
    /// All particles.
    All = 0,
    /// Fewer particles.
    Decreased = 1,
    /// As few as the game will draw.
    Minimal = 2,
}

impl ParticleStatus {
    /// Resolve a wire ordinal.
    ///
    /// # Errors
    /// Returns [`Error::InvalidEnum`] outside 0..=2.
    pub const fn from_raw(value: i32) -> Result<Self> {
        match value {
            0 => Ok(Self::All),
            1 => Ok(Self::Decreased),
            2 => Ok(Self::Minimal),
            _ => Err(Error::InvalidEnum {
                name: "ParticleStatus",
                value,
            }),
        }
    }

    /// The wire ordinal.
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self as i32
    }
}

/// `minecraft:client_information`, serverbound
/// (`ServerboundClientInformationPacket` wrapping `ClientInformation`).
///
/// Sent in configuration and again in play whenever the player changes a
/// setting, which is why the body lives in a shared class.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientInformation<'a> {
    /// Locale, e.g. `en_us`.
    pub language: &'a str,
    /// Render distance in chunks. Signed, because the field is a raw byte and
    /// `ClientInformation` reads it with `readByte`.
    pub view_distance: i8,
    /// How much chat to deliver.
    pub chat_visibility: ChatVisiblity,
    /// Whether to colour chat.
    pub chat_colors: bool,
    /// Bitmask of enabled skin overlays, read with `readUnsignedByte`.
    pub model_customisation: u8,
    /// Which hand the player considers dominant.
    pub main_hand: HumanoidArm,
    /// Whether the client opted into server-side text filtering.
    pub text_filtering_enabled: bool,
    /// Whether the player may appear in the server list.
    pub allows_listing: bool,
    /// How many particles the client wants.
    pub particle_status: ParticleStatus,
}

/// `ClientInformation.MAX_LANGUAGE_LENGTH`.
pub const MAX_LANGUAGE_LENGTH: usize = 16;

impl Encode for ClientInformation<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        // `write` calls the unlimited `writeUtf` and the limit is applied on
        // read; it is applied on both sides here so a value the far side would
        // reject cannot leave.
        writer.string_with_limit(self.language, MAX_LANGUAGE_LENGTH)?;
        writer.i8(self.view_distance);
        writer.var_int(self.chat_visibility.to_raw());
        writer.bool(self.chat_colors);
        writer.u8(self.model_customisation);
        writer.var_int(self.main_hand.to_raw());
        writer.bool(self.text_filtering_enabled);
        writer.bool(self.allows_listing);
        writer.var_int(self.particle_status.to_raw());
        Ok(())
    }
}

impl<'a> Decode<'a> for ClientInformation<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            language: reader.string_with_limit(MAX_LANGUAGE_LENGTH)?,
            view_distance: reader.i8()?,
            chat_visibility: ChatVisiblity::from_raw(reader.var_int()?)?,
            chat_colors: reader.bool()?,
            model_customisation: reader.u8()?,
            main_hand: HumanoidArm::from_raw(reader.var_int()?)?,
            text_filtering_enabled: reader.bool()?,
            allows_listing: reader.bool()?,
            particle_status: ParticleStatus::from_raw(reader.var_int()?)?,
        })
    }
}

// --- custom payload -------------------------------------------------------

/// `minecraft:custom_payload`, both directions (`Clientbound`- and
/// `ServerboundCustomPayloadPacket`).
///
/// The body is a channel identifier followed by the rest of the frame with no
/// length of its own: `DiscardedPayload` reads `readableBytes()`. So a payload
/// can only be delimited by the frame around it, and [`data`](Self::data) is
/// whatever remains.
///
/// The one channel vanilla parses in both directions is `minecraft:brand`,
/// whose payload is a single string. [`brand`](Self::brand) and
/// [`as_brand`](Self::as_brand) build and read that form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomPayload<'a> {
    /// Channel identifier, e.g. `minecraft:brand`.
    pub channel: &'a str,
    /// Channel-defined body, running to the end of the packet.
    pub data: &'a [u8],
}

/// `ClientboundCustomPayloadPacket.MAX_PAYLOAD_SIZE`.
///
/// The two directions cap the payload differently and the cap is not part of
/// the body, so neither is enforced by [`CustomPayload`]'s own codec; a caller
/// that knows the direction can check [`CustomPayload::data`] against the
/// matching constant.
pub const MAX_CLIENTBOUND_PAYLOAD_SIZE: usize = 0x0010_0000;

/// `ServerboundCustomPayloadPacket.MAX_PAYLOAD_SIZE` (`Short.MAX_VALUE`).
pub const MAX_SERVERBOUND_PAYLOAD_SIZE: usize = 32767;

/// The channel `BrandPayload` registers (`CustomPacketPayload.createType`).
pub const BRAND_CHANNEL: &str = "minecraft:brand";

impl<'a> CustomPayload<'a> {
    /// The body of a `minecraft:brand` payload, which is one string.
    ///
    /// Returns the bytes rather than a whole [`CustomPayload`] because the
    /// body has to outlive the borrow, and `CustomPayload` borrows rather than
    /// owns.
    ///
    /// # Errors
    /// Returns [`Error::StringTooLong`] past `Short.MAX_VALUE` characters.
    pub fn encode_brand(brand: &str) -> Result<Vec<u8>> {
        let mut writer = Writer::new();
        writer.string(brand)?;
        Ok(writer.into_vec())
    }

    /// The brand this payload names, if it is a `minecraft:brand` payload.
    ///
    /// # Errors
    /// Returns an error when the channel matches but the body is not a single
    /// well-formed string.
    pub fn as_brand(&self) -> Result<Option<&'a str>> {
        if self.channel != BRAND_CHANNEL {
            return Ok(None);
        }
        let mut reader = Reader::new(self.data);
        let brand = reader.string()?;
        reader.finish()?;
        Ok(Some(brand))
    }
}

impl Encode for CustomPayload<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.channel)?;
        writer.raw(self.data);
        Ok(())
    }
}

impl<'a> Decode<'a> for CustomPayload<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let channel = reader.string()?;
        let length = reader.remaining_len();
        Ok(Self {
            channel,
            data: reader.take(length)?,
        })
    }
}

// --- known packs ----------------------------------------------------------

/// One data pack both sides can name (`KnownPack`).
///
/// A client reports the packs it shipped with so the server can leave their
/// registry contents out of [`RegistryData`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownPack<'a> {
    /// Pack namespace; `minecraft` for the built-in packs.
    pub namespace: &'a str,
    /// Pack id within the namespace.
    pub id: &'a str,
    /// Pack version. For vanilla packs this is the game version string.
    pub version: &'a str,
}

/// `KnownPack.VANILLA_NAMESPACE`.
pub const VANILLA_PACK_NAMESPACE: &str = "minecraft";

impl Encode for KnownPack<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.namespace)?;
        writer.string(self.id)?;
        writer.string(self.version)?;
        Ok(())
    }
}

impl<'a> Decode<'a> for KnownPack<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            namespace: reader.string()?,
            id: reader.string()?,
            version: reader.string()?,
        })
    }
}

/// `minecraft:select_known_packs`, both directions (`Clientbound`- and
/// `ServerboundSelectKnownPacks`).
///
/// The server offers the packs it would send; the client answers with the
/// subset it already has. Both bodies are the same list.
///
/// The serverbound codec is `ByteBufCodecs.list(64)` and the clientbound one
/// is unbounded. That cap is not enforced here -- reporting it needs an error
/// variant this module does not own -- so the guard on decode is only that the
/// frame is long enough to hold the count it declares. A caller reading a
/// serverbound packet can check the length against
/// [`MAX_SERVERBOUND_KNOWN_PACKS`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SelectKnownPacks<'a> {
    /// The packs being offered or acknowledged.
    pub known_packs: Vec<KnownPack<'a>>,
}

/// The list cap in `ServerboundSelectKnownPacks.STREAM_CODEC`.
pub const MAX_SERVERBOUND_KNOWN_PACKS: usize = 64;

/// Three strings, each at minimum a one-byte length prefix.
const MIN_KNOWN_PACK_SIZE: usize = 3;

impl Encode for SelectKnownPacks<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        write_count(writer, self.known_packs.len())?;
        for pack in &self.known_packs {
            pack.encode(writer)?;
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for SelectKnownPacks<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let count = read_count(reader, MIN_KNOWN_PACK_SIZE)?;
        let mut known_packs = Vec::with_capacity(count);
        for _ in 0..count {
            known_packs.push(KnownPack::decode(reader)?);
        }
        Ok(Self { known_packs })
    }
}

// --- registry data --------------------------------------------------------

/// One element of a synchronised registry
/// (`RegistrySynchronization.PackedRegistryEntry`).
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryEntry<'a> {
    /// Element name, e.g. `minecraft:overworld`.
    pub id: &'a str,
    /// The element's contents, or `None` when the client already has them from
    /// a pack it reported as known.
    pub data: Option<Tag<'a>>,
}

impl Encode for RegistryEntry<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.id)?;
        // `ByteBufCodecs.TAG.apply(ByteBufCodecs::optional)`: a boolean and
        // then the tag, not the bare `TAG_End` that `writeNbt` uses for null.
        match &self.data {
            Some(tag) => {
                writer.bool(true);
                tag.encode(writer)?;
            }
            None => writer.bool(false),
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for RegistryEntry<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let id = reader.string()?;
        let data = if reader.bool()? {
            Some(Tag::decode(reader)?)
        } else {
            None
        };
        Ok(Self { id, data })
    }
}

/// `minecraft:registry_data`, clientbound (`ClientboundRegistryDataPacket`).
///
/// One packet per registry, sent for each of the registries in
/// `RegistryDataLoader.SYNCHRONIZED_REGISTRIES`. `minecraft:dimension_type`,
/// `minecraft:worldgen/biome` and `minecraft:chat_type` are the ones a client
/// cannot start without, because the join packet and every chat message index
/// into them by network id.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryData<'a> {
    /// Registry name, e.g. `minecraft:dimension_type`.
    pub registry: &'a str,
    /// Elements in network-id order; the index is the id.
    pub entries: Vec<RegistryEntry<'a>>,
}

/// An identifier's length prefix plus the optional's boolean.
const MIN_REGISTRY_ENTRY_SIZE: usize = 2;

impl Encode for RegistryData<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.registry)?;
        write_count(writer, self.entries.len())?;
        for entry in &self.entries {
            entry.encode(writer)?;
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for RegistryData<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let registry = reader.string()?;
        let count = read_count(reader, MIN_REGISTRY_ENTRY_SIZE)?;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            entries.push(RegistryEntry::decode(reader)?);
        }
        Ok(Self { registry, entries })
    }
}

// --- feature flags --------------------------------------------------------

/// `minecraft:update_enabled_features`, clientbound
/// (`ClientboundUpdateEnabledFeaturesPacket`).
///
/// Names the feature flags the world was loaded with. A client that does not
/// recognise one refuses to join, which is what keeps an experimental world
/// from opening on a release client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateEnabledFeatures<'a> {
    /// Enabled feature flag names, e.g. `minecraft:vanilla`.
    ///
    /// A `Set` on the server, so the order carries no meaning; it is kept as
    /// read so the packet re-encodes byte for byte.
    pub features: Vec<&'a str>,
}

impl Encode for UpdateEnabledFeatures<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        write_count(writer, self.features.len())?;
        for feature in &self.features {
            writer.string(feature)?;
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for UpdateEnabledFeatures<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let count = read_count(reader, 1)?;
        let mut features = Vec::with_capacity(count);
        for _ in 0..count {
            features.push(reader.string()?);
        }
        Ok(Self { features })
    }
}

// --- tags -----------------------------------------------------------------

/// One tag and the registry ids it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry<'a> {
    /// Tag name without the `#`, e.g. `minecraft:wool`.
    pub name: &'a str,
    /// Network ids of the elements in the tag, as `VarInt`s.
    pub entries: Vec<i32>,
}

/// The tags of one registry (`TagNetworkSerialization.NetworkPayload`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryTags<'a> {
    /// Registry the tags index into, e.g. `minecraft:block`.
    pub registry: &'a str,
    /// Tags defined for that registry.
    pub tags: Vec<TagEntry<'a>>,
}

/// `minecraft:update_tags`, clientbound (`ClientboundUpdateTagsPacket`).
///
/// Sent once in configuration and again after a datapack reload. Only
/// registries with at least one tag appear: `serializeTagsToNetwork` filters
/// the empty payloads out.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateTags<'a> {
    /// Per-registry tag sets.
    ///
    /// A `Map` on the server, so the order carries no meaning; it is kept as
    /// read so the packet re-encodes byte for byte.
    pub tags: Vec<RegistryTags<'a>>,
}

/// An identifier's length prefix plus a tag count.
const MIN_TAG_MAP_ENTRY_SIZE: usize = 2;

impl Encode for UpdateTags<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        write_count(writer, self.tags.len())?;
        for registry in &self.tags {
            writer.string(registry.registry)?;
            write_count(writer, registry.tags.len())?;
            for tag in &registry.tags {
                writer.string(tag.name)?;
                write_count(writer, tag.entries.len())?;
                for id in &tag.entries {
                    writer.var_int(*id);
                }
            }
        }
        Ok(())
    }
}

impl<'a> Decode<'a> for UpdateTags<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let registry_count = read_count(reader, MIN_TAG_MAP_ENTRY_SIZE)?;
        let mut tags = Vec::with_capacity(registry_count);
        for _ in 0..registry_count {
            let registry = reader.string()?;
            let tag_count = read_count(reader, MIN_TAG_MAP_ENTRY_SIZE)?;
            let mut registry_tags = Vec::with_capacity(tag_count);
            for _ in 0..tag_count {
                let name = reader.string()?;
                let entry_count = read_count(reader, 1)?;
                let mut entries = Vec::with_capacity(entry_count);
                for _ in 0..entry_count {
                    entries.push(reader.var_int()?);
                }
                registry_tags.push(TagEntry { name, entries });
            }
            tags.push(RegistryTags {
                registry,
                tags: registry_tags,
            });
        }
        Ok(Self { tags })
    }
}

// --- keep alive, ping, disconnect -----------------------------------------

/// `minecraft:keep_alive`, both directions (`Clientbound`- and
/// `ServerboundKeepAlivePacket`).
///
/// The two classes have the same one-field body, so one type covers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepAlive {
    /// Opaque value the client echoes back.
    pub id: i64,
}

impl Encode for KeepAlive {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.id);
        Ok(())
    }
}

impl Decode<'_> for KeepAlive {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { id: reader.i64()? })
    }
}

/// `minecraft:ping`, clientbound (`ClientboundPingPacket`).
///
/// Distinct from the status-state ping: this one is a plain `int`, not the
/// `long` [`crate::packets::status::PingRequest`] carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ping {
    /// Opaque value returned in the [`Pong`].
    pub id: i32,
}

impl Encode for Ping {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i32(self.id);
        Ok(())
    }
}

impl Decode<'_> for Ping {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { id: reader.i32()? })
    }
}

/// `minecraft:pong`, serverbound (`ServerboundPongPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pong {
    /// The value from the matching [`Ping`].
    pub id: i32,
}

impl Encode for Pong {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i32(self.id);
        Ok(())
    }
}

impl Decode<'_> for Pong {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { id: reader.i32()? })
    }
}

/// `minecraft:disconnect`, clientbound (`ClientboundDisconnectPacket`).
///
/// The reason is an NBT component, unlike
/// [`crate::packets::login::LoginDisconnect`], which is still JSON because it
/// can fire before any registry has been sent.
#[derive(Debug, Clone, PartialEq)]
pub struct Disconnect<'a> {
    /// Why the connection is being closed.
    pub reason: Component<'a>,
}

impl Encode for Disconnect<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.reason.encode(writer)
    }
}

impl<'a> Decode<'a> for Disconnect<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            reason: Component::decode(reader)?,
        })
    }
}

// --- code of conduct ------------------------------------------------------

/// `minecraft:code_of_conduct`, clientbound
/// (`ClientboundCodeOfConductPacket`).
///
/// New in 26.x. A server that sets one holds the client in configuration until
/// it answers with [`AcceptCodeOfConduct`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeOfConduct<'a> {
    /// The text the client must accept.
    pub code_of_conduct: &'a str,
}

impl Encode for CodeOfConduct<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.code_of_conduct)
    }
}

impl<'a> Decode<'a> for CodeOfConduct<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            code_of_conduct: reader.string()?,
        })
    }
}

// --- empty packets --------------------------------------------------------

/// Define a packet whose codec is `StreamCodec.unit`, i.e. no body at all.
macro_rules! empty_packet {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
        pub struct $name;

        impl Encode for $name {
            fn encode(&self, _writer: &mut Writer) -> Result<()> {
                Ok(())
            }
        }

        impl Decode<'_> for $name {
            fn decode(_reader: &mut Reader<'_>) -> Result<Self> {
                Ok(Self)
            }
        }
    };
}

empty_packet! {
    /// `minecraft:finish_configuration`, clientbound
    /// (`ClientboundFinishConfigurationPacket`).
    ///
    /// Terminal: the server sends it and stops writing configuration packets
    /// until the client answers with [`FinishConfigurationAck`].
    FinishConfiguration
}

empty_packet! {
    /// `minecraft:finish_configuration`, serverbound
    /// (`ServerboundFinishConfigurationPacket`).
    ///
    /// Terminal, and the point the connection enters play. The next thing the
    /// server writes is [`crate::packets::play_login::Login`].
    FinishConfigurationAck
}

empty_packet! {
    /// `minecraft:reset_chat`, clientbound (`ClientboundResetChatPacket`).
    ///
    /// Clears the client's chat session state, which a server sends when it
    /// puts a player back through configuration.
    ResetChat
}

empty_packet! {
    /// `minecraft:accept_code_of_conduct`, serverbound
    /// (`ServerboundAcceptCodeOfConductPacket`).
    AcceptCodeOfConduct
}

// `play_login` has unit packets of its own and shares the definition.
pub(super) use empty_packet;

/// Packets the server sends, generated from `protocol.json`.
///
/// See this module's own note: several of these are also defined above by
/// hand, and these are the ones to keep.
pub mod clientbound {
    include!(concat!(
        env!("OUT_DIR"),
        "/packets/configuration_clientbound.rs"
    ));
}

/// Packets the client sends, generated from `protocol.json`.
pub mod serverbound {
    include!(concat!(
        env!("OUT_DIR"),
        "/packets/configuration_serverbound.rs"
    ));
}
