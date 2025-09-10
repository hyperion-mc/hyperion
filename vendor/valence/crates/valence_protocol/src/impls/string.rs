use std::io::Write;
use std::str::FromStr;

use anyhow::{Context, ensure};
use bytes::Bytes;
use valence_bytes::{CowUtf8Bytes, Utf8Bytes};
use valence_text::Text;

use crate::{Bounded, Decode, DecodeBytes, Encode, VarInt, impl_decode_bytes_auto};

const DEFAULT_MAX_STRING_CHARS: usize = 32767;
const MAX_TEXT_CHARS: usize = 262144;

impl Encode for str {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        Bounded::<_, DEFAULT_MAX_STRING_CHARS>(self).encode(w)
    }
}

impl<const MAX_CHARS: usize> Encode for Bounded<&str, MAX_CHARS> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        let char_count = self.encode_utf16().count();

        ensure!(
            char_count <= MAX_CHARS,
            "char count of string exceeds maximum (expected <= {MAX_CHARS}, got {char_count})"
        );

        VarInt(self.len() as i32).encode(&mut w)?;
        Ok(w.write_all(self.as_bytes())?)
    }
}

impl Encode for String {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_str().encode(w)
    }
}

impl<const MAX_CHARS: usize> Encode for Bounded<String, MAX_CHARS> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        Bounded::<_, MAX_CHARS>(self.as_str()).encode(w)
    }
}

// TODO:
// impl Decode for String {
//     fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
//         Ok(<&str>::decode(r)?.into())
//     }
// }
//
// impl<const MAX_CHARS: usize> Decode for Bounded<String, MAX_CHARS> {
//     fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
//         Ok(Bounded(Bounded::<&str, MAX_CHARS>::decode(r)?.0.into()))
//     }
// }
//
// impl Decode for Box<str> {
//     fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
//         Ok(<&str>::decode(r)?.into())
//     }
// }
//
// impl<const MAX_CHARS: usize> Decode for Bounded<Box<str>, MAX_CHARS> {
//     fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
//         Ok(Bounded(Bounded::<&str, MAX_CHARS>::decode(r)?.0.into()))
//     }
// }

impl Encode for Text {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        let s = serde_json::to_string(self).context("serializing text JSON")?;

        Bounded::<_, MAX_TEXT_CHARS>(s).encode(w)
    }
}

impl Decode for Text {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        // TODO: don't do this
        let len = VarInt::decode(r)?.0;
        ensure!(len >= 0, "attempt to decode string with negative length");
        let len = len as usize;
        ensure!(
            len <= r.len(),
            "not enough data remaining ({} bytes) to decode string of {len} bytes",
            r.len()
        );

        let (res, remaining) = r.split_at(len);
        *r = remaining;
        let res = Utf8Bytes::new(res.to_owned().into())?;

        let char_count = res.encode_utf16().count();
        ensure!(
            char_count <= MAX_TEXT_CHARS,
            "char count of string exceeds maximum (expected <= {MAX_TEXT_CHARS}, got {char_count})"
        );

        *r = remaining;

        Self::from_str(&res).context("deserializing text JSON")
    }
}

impl_decode_bytes_auto!(Text);

impl Encode for Utf8Bytes {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        <str>::encode(self, w)
    }
}

impl DecodeBytes for Utf8Bytes {
    fn decode_bytes(bytes: &mut Bytes) -> anyhow::Result<Self> {
        Ok(Bytes::decode_bytes(bytes)?.try_into()?)
    }
}

impl<'a> Encode for CowUtf8Bytes<'a> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        <str>::encode(self, w)
    }
}

impl<'a> DecodeBytes for CowUtf8Bytes<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Utf8Bytes::decode_bytes(r).map(Self::Owned)
    }
}

impl<const MAX_CHARS: usize> Encode for Bounded<Utf8Bytes, MAX_CHARS> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        Bounded::<_, MAX_CHARS>(self.as_str()).encode(w)
    }
}

impl<const MAX_CHARS: usize> DecodeBytes for Bounded<Utf8Bytes, MAX_CHARS> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let len = VarInt::decode_bytes(r)?.0;
        ensure!(len >= 0, "attempt to decode string with negative length");
        let len = len as usize;
        ensure!(
            len <= r.len(),
            "not enough data remaining ({} bytes) to decode string of {len} bytes",
            r.len()
        );

        let res = r.split_to(len);
        let res = Utf8Bytes::new(res)?;

        let char_count = res.encode_utf16().count();
        ensure!(
            char_count <= MAX_CHARS,
            "char count of Utf8Bytes exceeds maximum (expected <= {MAX_CHARS}, got {char_count})"
        );

        Ok(Bounded(res))
    }
}

impl<'a, const MAX_CHARS: usize> Encode for Bounded<CowUtf8Bytes<'a>, MAX_CHARS> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        Bounded::<_, MAX_CHARS>(self.as_str()).encode(w)
    }
}

impl<'a, const MAX_CHARS: usize> DecodeBytes for Bounded<CowUtf8Bytes<'a>, MAX_CHARS> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let res = <Bounded<Utf8Bytes, MAX_CHARS>>::decode_bytes(r)?;

        Ok(Bounded(res.0.into()))
    }
}
