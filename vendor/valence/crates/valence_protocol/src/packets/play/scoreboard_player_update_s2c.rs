use valence_bytes::CowUtf8Bytes;

use crate::{DecodeBytes, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct ScoreboardPlayerUpdateS2c<'a> {
    pub entity_name: CowUtf8Bytes<'a>,
    pub action: ScoreboardPlayerUpdateAction<'a>,
}

#[derive(Clone, PartialEq, Debug, Encode, DecodeBytes)]
pub enum ScoreboardPlayerUpdateAction<'a> {
    Update {
        objective_name: CowUtf8Bytes<'a>,
        objective_score: VarInt,
    },
    Remove {
        objective_name: CowUtf8Bytes<'a>,
    },
}
