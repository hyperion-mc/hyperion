use std::io::Write;

use bytes::Bytes;
pub use valence_generated::sound::Sound;
use valence_ident::Ident;

use crate::var_int::VarInt;
use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode};

#[derive(Clone, PartialEq, Debug)]
pub enum SoundId {
    Direct { id: Ident, range: Option<f32> },
    Reference { id: VarInt },
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum SoundCategory {
    Master,
    Music,
    Record,
    Weather,
    Block,
    Hostile,
    Neutral,
    Player,
    Ambient,
    Voice,
}

impl Encode for SoundId {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        match self {
            SoundId::Direct { id, range } => {
                VarInt(0).encode(&mut w)?;
                id.encode(&mut w)?;
                range.encode(&mut w)?;
            }
            SoundId::Reference { id } => VarInt(id.0 + 1).encode(&mut w)?,
        }

        Ok(())
    }
}

impl DecodeBytes for SoundId {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let i = VarInt::decode_bytes(r)?.0;

        if i == 0 {
            Ok(SoundId::Direct {
                id: Ident::decode_bytes(r)?,
                range: <Option<f32>>::decode_bytes(r)?,
            })
        } else {
            Ok(SoundId::Reference { id: VarInt(i - 1) })
        }
    }
}
