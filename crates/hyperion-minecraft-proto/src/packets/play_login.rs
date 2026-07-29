//! The play-state join sequence.
//!
//! Only the packets a connection has to exchange between leaving
//! configuration and standing in a world live here; the rest of play does not.
//! A vanilla server sends [`Login`] first, then [`PlayerPosition`],
//! [`SetDefaultSpawnPosition`] and [`SetChunkCacheCenter`], then chunk data,
//! and finally [`GameEvent`] with [`GameEvent::LEVEL_CHUNKS_LOAD_START`],
//! which is what dismisses the loading screen. The client answers the
//! teleport with [`AcceptTeleportation`] before it will accept its own
//! movement.
//!
//! # Registry ids appear here, not names
//!
//! [`CommonPlayerSpawnInfo::dimension_type`] is a *network id* into
//! `minecraft:dimension_type`, written by
//! `ByteBufCodecs.holderRegistry(Registries.DIMENSION_TYPE)`, which is a bare
//! `VarInt` with no direct-holder escape. The id is positional in the registry
//! the server sent during configuration, so [`Login`] cannot be built without
//! having sent [`crate::packets::configuration::RegistryData`] first. The
//! adjacent `dimension` field is the opposite: a `ResourceKey<Level>`, written
//! as its identifier string, because levels are not a synchronised registry.

use crate::{
    Decode, Encode, Reader, Result, Writer,
    packets::configuration::{empty_packet, read_count, write_count},
};

// --- geometry -------------------------------------------------------------

/// A block position, packed into one `long` (`BlockPos.asLong`).
///
/// 26 bits of x, then 26 of z, then 12 of y in the low bits. The odd order is
/// historical and the field widths come from `PACKED_HORIZONTAL_LENGTH`, which
/// is derived from the 30-million-block world limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct BlockPos {
    /// Block x.
    pub x: i32,
    /// Block y.
    pub y: i32,
    /// Block z.
    pub z: i32,
}

/// `BlockPos.PACKED_HORIZONTAL_LENGTH`, `1 + log2(2^25)`.
const HORIZONTAL_BITS: u32 = 26;
/// `BlockPos.PACKED_Y_LENGTH`.
const Y_BITS: u32 = 64 - 2 * HORIZONTAL_BITS;
/// `BlockPos.X_OFFSET`.
const X_SHIFT: u32 = Y_BITS + HORIZONTAL_BITS;
/// `BlockPos.Z_OFFSET`.
const Z_SHIFT: u32 = Y_BITS;

impl BlockPos {
    /// A position at the given coordinates.
    #[must_use]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The packed form (`BlockPos.asLong`).
    ///
    /// Coordinates outside the field widths wrap, exactly as the server's own
    /// masking does.
    #[must_use]
    pub fn to_packed(self) -> i64 {
        let x = (i64::from(self.x) & mask(HORIZONTAL_BITS)) << X_SHIFT;
        let z = (i64::from(self.z) & mask(HORIZONTAL_BITS)) << Z_SHIFT;
        let y = i64::from(self.y) & mask(Y_BITS);
        x | z | y
    }

    /// Unpack a position (`BlockPos.of`).
    #[must_use]
    pub fn from_packed(packed: i64) -> Self {
        Self {
            x: narrow(sign_extend(packed >> X_SHIFT, HORIZONTAL_BITS)),
            y: narrow(sign_extend(packed, Y_BITS)),
            z: narrow(sign_extend(packed >> Z_SHIFT, HORIZONTAL_BITS)),
        }
    }
}

/// A mask with the low `bits` set.
const fn mask(bits: u32) -> i64 {
    (1i64 << bits) - 1
}

/// The low `bits` of `value`, sign-extended into the full `i64`.
const fn sign_extend(value: i64, bits: u32) -> i64 {
    (value << (64 - bits)) >> (64 - bits)
}

/// Narrow a coordinate `sign_extend` has already confined to 26 bits or fewer.
///
/// # Panics
/// Never: the callers pass 26 and 12, both of which leave the value inside
/// `i32`. The check is here rather than a cast so that changing a field width
/// fails loudly instead of silently truncating.
fn narrow(value: i64) -> i32 {
    i32::try_from(value).expect("sign_extend confines the value to 26 bits")
}

impl Encode for BlockPos {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.to_packed());
        Ok(())
    }
}

impl Decode<'_> for BlockPos {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self::from_packed(reader.i64()?))
    }
}

/// A block position qualified by the level it is in (`GlobalPos`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlobalPos<'a> {
    /// Level key, e.g. `minecraft:overworld`.
    pub dimension: &'a str,
    /// Position within that level.
    pub pos: BlockPos,
}

impl Encode for GlobalPos<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.string(self.dimension)?;
        self.pos.encode(writer)
    }
}

impl<'a> Decode<'a> for GlobalPos<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            dimension: reader.string()?,
            pos: BlockPos::decode(reader)?,
        })
    }
}

/// A position or velocity in world space (`Vec3`), three big-endian doubles.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Vec3 {
    /// x component.
    pub x: f64,
    /// y component.
    pub y: f64,
    /// z component.
    pub z: f64,
}

impl Vec3 {
    /// A vector with the given components.
    #[must_use]
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }
}

impl Encode for Vec3 {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.f64(self.x);
        writer.f64(self.y);
        writer.f64(self.z);
        Ok(())
    }
}

impl Decode<'_> for Vec3 {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.f64()?,
            y: reader.f64()?,
            z: reader.f64()?,
        })
    }
}

// --- game mode ------------------------------------------------------------

/// A game mode.
///
/// Generated, not hand-written: `net.minecraft.world.level.GameType.STREAM_CODEC`
/// is `ByteBufCodecs.idMapper(BY_ID, GameType::getId)`, and the extractor now
/// follows that back to the constant list and the id each constant carries
/// rather than emitting a bare varint. Retyping the four variants here was a
/// second copy of a table the jar already states.
///
/// The re-export keeps this module the place to look for it, since the packets
/// that carry a game mode are here.
pub use crate::types::GameType;

/// `GameType.DEFAULT_MODE`, which is also what an out-of-range id resolves to.
///
/// Hand-written rather than generated: which variant is the default is a fact
/// about the game, not about the wire, and the generator only reads layouts.
impl Default for GameType {
    fn default() -> Self {
        Self::Survival
    }
}

impl GameType {
    /// Resolve a wire id the way `GameType.byId` does.
    ///
    /// `ByIdMap.continuous(..., OutOfBoundsStrategy.ZERO)` maps anything
    /// outside 0..=3 to the first value, so an unknown id is survival rather
    /// than an error. Diverging from that would reject streams the vanilla
    /// client accepts.
    #[must_use]
    pub const fn from_id(id: i8) -> Self {
        match id {
            1 => Self::Creative,
            2 => Self::Adventure,
            3 => Self::Spectator,
            _ => Self::Survival,
        }
    }

    /// Resolve a wire id where `-1` means "none" (`GameType.byNullableId`).
    #[must_use]
    pub const fn from_nullable_id(id: i8) -> Option<Self> {
        if id == -1 {
            None
        } else {
            Some(Self::from_id(id))
        }
    }

    /// The wire id.
    #[must_use]
    pub const fn to_id(self) -> i8 {
        self as i8
    }

    /// The wire id of an optional game mode, `-1` for `None`
    /// (`GameType.getNullableId`).
    #[must_use]
    pub const fn nullable_to_id(value: Option<Self>) -> i8 {
        match value {
            Some(game_type) => game_type.to_id(),
            None => -1,
        }
    }
}

// --- spawn info -----------------------------------------------------------

/// The per-level state a player needs on join or respawn
/// (`CommonPlayerSpawnInfo`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonPlayerSpawnInfo<'a> {
    /// Network id into `minecraft:dimension_type`, not a name.
    pub dimension_type: i32,
    /// Level key, e.g. `minecraft:overworld`.
    pub dimension: &'a str,
    /// First eight bytes of the SHA-256 of the world seed, used for client-side
    /// biome noise.
    pub seed: i64,
    /// Game mode the player is in.
    pub game_type: GameType,
    /// Game mode the player was in, or `None` when there was none.
    pub previous_game_type: Option<GameType>,
    /// Whether this is a debug world.
    pub is_debug: bool,
    /// Whether this is a superflat world, which changes the horizon.
    pub is_flat: bool,
    /// Where the player last died, for the recovery compass.
    pub last_death_location: Option<GlobalPos<'a>>,
    /// Ticks left before the player can use a portal again.
    pub portal_cooldown: i32,
    /// Sea level, which the client needs for fog and ambience.
    pub sea_level: i32,
}

impl Encode for CommonPlayerSpawnInfo<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.dimension_type);
        writer.string(self.dimension)?;
        writer.i64(self.seed);
        // Raw bytes, not the `VarInt` `GameType.STREAM_CODEC` would write:
        // `CommonPlayerSpawnInfo.write` calls `writeByte` directly.
        writer.i8(self.game_type.to_id());
        writer.i8(GameType::nullable_to_id(self.previous_game_type));
        writer.bool(self.is_debug);
        writer.bool(self.is_flat);
        match &self.last_death_location {
            Some(pos) => {
                writer.bool(true);
                pos.encode(writer)?;
            }
            None => writer.bool(false),
        }
        writer.var_int(self.portal_cooldown);
        writer.var_int(self.sea_level);
        Ok(())
    }
}

impl<'a> Decode<'a> for CommonPlayerSpawnInfo<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let dimension_type = reader.var_int()?;
        let dimension = reader.string()?;
        let seed = reader.i64()?;
        let game_type = GameType::from_id(reader.i8()?);
        let previous_game_type = GameType::from_nullable_id(reader.i8()?);
        let is_debug = reader.bool()?;
        let is_flat = reader.bool()?;
        let last_death_location = if reader.bool()? {
            Some(GlobalPos::decode(reader)?)
        } else {
            None
        };
        Ok(Self {
            dimension_type,
            dimension,
            seed,
            game_type,
            previous_game_type,
            is_debug,
            is_flat,
            last_death_location,
            portal_cooldown: reader.var_int()?,
            sea_level: reader.var_int()?,
        })
    }
}

// --- join game ------------------------------------------------------------

/// `minecraft:login`, clientbound (`ClientboundLoginPacket`).
///
/// The join packet. Everything before [`spawn_info`](Self::spawn_info)
/// describes the server; everything inside it describes the level the player
/// is arriving in.
///
/// `player_id` is an `int` and not a `VarInt`: it is the only entity id in the
/// protocol written unpacked, because it predates the `VarInt` conversion and
/// was never changed.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the packet has six independent flags; grouping them would invent structure the wire \
              format does not have"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login<'a> {
    /// Entity id of the player being logged in.
    pub player_id: i32,
    /// Whether the world is hardcore, which changes the death screen.
    pub hardcore: bool,
    /// Every level key the server has, so the client can size its dimension
    /// dropdown and validate a later [`Respawn`].
    pub levels: Vec<&'a str>,
    /// Player limit, only used to size the tab list.
    pub max_players: i32,
    /// Server view distance in chunks.
    pub chunk_radius: i32,
    /// Distance within which entities tick, always at most `chunk_radius`.
    pub simulation_distance: i32,
    /// Whether F3 hides coordinates.
    pub reduced_debug_info: bool,
    /// Whether dying shows the death screen rather than respawning at once.
    pub show_death_screen: bool,
    /// Whether the player can only craft unlocked recipes.
    pub do_limited_crafting: bool,
    /// The level the player is arriving in.
    pub spawn_info: CommonPlayerSpawnInfo<'a>,
    /// Whether the server is in online mode.
    pub online_mode: bool,
    /// Whether the server rejects unsigned chat.
    pub enforces_secure_chat: bool,
}

impl Encode for Login<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i32(self.player_id);
        writer.bool(self.hardcore);
        write_count(writer, self.levels.len())?;
        for level in &self.levels {
            writer.string(level)?;
        }
        writer.var_int(self.max_players);
        writer.var_int(self.chunk_radius);
        writer.var_int(self.simulation_distance);
        writer.bool(self.reduced_debug_info);
        writer.bool(self.show_death_screen);
        writer.bool(self.do_limited_crafting);
        self.spawn_info.encode(writer)?;
        writer.bool(self.online_mode);
        writer.bool(self.enforces_secure_chat);
        Ok(())
    }
}

impl<'a> Decode<'a> for Login<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        let player_id = reader.i32()?;
        let hardcore = reader.bool()?;
        let count = read_count(reader, 1)?;
        let mut levels = Vec::with_capacity(count);
        for _ in 0..count {
            levels.push(reader.string()?);
        }
        Ok(Self {
            player_id,
            hardcore,
            levels,
            max_players: reader.var_int()?,
            chunk_radius: reader.var_int()?,
            simulation_distance: reader.var_int()?,
            reduced_debug_info: reader.bool()?,
            show_death_screen: reader.bool()?,
            do_limited_crafting: reader.bool()?,
            spawn_info: CommonPlayerSpawnInfo::decode(reader)?,
            online_mode: reader.bool()?,
            enforces_secure_chat: reader.bool()?,
        })
    }
}

/// `minecraft:respawn`, clientbound (`ClientboundRespawnPacket`).
///
/// The same level description as [`Login`] without the server-wide fields, so
/// the two share [`CommonPlayerSpawnInfo`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Respawn<'a> {
    /// The level the player is arriving in.
    pub spawn_info: CommonPlayerSpawnInfo<'a>,
    /// Bitmask of state to keep across the respawn.
    pub data_to_keep: i8,
}

impl Respawn<'_> {
    /// `ClientboundRespawnPacket.KEEP_ALL_DATA`.
    pub const KEEP_ALL_DATA: i8 = 3;
    /// `ClientboundRespawnPacket.KEEP_ATTRIBUTE_MODIFIERS`.
    pub const KEEP_ATTRIBUTE_MODIFIERS: i8 = 1;
    /// `ClientboundRespawnPacket.KEEP_ENTITY_DATA`.
    pub const KEEP_ENTITY_DATA: i8 = 2;
}

impl Encode for Respawn<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.spawn_info.encode(writer)?;
        writer.i8(self.data_to_keep);
        Ok(())
    }
}

impl<'a> Decode<'a> for Respawn<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            spawn_info: CommonPlayerSpawnInfo::decode(reader)?,
            data_to_keep: reader.i8()?,
        })
    }
}

// --- game event -----------------------------------------------------------

/// `minecraft:game_event`, clientbound (`ClientboundGameEventPacket`).
///
/// A one-byte event and a float parameter whose meaning depends on it. The
/// event ids are `ClientboundGameEventPacket.Type` instances rather than an
/// enum, so unknown ids are kept as the raw byte here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GameEvent {
    /// Event id.
    pub event: u8,
    /// Event-dependent parameter; zero where the event takes none.
    pub param: f32,
}

impl GameEvent {
    /// Game mode changed; the parameter is the new [`GameType`] id.
    pub const CHANGE_GAME_MODE: u8 = 3;
    /// Demo-mode prompt.
    pub const DEMO_EVENT: u8 = 5;
    /// Elder guardian effect.
    pub const GUARDIAN_ELDER_EFFECT: u8 = 10;
    /// Respawn without the death screen.
    pub const IMMEDIATE_RESPAWN: u8 = 11;
    /// Chunks are on their way; this is what dismisses the loading screen, so
    /// a join sequence that never sends it leaves the client stuck on it.
    pub const LEVEL_CHUNKS_LOAD_START: u8 = 13;
    /// Limited crafting toggled.
    pub const LIMITED_CRAFTING: u8 = 12;
    /// No bed or respawn anchor to respawn at.
    pub const NO_RESPAWN_BLOCK_AVAILABLE: u8 = 0;
    /// An arrow hit a player.
    pub const PLAY_ARROW_HIT_SOUND: u8 = 6;
    /// Pufferfish sting.
    pub const PUFFER_FISH_STING: u8 = 9;
    /// Rain strength changed.
    pub const RAIN_LEVEL_CHANGE: u8 = 7;
    /// Rain started.
    pub const START_RAINING: u8 = 1;
    /// Rain stopped.
    pub const STOP_RAINING: u8 = 2;
    /// Thunder strength changed.
    pub const THUNDER_LEVEL_CHANGE: u8 = 8;
    /// The player won.
    pub const WIN_GAME: u8 = 4;
}

impl Encode for GameEvent {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.u8(self.event);
        writer.f32(self.param);
        Ok(())
    }
}

impl Decode<'_> for GameEvent {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            event: reader.u8()?,
            param: reader.f32()?,
        })
    }
}

// --- set time -------------------------------------------------------------

/// The minimum bytes one clock update occupies on the wire: a `VarInt` clock
/// id, a `VarLong` `total_ticks`, and two `f32`s, so `1 + 1 + 4 + 4`.
const MIN_CLOCK_UPDATE_SIZE: usize = 10;

/// The network id of the overworld clock in `minecraft:world_clock`.
///
/// The registry the server sends during configuration lists
/// `minecraft:overworld` first, so it is id `0`. This is the clock that moves
/// the sun; freezing it freezes the daylight cycle.
pub const OVERWORLD_CLOCK_ID: i32 = 0;

/// One world clock's state on the wire (`ClockNetworkState`).
///
/// The client advances the clock by `rate` ticks each tick and interpolates
/// the sun with `partial_tick`, so it holds the day time without the server
/// resending it. A `rate` of `0.0` freezes the clock: that is exactly what a
/// paused clock -- or the `advance_time` gamerule being off -- serialises to,
/// per `ServerClockManager.ClockInstance.packNetworkState` in 26.2.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockNetworkState {
    /// The clock's accumulated ticks. For the overworld clock this is the day
    /// time that positions the sun (`6000` noon, `18000` midnight). `VarLong`.
    pub total_ticks: i64,
    /// Sub-tick remainder the client interpolates from; `0.0` on a fresh sync.
    pub partial_tick: f32,
    /// Ticks the clock advances per server tick. `1.0` is the vanilla daylight
    /// speed; `0.0` freezes it.
    pub rate: f32,
}

impl ClockNetworkState {
    /// A frozen clock parked at `total_ticks` (`partial_tick` `0.0`, `rate`
    /// `0.0`). The client holds the sun here and never advances it.
    #[must_use]
    pub const fn frozen(total_ticks: i64) -> Self {
        Self {
            total_ticks,
            partial_tick: 0.0,
            rate: 0.0,
        }
    }

    fn encode(&self, writer: &mut Writer) {
        writer.var_long(self.total_ticks);
        writer.f32(self.partial_tick);
        writer.f32(self.rate);
    }

    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            total_ticks: reader.var_long()?,
            partial_tick: reader.f32()?,
            rate: reader.f32()?,
        })
    }
}

/// `minecraft:set_time`, clientbound (`ClientboundSetTimePacket`).
///
/// 26.2 replaced the old `(gameTime, timeOfDay)` pair with a `gameTime` plus a
/// map of per-`WorldClock` states (`ClockNetworkState`). `game_time` is the
/// world age; each clock update carries its own day time and advance `rate`.
///
/// # Freezing the sun
///
/// With no `SetTime` at all the client free-runs its own daylight cycle and
/// the sun drifts. Sending the overworld clock ([`OVERWORLD_CLOCK_ID`]) once
/// with [`ClockNetworkState::frozen`] pins the day time: the client's `rate`
/// is `0.0`, so it holds the sun without the server resending time every tick.
///
/// # Ids, not names
///
/// The map is keyed by the clock's network id in `minecraft:world_clock`,
/// written by `ByteBufCodecs.holderRegistry(Registries.WORLD_CLOCK)`, which is
/// a bare `VarInt` with no direct-holder escape (the same shape as
/// [`CommonPlayerSpawnInfo::dimension_type`]). The id is positional in the
/// registry the server sent during configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct SetTime {
    /// The world age (`Level.getGameTime`). Not the day time; anything that
    /// reads game time still gets the real value while the day time is frozen.
    pub game_time: i64,
    /// Per-clock updates keyed by the clock's `minecraft:world_clock` network
    /// id. A single overworld entry is enough to freeze the sky; vanilla's own
    /// per-clock updates are one-entry maps too.
    pub clock_updates: Vec<(i32, ClockNetworkState)>,
}

impl SetTime {
    /// A `SetTime` that freezes the overworld daylight cycle at `day_time`
    /// while reporting `game_time` as the world age.
    #[must_use]
    pub fn freeze_overworld(game_time: i64, day_time: i64) -> Self {
        Self {
            game_time,
            clock_updates: vec![(OVERWORLD_CLOCK_ID, ClockNetworkState::frozen(day_time))],
        }
    }
}

impl Encode for SetTime {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.i64(self.game_time);
        write_count(writer, self.clock_updates.len())?;
        for (clock_id, state) in &self.clock_updates {
            writer.var_int(*clock_id);
            state.encode(writer);
        }
        Ok(())
    }
}

impl Decode<'_> for SetTime {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        let game_time = reader.i64()?;
        let count = read_count(reader, MIN_CLOCK_UPDATE_SIZE)?;
        let mut clock_updates = Vec::with_capacity(count);
        for _ in 0..count {
            let clock_id = reader.var_int()?;
            clock_updates.push((clock_id, ClockNetworkState::decode(reader)?));
        }
        Ok(Self {
            game_time,
            clock_updates,
        })
    }
}

// --- teleport -------------------------------------------------------------

/// Which fields of a teleport are relative to the player's current state
/// (`Relative`), as a bitmask.
///
/// `Relative.SET_STREAM_CODEC` is `ByteBufCodecs.INT` mapped through
/// `unpack`/`pack`, so this is a plain big-endian `int` rather than a `VarInt`.
/// Bits outside the nine defined ones are dropped on decode, because `unpack`
/// only tests the bits it knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Relative(i32);

impl Relative {
    /// Every defined bit (`Relative.ALL`).
    pub const ALL: Self = Self((1 << 9) - 1);
    /// Velocity x is relative.
    pub const DELTA_X: Self = Self(1 << 5);
    /// Velocity y is relative.
    pub const DELTA_Y: Self = Self(1 << 6);
    /// Velocity z is relative.
    pub const DELTA_Z: Self = Self(1 << 7);
    /// Nothing is relative, i.e. an absolute teleport.
    pub const NONE: Self = Self(0);
    /// Velocity is rotated by the change in rotation.
    pub const ROTATE_DELTA: Self = Self(1 << 8);
    /// x is relative.
    pub const X: Self = Self(1 << 0);
    /// Pitch is relative.
    pub const X_ROT: Self = Self(1 << 4);
    /// y is relative.
    pub const Y: Self = Self(1 << 1);
    /// Yaw is relative.
    pub const Y_ROT: Self = Self(1 << 3);
    /// z is relative.
    pub const Z: Self = Self(1 << 2);

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

    /// The wire value (`Relative.pack`).
    #[must_use]
    pub const fn to_raw(self) -> i32 {
        self.0
    }

    /// Read a wire value, dropping undefined bits (`Relative.unpack`).
    #[must_use]
    pub const fn from_raw(value: i32) -> Self {
        Self(value & Self::ALL.0)
    }
}

/// A position, a velocity and a rotation (`PositionMoveRotation`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PositionMoveRotation {
    /// Where the entity is.
    pub position: Vec3,
    /// How fast it is moving, in blocks per tick.
    pub delta_movement: Vec3,
    /// Yaw in degrees.
    pub y_rot: f32,
    /// Pitch in degrees.
    pub x_rot: f32,
}

impl Encode for PositionMoveRotation {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.position.encode(writer)?;
        self.delta_movement.encode(writer)?;
        writer.f32(self.y_rot);
        writer.f32(self.x_rot);
        Ok(())
    }
}

impl Decode<'_> for PositionMoveRotation {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            position: Vec3::decode(reader)?,
            delta_movement: Vec3::decode(reader)?,
            y_rot: reader.f32()?,
            x_rot: reader.f32()?,
        })
    }
}

/// `minecraft:player_position`, clientbound
/// (`ClientboundPlayerPositionPacket`).
///
/// The client will not accept movement until it has answered one of these with
/// [`AcceptTeleportation`] carrying the same [`id`](Self::id), so the join
/// sequence has to include it even when the position is unchanged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerPosition {
    /// Teleport id the client echoes back.
    pub id: i32,
    /// Where the player is being put.
    pub change: PositionMoveRotation,
    /// Which fields of `change` are offsets rather than absolutes.
    pub relatives: Relative,
}

impl Encode for PlayerPosition {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.id);
        self.change.encode(writer)?;
        writer.i32(self.relatives.to_raw());
        Ok(())
    }
}

impl Decode<'_> for PlayerPosition {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            id: reader.var_int()?,
            change: PositionMoveRotation::decode(reader)?,
            relatives: Relative::from_raw(reader.i32()?),
        })
    }
}

/// `minecraft:accept_teleportation`, serverbound
/// (`ServerboundAcceptTeleportationPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AcceptTeleportation {
    /// The id from the [`PlayerPosition`] being acknowledged.
    pub id: i32,
}

impl Encode for AcceptTeleportation {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.id);
        Ok(())
    }
}

impl Decode<'_> for AcceptTeleportation {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            id: reader.var_int()?,
        })
    }
}

// --- spawn point and chunk window -----------------------------------------

/// `minecraft:set_default_spawn_position`, clientbound
/// (`ClientboundSetDefaultSpawnPositionPacket` wrapping
/// `LevelData.RespawnData`).
///
/// The compass points here. As of 26.x the body is a `GlobalPos` plus a yaw
/// and a pitch, where earlier versions sent a bare `BlockPos` and one angle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SetDefaultSpawnPosition<'a> {
    /// Where the world spawn is.
    pub global_pos: GlobalPos<'a>,
    /// Spawn yaw in degrees, in -180..=180.
    pub yaw: f32,
    /// Spawn pitch in degrees, in -90..=90.
    pub pitch: f32,
}

impl Encode for SetDefaultSpawnPosition<'_> {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        self.global_pos.encode(writer)?;
        writer.f32(self.yaw);
        writer.f32(self.pitch);
        Ok(())
    }
}

impl<'a> Decode<'a> for SetDefaultSpawnPosition<'a> {
    fn decode(reader: &mut Reader<'a>) -> Result<Self> {
        Ok(Self {
            global_pos: GlobalPos::decode(reader)?,
            yaw: reader.f32()?,
            pitch: reader.f32()?,
        })
    }
}

/// `minecraft:set_chunk_cache_center`, clientbound
/// (`ClientboundSetChunkCacheCenterPacket`).
///
/// Tells the client which chunk its view is centred on. Chunks outside the
/// window this implies are discarded, so sending chunk data without having set
/// the centre first loses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetChunkCacheCenter {
    /// Chunk x, i.e. block x shifted right by four.
    pub x: i32,
    /// Chunk z.
    pub z: i32,
}

impl Encode for SetChunkCacheCenter {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.x);
        writer.var_int(self.z);
        Ok(())
    }
}

impl Decode<'_> for SetChunkCacheCenter {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            x: reader.var_int()?,
            z: reader.var_int()?,
        })
    }
}

/// `minecraft:set_chunk_cache_radius`, clientbound
/// (`ClientboundSetChunkCacheRadiusPacket`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetChunkCacheRadius {
    /// View distance in chunks.
    pub radius: i32,
}

impl Encode for SetChunkCacheRadius {
    fn encode(&self, writer: &mut Writer) -> Result<()> {
        writer.var_int(self.radius);
        Ok(())
    }
}

impl Decode<'_> for SetChunkCacheRadius {
    fn decode(reader: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            radius: reader.var_int()?,
        })
    }
}

// --- returning to configuration -------------------------------------------

empty_packet! {
    /// `minecraft:start_configuration`, clientbound
    /// (`ClientboundStartConfigurationPacket`).
    ///
    /// Terminal. Sends a player already in play back to configuration, which
    /// is how a server changes registries or resource packs without a
    /// reconnect.
    StartConfiguration
}

empty_packet! {
    /// `minecraft:configuration_acknowledged`, serverbound
    /// (`ServerboundConfigurationAcknowledgedPacket`).
    ///
    /// Terminal, and the client's half of the switch back.
    ConfigurationAcknowledged
}
