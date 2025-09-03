use valence_math::Vec3;

use crate::{Decode, DecodeBytesAuto, Encode, Hand, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct PlayerInteractEntityC2s {
    pub entity_id: VarInt,
    pub interact: EntityInteraction,
    pub sneaking: bool,
}

#[derive(Copy, Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum EntityInteraction {
    Interact(Hand),
    Attack,
    InteractAt { target: Vec3, hand: Hand },
}
