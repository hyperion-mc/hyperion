//! Wire tests for the play-state join sequence.
//!
//! Provenance is the same as `tests/configuration.rs`: every vector is hex a
//! Java harness printed by running the packet's own `STREAM_CODEC` from the
//! pinned `server-26.2.jar`.
//!
//! [`Login`] and [`Respawn`] need a `RegistryFriendlyByteBuf`, so the harness
//! builds a `MappedRegistry<DimensionType>` and registers the four keys
//! `DimensionTypes.bootstrap` registers, in that order. Only the *id* reaches
//! the wire -- `ByteBufCodecs.holderRegistry` writes a bare `VarInt` -- so the
//! values behind those keys are placeholders and `minecraft:overworld` lands
//! at id 0 the way it does on a vanilla server. Populating them for real needs
//! a datapack load, which a bare harness cannot do; nothing in these vectors
//! depends on it.

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Writer,
    packets::play_login::{
        AcceptTeleportation, BlockPos, CommonPlayerSpawnInfo, ConfigurationAcknowledged, GameEvent,
        GameType, GlobalPos, Login, PlayerPosition, PositionMoveRotation, Relative, Respawn,
        SetChunkCacheCenter, SetChunkCacheRadius, SetDefaultSpawnPosition, StartConfiguration,
        Vec3,
    },
};

fn hex(text: &str) -> Vec<u8> {
    assert!(
        text.len().is_multiple_of(2),
        "hex fixture has an odd length"
    );
    (0..text.len() / 2)
        .map(|index| {
            u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).expect("hex fixture digit")
        })
        .collect()
}

fn encode<T: Encode>(value: &T) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

fn round_trip<'a, T>(value: &T, bytes: &'a [u8])
where
    T: Encode + Decode<'a> + PartialEq + std::fmt::Debug,
{
    assert_eq!(encode(value), bytes, "encoding mismatch");
    let mut reader = Reader::new(bytes);
    let decoded = T::decode(&mut reader).expect("decode");
    reader.finish().expect("packet body fully consumed");
    assert_eq!(&decoded, value, "decoding mismatch");
}

/// The spawn info the harness built, shared by the login and respawn vectors.
const fn spawn_info() -> CommonPlayerSpawnInfo<'static> {
    CommonPlayerSpawnInfo {
        dimension_type: 0,
        dimension: "minecraft:overworld",
        seed: 0x0123_4567_89ab_cdef,
        game_type: GameType::Creative,
        previous_game_type: Some(GameType::Survival),
        is_debug: false,
        is_flat: true,
        last_death_location: Some(GlobalPos {
            dimension: "minecraft:the_nether",
            pos: BlockPos::new(-1, 70, 2),
        }),
        portal_cooldown: 30,
        sea_level: 63,
    }
}

// --- block positions ------------------------------------------------------

#[test]
fn block_pos_packs_the_way_vanilla_does() {
    // Taken out of the set_default_spawn_position vector below, where
    // BlockPos(1, 65, -3) is the eight bytes after the dimension string.
    assert_eq!(BlockPos::new(1, 65, -3).to_packed(), 0x0000_007f_ffff_d041);
    assert_eq!(
        BlockPos::from_packed(0x0000_007f_ffff_d041),
        BlockPos::new(1, 65, -3)
    );
}

#[test]
fn block_pos_sign_extends_every_field() {
    // 26 bits of x and z, 12 of y, each signed. The extremes are what catch a
    // shift that is logical where it should be arithmetic.
    for pos in [
        BlockPos::new(0, 0, 0),
        BlockPos::new(-1, -1, -1),
        BlockPos::new(30_000_000, 2047, -30_000_000),
        BlockPos::new(-30_000_000, -2048, 30_000_000),
    ] {
        assert_eq!(BlockPos::from_packed(pos.to_packed()), pos, "{pos:?}");
    }
}

// --- join -----------------------------------------------------------------

#[test]
fn login_matches_vanilla() {
    // ClientboundLoginPacket(42, false, {overworld, the_nether}, 20, 10, 8,
    //     false, true, false, spawn, true, false)
    let bytes = hex("0000002a0002136d696e6563726166743a6f766572776f726c64146d696e6563726166743a7468655f6e6574686572140a0800010000136d696e6563726166743a6f766572776f726c640123456789abcdef0100000101146d696e6563726166743a7468655f6e6574686572ffffffc0000020461e3f0100");
    round_trip(
        &Login {
            player_id: 42,
            hardcore: false,
            levels: vec!["minecraft:overworld", "minecraft:the_nether"],
            max_players: 20,
            chunk_radius: 10,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            do_limited_crafting: false,
            spawn_info: spawn_info(),
            online_mode: true,
            enforces_secure_chat: false,
        },
        &bytes,
    );
}

#[test]
fn login_with_no_previous_game_mode_matches_vanilla() {
    // The same packet with previousGameType null and no death location, which
    // is what a first join looks like. `getNullableId(null)` is -1, so the
    // byte is 0xff and not a VarInt.
    let bytes = hex("000000010101136d696e6563726166743a6f766572776f726c6401020201000100136d696e6563726166743a6f766572776f726c64000000000000000000ff000000003f0001");
    round_trip(
        &Login {
            player_id: 1,
            hardcore: true,
            levels: vec!["minecraft:overworld"],
            max_players: 1,
            chunk_radius: 2,
            simulation_distance: 2,
            reduced_debug_info: true,
            show_death_screen: false,
            do_limited_crafting: true,
            spawn_info: CommonPlayerSpawnInfo {
                dimension_type: 0,
                dimension: "minecraft:overworld",
                seed: 0,
                game_type: GameType::Survival,
                previous_game_type: None,
                is_debug: false,
                is_flat: false,
                last_death_location: None,
                portal_cooldown: 0,
                sea_level: 63,
            },
            online_mode: false,
            enforces_secure_chat: true,
        },
        &bytes,
    );
}

#[test]
fn respawn_matches_vanilla() {
    // ClientboundRespawnPacket(spawn, KEEP_ALL_DATA). Everything before the
    // trailing 0x03 is CommonPlayerSpawnInfo, shared with the login packet.
    let bytes = hex("00136d696e6563726166743a6f766572776f726c640123456789abcdef0100000101146d696e6563726166743a7468655f6e6574686572ffffffc0000020461e3f03");
    round_trip(
        &Respawn {
            spawn_info: spawn_info(),
            data_to_keep: Respawn::KEEP_ALL_DATA,
        },
        &bytes,
    );
}

#[test]
fn an_unknown_game_mode_id_is_survival() {
    // GameType.byId goes through ByIdMap.continuous with
    // OutOfBoundsStrategy.ZERO, so anything out of range is the first value
    // rather than an error. Rejecting it here would refuse a stream the
    // vanilla client accepts.
    assert_eq!(GameType::from_id(9), GameType::Survival);
    assert_eq!(GameType::from_id(-2), GameType::Survival);
    assert_eq!(GameType::from_nullable_id(-1), None);
    assert_eq!(GameType::from_nullable_id(3), Some(GameType::Spectator));
    assert_eq!(GameType::nullable_to_id(None), -1);
}

// --- game event -----------------------------------------------------------

#[test]
fn game_event_matches_vanilla() {
    // ClientboundGameEventPacket(LEVEL_CHUNKS_LOAD_START, 0.0f). The event is
    // a raw byte, not a VarInt.
    round_trip(
        &GameEvent {
            event: GameEvent::LEVEL_CHUNKS_LOAD_START,
            param: 0.0,
        },
        &hex("0d00000000"),
    );
    // ClientboundGameEventPacket(CHANGE_GAME_MODE, 1.0f), whose parameter is a
    // GameType id carried as a float.
    round_trip(
        &GameEvent {
            event: GameEvent::CHANGE_GAME_MODE,
            param: 1.0,
        },
        &hex("033f800000"),
    );
}

// --- teleport -------------------------------------------------------------

#[test]
fn player_position_matches_vanilla() {
    // ClientboundPlayerPositionPacket(7,
    //     PositionMoveRotation(Vec3(1.5, 64, -2.25), Vec3.ZERO, 90f, -12.5f),
    //     Set.of(X, Y_ROT))
    let bytes = hex("073ff80000000000004050000000000000c00200000000000000000000000000000000000000000000000000000000000042b40000c148000000000009");
    round_trip(
        &PlayerPosition {
            id: 7,
            change: PositionMoveRotation {
                position: Vec3::new(1.5, 64.0, -2.25),
                delta_movement: Vec3::default(),
                y_rot: 90.0,
                x_rot: -12.5,
            },
            relatives: Relative::X.union(Relative::Y_ROT),
        },
        &bytes,
    );
}

#[test]
fn relative_flags_are_a_plain_int() {
    // Relative.SET_STREAM_CODEC is ByteBufCodecs.INT mapped through
    // unpack/pack, so the mask occupies four fixed bytes; the trailing
    // 00000009 of the vector above is X | Y_ROT.
    assert_eq!(Relative::X.union(Relative::Y_ROT).to_raw(), 9);
    assert_eq!(Relative::ALL.to_raw(), 0x1ff);
    assert!(Relative::ALL.contains(Relative::ROTATE_DELTA));
    assert!(!Relative::NONE.contains(Relative::X));
    // Relative.unpack only tests the nine bits it knows, so anything above
    // them is dropped rather than round-tripped.
    assert_eq!(Relative::from_raw(-1), Relative::ALL);
}

#[test]
fn accept_teleportation_matches_vanilla() {
    round_trip(&AcceptTeleportation { id: 7 }, &hex("07"));
}

// --- spawn point and chunk window -----------------------------------------

#[test]
fn set_default_spawn_position_matches_vanilla() {
    // ClientboundSetDefaultSpawnPositionPacket(RespawnData(
    //     GlobalPos(overworld, BlockPos(1, 65, -3)), 45f, 0f))
    let bytes = hex("136d696e6563726166743a6f766572776f726c640000007fffffd0414234000000000000");
    round_trip(
        &SetDefaultSpawnPosition {
            global_pos: GlobalPos {
                dimension: "minecraft:overworld",
                pos: BlockPos::new(1, 65, -3),
            },
            yaw: 45.0,
            pitch: 0.0,
        },
        &bytes,
    );
}

#[test]
fn set_chunk_cache_center_matches_vanilla() {
    // A negative chunk coordinate is a VarInt, so it costs the full five
    // bytes.
    round_trip(&SetChunkCacheCenter { x: 3, z: -4 }, &hex("03fcffffff0f"));
}

#[test]
fn set_chunk_cache_radius_matches_vanilla() {
    round_trip(&SetChunkCacheRadius { radius: 10 }, &hex("0a"));
}

// --- returning to configuration -------------------------------------------

#[test]
fn configuration_switch_packets_have_no_body() {
    round_trip(&StartConfiguration, &[]);
    round_trip(&ConfigurationAcknowledged, &[]);
}
