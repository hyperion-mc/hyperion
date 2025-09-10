use std::borrow::Cow;
use std::collections::BTreeSet;

use valence_ident::Ident;
use valence_nbt::Compound;

use crate::game_mode::OptGameMode;
use crate::{DecodeBytes, Encode, GameMode, GlobalPos, Packet, VarInt};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct GameJoinS2c<'a> {
    pub entity_id: i32,
    pub is_hardcore: bool,
    pub game_mode: GameMode,
    pub previous_game_mode: OptGameMode,
    pub dimension_names: Cow<'a, BTreeSet<Ident>>,
    pub registry_codec: Cow<'a, Compound>,
    pub dimension_type_name: Ident,
    pub dimension_name: Ident,
    pub hashed_seed: i64,
    pub max_players: VarInt,
    pub view_distance: VarInt,
    pub simulation_distance: VarInt,
    pub reduced_debug_info: bool,
    pub enable_respawn_screen: bool,
    pub is_debug: bool,
    pub is_flat: bool,
    pub last_death_location: Option<GlobalPos>,
    pub portal_cooldown: VarInt,
}
