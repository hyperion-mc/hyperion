use crate::{Decode, DecodeBytesAuto, Encode, ItemKind, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct CooldownUpdateS2c {
    pub item_id: ItemKind,
    pub cooldown_ticks: VarInt,
}
