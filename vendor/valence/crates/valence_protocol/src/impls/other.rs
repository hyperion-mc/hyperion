use std::io::Write;

use anyhow::Context;
use bytes::Bytes;
use uuid::Uuid;
use valence_bytes::Utf8Bytes;
use valence_generated::block::{BlockEntityKind, BlockKind, BlockState};
use valence_generated::item::ItemKind;
use valence_ident::Ident;
use valence_nbt::Compound;

use crate::{Decode, DecodeBytes, Encode, VarInt, impl_decode_bytes_auto};

impl<T: Encode> Encode for Option<T> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        match self {
            Some(t) => {
                true.encode(&mut w)?;
                t.encode(w)
            }
            None => false.encode(w),
        }
    }
}

impl<T: Decode> Decode for Option<T> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(match bool::decode(r)? {
            true => Some(T::decode(r)?),
            false => None,
        })
    }
}

impl<T: DecodeBytes> DecodeBytes for Option<T> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Ok(match bool::decode_bytes(r)? {
            true => Some(T::decode_bytes(r)?),
            false => None,
        })
    }
}

impl Encode for Uuid {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_u128().encode(w)
    }
}

impl Decode for Uuid {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        u128::decode(r).map(Uuid::from_u128)
    }
}

impl_decode_bytes_auto!(Uuid);

impl Encode for Compound {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        Ok(valence_nbt::to_binary(self, w, "")?)
    }
}

impl Decode for Compound {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        // Check for null compound.
        if r.first() == Some(&0) {
            *r = &r[1..];
            return Ok(Compound::new());
        }

        // TODO: consider if we need to bound the input slice or add some other
        // mitigation to prevent excessive memory usage on hostile input.
        Ok(valence_nbt::from_binary(r)?.0)
    }
}

impl_decode_bytes_auto!(Compound);

impl Encode for Ident {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_ref().encode(w)
    }
}

impl DecodeBytes for Ident {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Ok(Ident::try_from(Utf8Bytes::decode_bytes(r)?)?)
    }
}

impl Encode for BlockState {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        VarInt(self.to_raw() as i32).encode(w)
    }
}

impl Decode for BlockState {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let id = VarInt::decode(r)?.0;
        let errmsg = "invalid block state ID";

        BlockState::from_raw(id.try_into().context(errmsg)?).context(errmsg)
    }
}

impl_decode_bytes_auto!(BlockState);

impl Encode for BlockKind {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        VarInt(self.to_raw() as i32).encode(w)
    }
}

impl Decode for BlockKind {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let id = VarInt::decode(r)?.0;
        let errmsg = "invalid block kind ID";

        BlockKind::from_raw(id.try_into().context(errmsg)?).context(errmsg)
    }
}

impl_decode_bytes_auto!(BlockKind);

impl Encode for BlockEntityKind {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        VarInt(self.id() as i32).encode(w)
    }
}

impl Decode for BlockEntityKind {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let id = VarInt::decode(r)?;
        Self::from_id(id.0 as u32).with_context(|| format!("id {}", id.0))
    }
}

impl_decode_bytes_auto!(BlockEntityKind);

impl Encode for ItemKind {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        VarInt(self.to_raw() as i32).encode(w)
    }
}

impl Decode for ItemKind {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let id = VarInt::decode(r)?.0;
        let errmsg = "invalid item ID";

        ItemKind::from_raw(id.try_into().context(errmsg)?).context(errmsg)
    }
}

impl_decode_bytes_auto!(ItemKind);
