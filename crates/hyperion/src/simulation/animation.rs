use enumset::{EnumSet, EnumSetType};
use flecs_ecs::prelude::Component;
use valence_protocol::{VarInt, packets::play::EntityAnimationS2c};

#[derive(EnumSetType)]
#[repr(u8)]
pub enum Kind {
    SwingMainArm = 0,
    UseItem = 1,
    LeaveBed = 2,
    SwingOffHand = 3,
    Critical = 4,
    MagicCritical = 5,
}

#[derive(Component)]
pub struct ActiveAnimation {
    kind: EnumSet<Kind>,
}

impl ActiveAnimation {
    pub const NONE: Self = Self {
        kind: EnumSet::empty(),
    };

    pub fn packets(
        &mut self,
        entity_id: VarInt,
    ) -> impl Iterator<Item = EntityAnimationS2c> + use<> {
        self.kind.iter().map(move |kind| {
            let kind = kind as u8;
            EntityAnimationS2c {
                entity_id,
                animation: kind,
            }
        })
    }

    pub fn push(&mut self, kind: Kind) {
        self.kind.insert(kind);
    }

    pub fn clear(&mut self) {
        self.kind.clear();
    }
}
