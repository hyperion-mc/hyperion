use derive_more::Deref;
use flecs_ecs::macros::Component;

use crate::simulation::metadata::Metadata;

// todo: can be u8
#[derive(Component, PartialEq, Eq, Copy, Clone, Debug, Deref)]
#[meta]
pub struct EntityFlags {
    value: u8,
}

impl Default for EntityFlags {
    fn default() -> Self {
        Self::new()
    }
}

impl EntityFlags {
    pub const CROUCHING: Self = Self { value: 0x02 };
    pub const FLYING_WITH_ELYTRA: Self = Self { value: 0x80 };
    pub const GLOWING: Self = Self { value: 0x40 };
    pub const INVISIBLE: Self = Self { value: 0x20 };
    pub const ON_FIRE: Self = Self { value: 0x01 };
    // 0x04 skipped (previously riding)
    pub const SPRINTING: Self = Self { value: 0x08 };
    pub const SWIMMING: Self = Self { value: 0x10 };

    const fn new() -> Self {
        Self { value: 0 }
    }
}

impl std::ops::BitOrAssign for EntityFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.value |= rhs.value;
    }
}

impl std::ops::BitAndAssign for EntityFlags {
    fn bitand_assign(&mut self, rhs: Self) {
        self.value &= rhs.value;
    }
}

impl std::ops::BitOr for EntityFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self {
            value: self.value | rhs.value,
        }
    }
}

impl std::ops::BitAnd for EntityFlags {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self {
            value: self.value & rhs.value,
        }
    }
}

impl std::ops::Not for EntityFlags {
    type Output = Self;

    fn not(self) -> Self {
        Self { value: !self.value }
    }
}

impl Metadata for EntityFlags {
    type Type = u8;

    const INDEX: u8 = 0;

    fn to_type(self) -> Self::Type {
        self.value
    }
}
