use crate::{Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub enum ClientStatusC2s {
    PerformRespawn,
    RequestStats,
}
