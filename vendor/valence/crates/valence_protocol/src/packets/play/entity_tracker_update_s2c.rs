use crate::{DecodeBytes, Encode, Packet, RawBytes, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct EntityTrackerUpdateS2c<'a> {
    pub entity_id: VarInt,
    pub tracked_values: RawBytes<'a>,
}
