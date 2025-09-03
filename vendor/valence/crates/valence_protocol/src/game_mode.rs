use std::io::Write;

use anyhow::bail;
use bevy_ecs::prelude::*;
use derive_more::{From, Into};

use crate::{Decode, DecodeBytesAuto, Encode};

#[derive(
    Copy, Clone, PartialEq, Eq, Debug, Default, Encode, Decode, DecodeBytesAuto, Component,
)]
pub enum GameMode {
    #[default]
    Survival,
    Creative,
    Adventure,
    Spectator,
}

/// An optional [`GameMode`] with `None` encoded as `-1`. Isomorphic to
/// `Option<GameMode>`.
#[derive(Copy, Clone, PartialEq, Eq, Default, Debug, From, Into, DecodeBytesAuto)]
pub struct OptGameMode(pub Option<GameMode>);

impl Encode for OptGameMode {
    fn encode(&self, w: impl Write) -> anyhow::Result<()> {
        match self.0 {
            Some(gm) => (gm as i8).encode(w),
            None => (-1i8).encode(w),
        }
    }
}

impl Decode for OptGameMode {
    fn decode(r: &mut &[u8]) -> anyhow::Result<Self> {
        Ok(Self(match i8::decode(r)? {
            -1 => None,
            0 => Some(GameMode::Survival),
            1 => Some(GameMode::Creative),
            2 => Some(GameMode::Adventure),
            3 => Some(GameMode::Spectator),
            other => bail!("invalid game mode byte of {other}"),
        }))
    }
}
