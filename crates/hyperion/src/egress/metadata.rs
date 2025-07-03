//! Utilities for working with the Entity Metadata packet.

use valence_protocol::{RawBytes, VarInt, packets::play};

/// Packet to show all parts of the skin.
#[must_use]
pub fn show_all(id: i32) -> play::EntityTrackerUpdateS2c<'static> {
    // https://wiki.vg/Entity_metadata#Entity_Metadata_Format
    // https://wiki.vg/Entity_metadata#Player
    // 17 = Metadata, type = byte
    static BYTES: &[u8] = &[17, 0, 0xff, 0xff];

    let entity_id = VarInt(id);

    play::EntityTrackerUpdateS2c {
        entity_id,
        tracked_values: RawBytes(BYTES.into()),
    }
}
