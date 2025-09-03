use std::io::Write;

use valence_bytes::Bytes;
use valence_ident::Ident;

use crate::sound::SoundCategory;
use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, PartialEq, Debug, Packet)]
pub struct StopSoundS2c {
    pub source: Option<SoundCategory>,
    pub sound: Option<Ident>,
}

impl Encode for StopSoundS2c {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        match (self.source, self.sound.as_ref()) {
            (Some(source), Some(sound)) => {
                3i8.encode(&mut w)?;
                source.encode(&mut w)?;
                sound.encode(&mut w)?;
            }
            (None, Some(sound)) => {
                2i8.encode(&mut w)?;
                sound.encode(&mut w)?;
            }
            (Some(source), None) => {
                1i8.encode(&mut w)?;
                source.encode(&mut w)?;
            }
            _ => 0i8.encode(&mut w)?,
        }

        Ok(())
    }
}

impl DecodeBytes for StopSoundS2c {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let (source, sound) = match i8::decode_bytes(r)? {
            3 => (
                Some(SoundCategory::decode_bytes(r)?),
                Some(Ident::decode_bytes(r)?),
            ),
            2 => (None, Some(<Ident>::decode_bytes(r)?)),
            1 => (Some(SoundCategory::decode_bytes(r)?), None),
            _ => (None, None),
        };

        Ok(Self { source, sound })
    }
}
