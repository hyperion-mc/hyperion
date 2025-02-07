use std::ops::BitOr;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EntityStatus(pub u8);

impl EntityStatus {
    pub const HAS_GLOWING_EFFECT: Self = Self(0x40);
    pub const IS_CROUCHING: Self = Self(0x02);
    pub const IS_FLYING_WITH_ELYTRA: Self = Self(0x80);
    pub const IS_INVISIBLE: Self = Self(0x20);
    pub const IS_ON_FIRE: Self = Self(0x01);
    pub const IS_SPRINTING: Self = Self(0x08);
    pub const IS_SWIMMING: Self = Self(0x10);

    pub const fn has_status(self, status: Self) -> bool {
        self.0 & status.0 != 0
    }

    pub const fn set_status(&mut self, status: Self) {
        self.0 |= status.0;
    }

    pub const fn clear_status(&mut self, status: Self) {
        self.0 &= !status.0;
    }

    pub const fn toggle_status(&mut self, status: Self) {
        self.0 ^= status.0;
    }
}

impl BitOr for EntityStatus {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOr<EntityStatus> for &EntityStatus {
    type Output = EntityStatus;

    fn bitor(self, rhs: EntityStatus) -> Self::Output {
        EntityStatus(self.0 | rhs.0)
    }
}
