use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct SimulationDistanceS2c {
    pub simulation_distance: VarInt,
}
