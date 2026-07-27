use std::{borrow::Cow, io::Write};

use uuid::Uuid;
use valence_protocol::{
    Decode, DecodeBytesAuto, Encode, ItemStack, Packet, VarInt,
    packets::play::{
        boss_bar_s2c::{BossBarColor, BossBarDivision, BossBarFlags},
        entity_equipment_update_s2c::EquipmentEntry,
    },
};

#[derive(Clone, PartialEq, Debug, Packet, DecodeBytesAuto)]
pub struct EntityEquipmentUpdateS2c<'a> {
    pub entity_id: VarInt,
    pub equipment: Cow<'a, [EquipmentEntry]>,
}

impl Encode for EntityEquipmentUpdateS2c<'_> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.entity_id.encode(&mut w)?;

        for i in 0..self.equipment.len() {
            let slot = self.equipment[i].slot;
            if i == self.equipment.len() - 1 {
                slot.encode(&mut w)?;
            } else {
                (slot | -128).encode(&mut w)?;
            }
            self.equipment[i].item.encode(&mut w)?;
        }

        Ok(())
    }
}

impl Decode for EntityEquipmentUpdateS2c<'static> {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        let entity_id = VarInt::decode(r)?;

        let mut equipment = vec![];

        loop {
            let slot = i8::decode(r)?;
            let item = ItemStack::decode(r)?;
            equipment.push(EquipmentEntry {
                slot: slot & 127,
                item,
            });
            if slot & -128 == 0 {
                break;
            }
        }

        Ok(Self {
            entity_id,
            equipment: Cow::Owned(equipment),
        })
    }
}

#[derive(Clone, Debug, Encode, Packet)]
pub struct BossBarS2c<'a> {
    pub id: Uuid,
    pub action: BossBarAction<'a>,
}

#[derive(Clone, PartialEq, Debug, Encode)]
pub enum BossBarAction<'a> {
    Add {
        title: hyperion_text::Text<'a>,
        health: f32,
        color: BossBarColor,
        division: BossBarDivision,
        flags: BossBarFlags,
    },
    Remove,
    UpdateHealth(f32),
    UpdateTitle(hyperion_text::Text<'a>),
    UpdateStyle(BossBarColor, BossBarDivision),
    UpdateFlags(BossBarFlags),
}
