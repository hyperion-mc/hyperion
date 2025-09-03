use std::io::Write;

use bytes::Bytes;

use crate::{Decode, DecodeBytes, Encode};

macro_rules! impl_tuple {
    ($($ty:ident)*) => {
        #[allow(non_snake_case)]
        impl<$($ty: Encode,)*> Encode for ($($ty,)*) {
            fn encode(&self, mut _w: impl Write) -> anyhow::Result<()> {
                let ($($ty,)*) = self;
                $(
                    $ty.encode(&mut _w)?;
                )*
                Ok(())
            }
        }

        impl<$($ty: Decode,)*> Decode for ($($ty,)*) {
            fn decode(_r: &mut &[u8]) -> anyhow::Result<Self> {
                Ok(($($ty::decode(_r)?,)*))
            }
        }

        impl<$($ty: DecodeBytes,)*> DecodeBytes for ($($ty,)*) {
            fn decode_bytes(_r: &mut Bytes) -> anyhow::Result<Self> {
                Ok(($($ty::decode_bytes(_r)?,)*))
            }
        }
    }
}

impl_tuple!();
impl_tuple!(A);
impl_tuple!(A B);
impl_tuple!(A B C);
impl_tuple!(A B C D);
impl_tuple!(A B C D E);
impl_tuple!(A B C D E F);
impl_tuple!(A B C D E F G);
impl_tuple!(A B C D E F G H);
impl_tuple!(A B C D E F G H I);
impl_tuple!(A B C D E F G H I J);
impl_tuple!(A B C D E F G H I J K);
impl_tuple!(A B C D E F G H I J K L);
