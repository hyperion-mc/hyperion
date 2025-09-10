use crate::{Decode, DecodeBytesAuto, Encode, Packet, packet_id};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
#[packet(id = packet_id::BUNDLE_SPLITTER)]
pub struct BundleSplitterS2c;
