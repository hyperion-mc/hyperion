use std::io::Write;

use anyhow::ensure;
use bytes::Bytes;

use crate::var_int::VarInt;
use crate::{Decode, DecodeBytes, Encode};

/// A fixed-size array encoded and decoded with a [`VarInt`] length prefix.
///
/// This is used when the length of the array is known statically, but a
/// length prefix is needed anyway.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
#[repr(transparent)]
pub struct FixedArray<T, const N: usize>(pub [T; N]);

impl<T: Encode, const N: usize> Encode for FixedArray<T, N> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        VarInt(N as i32).encode(&mut w)?;
        self.0.encode(w)
    }
}

impl<T: Decode, const N: usize> Decode for FixedArray<T, N> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let len = VarInt::decode(r)?.0;
        ensure!(
            len == N as i32,
            "unexpected length of {len} for fixed-sized array of length {N}"
        );

        <[T; N]>::decode(r).map(FixedArray)
    }
}

impl<T: DecodeBytes, const N: usize> DecodeBytes for FixedArray<T, N> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let len = VarInt::decode_bytes(r)?.0;
        ensure!(
            len == N as i32,
            "unexpected length of {len} for fixed-sized array of length {N}"
        );

        <[T; N]>::decode_bytes(r).map(FixedArray)
    }
}

impl<T, const N: usize> From<[T; N]> for FixedArray<T, N> {
    fn from(value: [T; N]) -> Self {
        Self(value)
    }
}

impl<T, const N: usize> From<FixedArray<T, N>> for [T; N] {
    fn from(value: FixedArray<T, N>) -> Self {
        value.0
    }
}
