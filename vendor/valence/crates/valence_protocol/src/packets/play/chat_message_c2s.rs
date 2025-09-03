use valence_bytes::{CowFixedBytes, CowUtf8Bytes};

use crate::{Bounded, DecodeBytes, Encode, FixedBitSet, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ChatMessageC2s<'a> {
    pub message: Bounded<CowUtf8Bytes<'a>, 256>,
    pub timestamp: u64,
    pub salt: u64,
    pub signature: Option<CowFixedBytes<'a, 256>>,
    pub message_count: VarInt,
    // This is a bitset of 20; each bit represents one
    // of the last 20 messages received and whether or not
    // the message was acknowledged by the client
    pub acknowledgement: FixedBitSet<20, 3>,
}
