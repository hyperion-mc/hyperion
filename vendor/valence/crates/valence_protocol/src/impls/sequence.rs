use std::io::Write;
use std::mem::{self, MaybeUninit};

use anyhow::ensure;
use valence_bytes::{Bytes, CowBytes, CowFixedBytes, FixedBytes};

use crate::impls::cautious_capacity;
use crate::{Bounded, Decode, DecodeBytes, Encode, VarInt};

/// Like tuples, fixed-length arrays are encoded and decoded without a VarInt
/// length prefix.
impl<T: Encode, const N: usize> Encode for [T; N] {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        T::encode_slice(self, w)
    }
}

impl<T: Decode, const N: usize> Decode for [T; N] {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        // TODO: rewrite using std::array::try_from_fn when stabilized?

        let mut data: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };

        for (i, elem) in data.iter_mut().enumerate() {
            match T::decode(r) {
                Ok(val) => {
                    elem.write(val);
                }
                Err(e) => {
                    // Call destructors for values decoded so far.
                    for elem in &mut data[..i] {
                        unsafe { elem.assume_init_drop() };
                    }
                    return Err(e);
                }
            }
        }

        // All values in `data` are initialized.
        unsafe { Ok(mem::transmute_copy(&data)) }
    }
}

impl<T: DecodeBytes, const N: usize> DecodeBytes for [T; N] {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        // TODO: rewrite using std::array::try_from_fn when stabilized?

        let mut data: [MaybeUninit<T>; N] = unsafe { MaybeUninit::uninit().assume_init() };

        for (i, elem) in data.iter_mut().enumerate() {
            match T::decode_bytes(r) {
                Ok(val) => {
                    elem.write(val);
                }
                Err(e) => {
                    // Call destructors for values decoded so far.
                    for elem in &mut data[..i] {
                        unsafe { elem.assume_init_drop() };
                    }
                    return Err(e);
                }
            }
        }

        // All values in `data` are initialized.
        unsafe { Ok(mem::transmute_copy(&data)) }
    }
}

/// References to fixed-length arrays are not length prefixed.
// TODO:
//impl<'a, const N: usize> Decode<'a> for &'a [u8; N] {
//    fn decode(r: &mut &'a [u8]) -> anyhow::Result<Self> {
//        ensure!(
//            r.len() >= N,
//            "not enough data to decode u8 array of length {N}"
//        );
//
//        let (res, remaining) = r.split_at(N);
//        let arr = <&[u8; N]>::try_from(res).unwrap();
//        *r = remaining;
//        Ok(arr)
//    }
//}
impl<T: Encode> Encode for [T] {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        let len = self.len();
        ensure!(
            len <= i32::MAX as usize,
            "length of {} slice exceeds i32::MAX (got {len})",
            std::any::type_name::<T>()
        );

        VarInt(len as i32).encode(&mut w)?;

        T::encode_slice(self, w)
    }
}

impl<T: Encode, const MAX_LEN: usize> Encode for Bounded<&[T], MAX_LEN> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        let len = self.len();
        ensure!(
            len <= MAX_LEN,
            "length of {} slice exceeds max of {MAX_LEN} (got {len})",
            std::any::type_name::<T>(),
        );

        VarInt(len as i32).encode(&mut w)?;

        T::encode_slice(self, w)
    }
}

impl Encode for Bytes {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        let bytes: &[u8] = self;
        bytes.encode(w)
    }
}

impl DecodeBytes for Bytes {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let len = VarInt::decode_bytes(r)?.0;
        ensure!(len >= 0, "attempt to decode slice with negative length");
        let len = len as usize;
        ensure!(
            len <= r.len(),
            "not enough data remaining to decode byte slice (slice len is {len}, but input len is \
             {})",
            r.len()
        );

        Ok(r.split_to(len))
    }
}

impl<const N: usize> Encode for FixedBytes<N> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        <[u8; N]>::encode(self, w)
    }
}

impl<const N: usize> DecodeBytes for FixedBytes<N> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        ensure!(
            N <= r.len(),
            "not enough data remaining to decode byte array (array len is {N}, but input len is \
             {})",
            r.len()
        );

        Ok(r.split_to(N).try_into()?)
    }
}

impl<'a> Encode for CowBytes<'a> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        let bytes: &[u8] = self;
        bytes.encode(w)
    }
}

impl<'a> DecodeBytes for CowBytes<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Bytes::decode_bytes(r).map(Self::Owned)
    }
}

impl<'a, const N: usize> Encode for CowFixedBytes<'a, N> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        <[u8; N]>::encode(self, w)
    }
}

impl<'a, const N: usize> DecodeBytes for CowFixedBytes<'a, N> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        FixedBytes::decode_bytes(r).map(Self::Owned)
    }
}

impl<const MAX_LEN: usize> DecodeBytes for Bounded<Bytes, MAX_LEN> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let res = Bytes::decode_bytes(r)?;

        ensure!(
            res.len() <= MAX_LEN,
            "length of decoded byte slice exceeds max of {MAX_LEN} (got {})",
            res.len()
        );

        Ok(Bounded(res))
    }
}

impl<'a, const MAX_LEN: usize> DecodeBytes for Bounded<CowBytes<'a>, MAX_LEN> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let res = <Bounded<Bytes, MAX_LEN>>::decode_bytes(r)?;

        Ok(Bounded(res.0.into()))
    }
}

impl<T: Encode> Encode for Vec<T> {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        self.as_slice().encode(w)
    }
}

impl<T: Decode> Decode for Vec<T> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let len = VarInt::decode(r)?.0;
        ensure!(len >= 0, "attempt to decode Vec with negative length");
        let len = len as usize;

        let mut vec = Vec::with_capacity(cautious_capacity::<T>(len));

        for _ in 0..len {
            vec.push(T::decode(r)?);
        }

        Ok(vec)
    }
}

impl<T: DecodeBytes> DecodeBytes for Vec<T> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let len = VarInt::decode_bytes(r)?.0;
        ensure!(len >= 0, "attempt to decode Vec with negative length");
        let len = len as usize;

        let mut vec = Vec::with_capacity(cautious_capacity::<T>(len));

        for _ in 0..len {
            vec.push(T::decode_bytes(r)?);
        }

        Ok(vec)
    }
}

impl<T: Decode, const MAX_LEN: usize> Decode for Bounded<Vec<T>, MAX_LEN> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let len = VarInt::decode(r)?.0;
        ensure!(len >= 0, "attempt to decode Vec with negative length");
        let len = len as usize;

        ensure!(
            len <= MAX_LEN,
            "length of Vec exceeds max of {MAX_LEN} (got {len})"
        );

        let mut vec = Vec::with_capacity(len);

        for _ in 0..len {
            vec.push(T::decode(r)?);
        }

        Ok(Bounded(vec))
    }
}

impl<T: DecodeBytes, const MAX_LEN: usize> DecodeBytes for Bounded<Vec<T>, MAX_LEN> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let len = VarInt::decode_bytes(r)?.0;
        ensure!(len >= 0, "attempt to decode Vec with negative length");
        let len = len as usize;

        ensure!(
            len <= MAX_LEN,
            "length of Vec exceeds max of {MAX_LEN} (got {len})"
        );

        let mut vec = Vec::with_capacity(len);

        for _ in 0..len {
            vec.push(T::decode_bytes(r)?);
        }

        Ok(Bounded(vec))
    }
}

impl<T: Decode> Decode for Box<[T]> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Vec::decode(r)?.into_boxed_slice())
    }
}

impl<T: DecodeBytes> DecodeBytes for Box<[T]> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Ok(Vec::decode_bytes(r)?.into_boxed_slice())
    }
}

impl<T: Decode, const MAX_LEN: usize> Decode for Bounded<Box<[T]>, MAX_LEN> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Bounded::<Vec<_>, MAX_LEN>::decode(r)?.map_into())
    }
}

impl<T: DecodeBytes, const MAX_LEN: usize> DecodeBytes for Bounded<Box<[T]>, MAX_LEN> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        Ok(Bounded::<Vec<_>, MAX_LEN>::decode_bytes(r)?.map_into())
    }
}
