use valence_bytes::CowUtf8Bytes;

use crate::{Bounded, DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct RequestCommandCompletionsC2s<'a> {
    pub transaction_id: VarInt,
    pub text: Bounded<CowUtf8Bytes<'a>, 32500>,
}
