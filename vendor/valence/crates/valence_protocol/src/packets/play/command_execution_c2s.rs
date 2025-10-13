use valence_bytes::{CowFixedBytes, CowUtf8Bytes};

use crate::{Bounded, DecodeBytes, Encode, FixedBitSet, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct CommandExecutionC2s<'a> {
    pub command: Bounded<CowUtf8Bytes<'a>, 256>,
    pub timestamp: u64,
    pub salt: u64,
    pub argument_signatures: Vec<CommandArgumentSignature<'a>>,
    pub message_count: VarInt,
    //// This is a bitset of 20; each bit represents one
    //// of the last 20 messages received and whether or not
    //// the message was acknowledged by the client
    pub acknowledgement: FixedBitSet<20, 3>,
}

#[derive(Clone, Debug, Encode, DecodeBytes)]
pub struct CommandArgumentSignature<'a> {
    pub argument_name: Bounded<CowUtf8Bytes<'a>, 16>,
    pub signature: CowFixedBytes<'a, 256>,
}
