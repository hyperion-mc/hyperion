use crate::{Decode, DecodeBytesAuto, Encode, ItemStack, Packet};

#[derive(Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct CreativeInventoryActionC2s {
    pub slot: i16,
    pub clicked_item: ItemStack,
}
