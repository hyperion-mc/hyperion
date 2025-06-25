//! Utilities for working with the Entity Metadata packet.

use valence_bytes::Bytes;
use valence_protocol::{Encode, RawBytes, VarInt, packets::play};

/// Packet to show all parts of the skin.
#[must_use]
pub fn show_all(id: i32) -> play::EntityTrackerUpdateS2c<'static> {
    let entity_id = VarInt(id);

    // https://wiki.vg/Entity_metadata#Entity_Metadata_Format
    // https://wiki.vg/Entity_metadata#Player
    // 17 = Metadata, type = byte
    let mut bytes = Vec::new();
    bytes.push(17_u8);

    #[expect(clippy::unwrap_used, reason = "this should never fail")]
    VarInt(0).encode(&mut bytes).unwrap();

    // all 1s
    #[expect(clippy::unwrap_used, reason = "this should never fail")]
    u8::MAX.encode(&mut bytes).unwrap();

    // end with 0xff
    bytes.push(0xff);

    play::EntityTrackerUpdateS2c {
        entity_id,
        tracked_values: RawBytes(Bytes::from(bytes).into()),
    }
}
