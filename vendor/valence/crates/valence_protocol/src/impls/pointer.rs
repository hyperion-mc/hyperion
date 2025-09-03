use std::borrow::Cow;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;

use valence_bytes::Bytes;

use crate::{Decode, DecodeBytes, Encode};

impl<T: Encode + ?Sized> Encode for &T {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        (**self).encode(w)
    }
}

impl<T: Encode + ?Sized> Encode for &mut T {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        (**self).encode(w)
    }
}

impl<T: Encode + ?Sized> Encode for Box<T> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_ref().encode(w)
    }
}

impl<T: Decode> Decode for Box<T> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        T::decode(r).map(Box::new)
    }
}

impl<T: DecodeBytes> DecodeBytes for Box<T> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        T::decode_bytes(r).map(Box::new)
    }
}

impl<T: Encode + ?Sized> Encode for Rc<T> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_ref().encode(w)
    }
}

impl<T: Decode> Decode for Rc<T> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        T::decode(r).map(Rc::new)
    }
}

impl<T: DecodeBytes> DecodeBytes for Rc<T> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        T::decode_bytes(r).map(Rc::new)
    }
}

impl<T: Encode + ?Sized> Encode for Arc<T> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_ref().encode(w)
    }
}

impl<T: Decode> Decode for Arc<T> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        T::decode(r).map(Arc::new)
    }
}

impl<T: DecodeBytes> DecodeBytes for Arc<T> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        T::decode_bytes(r).map(Arc::new)
    }
}

impl<'a, B> Encode for Cow<'a, B>
where
    B: ToOwned + Encode + ?Sized,
{
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_ref().encode(w)
    }
}

impl<'a, B> Decode for Cow<'a, B>
where
    B: ToOwned + ?Sized,
    B::Owned: Decode,
{
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        B::Owned::decode(r).map(Cow::Owned)
    }
}

impl<'a, B> DecodeBytes for Cow<'a, B>
where
    B: ToOwned + ?Sized,
    B::Owned: DecodeBytes,
{
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        B::Owned::decode_bytes(r).map(Cow::Owned)
    }
}

// impl<'a, 'b, B> Decode<'a> for Cow<'b, B>
// where
//     B: ToOwned + ?Sized,
//     B::Owned: Decode<'a>,
//     &'b B: Decode<'a>,
// {
//     fn decode(r: &mut &'a [u8]) -> anyhow::Result<Self> {
//         let decoded: &'b B = Decode::decode(r)?;
//         Ok(Cow::Borrowed(decoded))
//     }
// }
