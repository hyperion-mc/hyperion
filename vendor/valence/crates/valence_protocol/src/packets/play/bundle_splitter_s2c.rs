use crate::{packet_id, Decode, DecodeBytesAuto, Encode, Packet};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::BUNDLE_SPLITTER)]
pub struct BundleSplitterS2c;
