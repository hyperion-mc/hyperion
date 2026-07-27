use enumset::{EnumSet, EnumSetType};
use flecs_ecs::prelude::Component;
use hyperion_minecraft_proto::packets::play::entity::Animate;

/// The arm swings and effect flashes `ClientboundAnimatePacket` carries.
///
/// The numbers are the `ClientboundAnimatePacket` action constants, which have
/// not moved since 1.9.
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

    pub fn packets(&mut self, entity_id: i32) -> impl Iterator<Item = Animate> + use<> {
        self.kind.iter().map(move |kind| Animate {
            id: entity_id,
            #[expect(
                clippy::cast_possible_wrap,
                reason = "the action is a signed byte on the wire and every kind is under 128"
            )]
            action: kind as u8 as i8,
        })
    }

    pub fn push(&mut self, kind: Kind) {
        self.kind.insert(kind);
    }

    pub fn clear(&mut self) {
        self.kind.clear();
    }
}
