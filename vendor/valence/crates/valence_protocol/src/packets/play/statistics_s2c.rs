use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct StatisticsS2c {
    pub statistics: Vec<Statistic>,
}

#[derive(Copy, Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto)]
pub struct Statistic {
    pub category_id: VarInt,
    pub statistic_id: VarInt,
    pub value: VarInt,
}
