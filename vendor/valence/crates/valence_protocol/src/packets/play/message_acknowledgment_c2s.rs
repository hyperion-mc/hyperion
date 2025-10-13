use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct MessageAcknowledgmentC2s {
    pub message_count: VarInt,
}
