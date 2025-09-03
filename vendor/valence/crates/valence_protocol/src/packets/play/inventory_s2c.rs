use std::borrow::Cow;

use crate::{Decode, DecodeBytesAuto, Encode, ItemStack, Packet, VarInt};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct InventoryS2c<'a> {
    pub window_id: u8,
    pub state_id: VarInt,
    pub slots: Cow<'a, [ItemStack]>,
    pub carried_item: Cow<'a, ItemStack>,
}
