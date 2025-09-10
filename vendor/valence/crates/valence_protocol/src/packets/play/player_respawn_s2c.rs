use valence_ident::Ident;

use crate::game_mode::OptGameMode;
use crate::{DecodeBytes, Encode, GameMode, GlobalPos, Packet, VarInt};

#[derive(Clone, PartialEq, Debug, Encode, DecodeBytes, Packet)]
pub struct PlayerRespawnS2c {
    pub dimension_type_name: Ident,
    pub dimension_name: Ident,
    pub hashed_seed: u64,
    pub game_mode: GameMode,
    pub previous_game_mode: OptGameMode,
    pub is_debug: bool,
    pub is_flat: bool,
    pub copy_metadata: bool,
    pub last_death_location: Option<GlobalPos>,
    pub portal_cooldown: VarInt,
}
