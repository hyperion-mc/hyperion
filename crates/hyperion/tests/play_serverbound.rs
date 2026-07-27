//! The serverbound play id table, checked against protocol 776 rather than the
//! 763 one hyperion used to decode with.
//!
//! Every id moved between those versions, and a stale table fails silently:
//! the frame decodes, a handler runs, and the values are plausible. These
//! tests pin the ids a playing client depends on, and pin that an id the
//! server cannot place is reported instead of dropped.

use hyperion::{
    net::{PROTOCOL_VERSION, protocol::decode_body},
    simulation::handlers::{Route, route},
};
use hyperion_minecraft_proto::{
    Reader,
    generated::packet_id::play::serverbound::PacketId,
    packets::play::serverbound::{MovePlayerPos, MovePlayerPosRot},
};

/// One `move_player_pos_rot` frame as a 26.2 client sends it, after the length
/// prefix and compression have been stripped: a `VarInt` id followed by the
/// body.
///
/// x = 8.5, y = 65.0, z = -12.25 as big-endian `f64`; yaw = 90.0,
/// pitch = -12.5 as big-endian `f32`; then the flags byte with the on-ground
/// bit set.
const MOVE_PLAYER_POS_ROT_FRAME: &[u8] = &[
    0x1f, // VarInt id 31
    0x40, 0x21, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // x
    0x40, 0x50, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, // y
    0xc0, 0x28, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, // z
    0x42, 0xb4, 0x00, 0x00, // yaw
    0xc1, 0x48, 0x00, 0x00, // pitch
    0x01, // flags: on ground, no horizontal collision
];

#[test]
fn a_captured_movement_frame_decodes_at_its_776_id() {
    let mut reader = Reader::new(MOVE_PLAYER_POS_ROT_FRAME);
    let id = reader.var_int().expect("frame carries a VarInt id");

    assert_eq!(
        PacketId::from_raw(id),
        Some(PacketId::MovePlayerPosRot),
        "id {id} must be move_player_pos_rot in protocol {PROTOCOL_VERSION}"
    );
    assert!(
        matches!(route(id), Route::Act(_)),
        "a client that cannot move is the whole point of this port"
    );

    let packet: MovePlayerPosRot =
        decode_body(reader.remaining()).expect("body must match the 776 layout");

    // Bit patterns rather than values: the point of the test is that the
    // fields line up with the bytes, and every value here is exact in binary.
    assert_eq!(packet.x.to_bits(), 8.5_f64.to_bits());
    assert_eq!(packet.y.to_bits(), 65.0_f64.to_bits());
    assert_eq!(packet.z.to_bits(), (-12.25_f64).to_bits());
    assert_eq!(packet.y_rot.to_bits(), 90.0_f32.to_bits());
    assert_eq!(packet.x_rot.to_bits(), (-12.5_f32).to_bits());
    assert_eq!(packet.on_ground, 1);
}

/// The same bytes read against the 763 table would be a different packet
/// entirely, which is the failure this port exists to remove: valence's
/// `FullC2s` is id 21 and `PositionAndOnGround` is 20, and neither number
/// means a movement packet in 776.
#[test]
fn the_763_movement_ids_are_not_movement_in_776() {
    for (valence_id, valence_name) in [
        (20, "position_and_on_ground"),
        (21, "full"),
        (22, "look_and_on_ground"),
        (23, "on_ground_only"),
    ] {
        let resolved = PacketId::from_raw(valence_id);
        assert!(
            !matches!(
                resolved,
                Some(
                    PacketId::MovePlayerPos
                        | PacketId::MovePlayerPosRot
                        | PacketId::MovePlayerRot
                        | PacketId::MovePlayerStatusOnly
                )
            ),
            "763's {valence_name} (id {valence_id}) resolved to {resolved:?}; the table is still \
             763"
        );
    }

    assert_eq!(PacketId::MovePlayerPos.to_raw(), 30);
    assert_eq!(PacketId::MovePlayerPosRot.to_raw(), 31);
    assert_eq!(PacketId::MovePlayerRot.to_raw(), 32);
    assert_eq!(PacketId::MovePlayerStatusOnly.to_raw(), 33);
}

/// A movement body is fixed width, so reading it at the wrong id would consume
/// the wrong number of bytes. `decode_body` refuses to leave any, which is
/// what turns a mismatched layout into an error rather than a value that is
/// silently off.
#[test]
fn a_movement_body_read_as_the_wrong_movement_packet_fails() {
    let body = &MOVE_PLAYER_POS_ROT_FRAME[1..];

    decode_body::<MovePlayerPos>(body)
        .expect_err("move_player_pos is eight bytes shorter and must not accept this body");
}

#[test]
fn an_id_outside_the_776_table_is_reported() {
    // 200 is past the last serverbound play id (68) and will stay past it for
    // as long as the protocol has fewer than 200 of them.
    let unknown = 200;
    assert_eq!(PacketId::from_raw(unknown), None);
    assert!(
        matches!(route(unknown), Route::Unknown(id) if id == unknown),
        "an id the protocol does not define must be reported, not dropped"
    );
}

/// Nothing in the 776 table may fall through to the unknown arm: that arm
/// means the id table and the dispatcher disagree, which is the state this
/// port started from.
#[test]
fn every_776_serverbound_id_is_classified() {
    for &packet in PacketId::ALL {
        let id = packet.to_raw();
        assert_eq!(PacketId::from_raw(id), Some(packet));
        assert!(
            !matches!(route(id), Route::Unknown(_)),
            "{packet:?} (id {id}) is in the 776 table but the dispatcher does not know it"
        );
    }
}

/// The packets a player needs to move, look around, fight, talk and hold
/// something. Losing any of these is not a missing feature, it is a player who
/// is stuck in the world, so they are named rather than counted.
#[test]
fn the_packets_a_player_needs_are_handled() {
    for packet in [
        PacketId::AcceptTeleportation,
        PacketId::Attack,
        PacketId::Chat,
        PacketId::ChatCommand,
        PacketId::ClientCommand,
        PacketId::ClientInformation,
        PacketId::CommandSuggestion,
        PacketId::ContainerClose,
        PacketId::MovePlayerPos,
        PacketId::MovePlayerPosRot,
        PacketId::MovePlayerRot,
        PacketId::MovePlayerStatusOnly,
        PacketId::PlayerAbilities,
        PacketId::PlayerAction,
        PacketId::PlayerCommand,
        PacketId::PlayerInput,
        PacketId::SetCarriedItem,
        PacketId::Swing,
        PacketId::UseItem,
        PacketId::UseItemOn,
    ] {
        assert!(
            matches!(route(packet.to_raw()), Route::Act(_)),
            "{packet:?} must be handled"
        );
    }
}
