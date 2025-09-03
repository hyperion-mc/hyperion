use uuid::Uuid;
use valence_ident::Ident;

use crate::{Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct EntityAttributesS2c {
    pub entity_id: VarInt,
    pub properties: Vec<AttributeProperty>,
}

#[derive(Clone, PartialEq, Debug, Encode, DecodeBytes)]
pub struct AttributeProperty {
    pub key: Ident,
    pub value: f64,
    pub modifiers: Vec<AttributeModifier>,
}

#[derive(Clone, PartialEq, Debug, Encode, Decode, DecodeBytesAuto)]
pub struct AttributeModifier {
    pub uuid: Uuid,
    pub amount: f64,
    pub operation: u8,
}
