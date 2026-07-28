//! What a player is allowed to do with the world.
//!
//! Every event wants this, so it is a server capability rather than something
//! each game re-derives: smash needs adventure so an arena cannot be dug up,
//! bedwars needs survival, and both need spectator for a dead player. Before
//! this existed the only gamemode anything sent was a `GameEvent` written by
//! hand inside the smash adapter, which told the client one thing and left the
//! server believing another.
//!
//! The mode is a flecs enum, so it is on the entity as the relation
//! `(Gamemode, Gamemode.Adventure)` with an entity target, and it is exclusive
//! for free: setting a new mode removes the old edge rather than needing a
//! clear-then-set pair. That is also why the mode is read back with
//! [`of`] rather than with `try_get`.

use flecs_ecs::prelude::*;
use hyperion_minecraft_proto::packets::play_login::GameType;

/// A player's gamemode.
///
/// Ordered as `GameType` is so [`Self::to_game_type`] is a total mapping with
/// nothing to fall back on.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Hash, Default)]
#[repr(C)]
pub enum Gamemode {
    #[default]
    Survival,
    Creative,
    Adventure,
    Spectator,
}

impl Gamemode {
    /// The wire value the client reads in `Login`, `Respawn` and the tab list.
    #[must_use]
    pub const fn to_game_type(self) -> GameType {
        match self {
            Self::Survival => GameType::Survival,
            Self::Creative => GameType::Creative,
            Self::Adventure => GameType::Adventure,
            Self::Spectator => GameType::Spectator,
        }
    }

    /// Whether a player in this mode may break or place blocks.
    ///
    /// `Player.mayBuild()` is `abilities.mayBuild`, which `GameType` sets false
    /// for adventure and spectator. A vanilla client in adventure mode will not
    /// even send the dig packet, but a client is not what decides this: the
    /// handlers consult this before turning a packet into an event, so a
    /// scripted or patched client gets the same answer.
    #[must_use]
    pub const fn may_build(self) -> bool {
        matches!(self, Self::Survival | Self::Creative)
    }
}

/// The mode every player is put in at login.
///
/// A singleton and a plain component rather than a relation: it is one value
/// scoped to the whole server, not an edge between two entities. The per-player
/// mode, which is the thing a relation buys, is the [`Gamemode`] enum above.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq, Default)]
pub struct DefaultGamemode(pub Gamemode);

/// The mode `entity` is in, or [`Gamemode::Survival`] if it has none.
///
/// A flecs enum lives as a pair, so this reads the target of `(Gamemode, *)`
/// rather than a component value.
#[must_use]
pub fn of(entity: EntityView<'_>) -> Gamemode {
    entity
        .target(id::<Gamemode>(), 0)
        .map_or(Gamemode::Survival, |constant| {
            constant.to_constant::<Gamemode>()
        })
}

#[cfg(test)]
mod tests {
    use super::Gamemode;

    /// The one rule the block handlers ask about, spelled out so a reordering
    /// of the enum cannot quietly let an arena be dug up.
    #[test]
    fn only_survival_and_creative_may_build() {
        assert!(Gamemode::Survival.may_build());
        assert!(Gamemode::Creative.may_build());
        assert!(!Gamemode::Adventure.may_build());
        assert!(!Gamemode::Spectator.may_build());
    }

    /// `GameType.getId` is what the client reads, and the two enums are
    /// declared in the same order for exactly that reason.
    #[test]
    fn wire_ids_match_game_type() {
        assert_eq!(Gamemode::Survival.to_game_type().to_id(), 0);
        assert_eq!(Gamemode::Creative.to_game_type().to_id(), 1);
        assert_eq!(Gamemode::Adventure.to_game_type().to_id(), 2);
        assert_eq!(Gamemode::Spectator.to_game_type().to_id(), 3);
    }
}
