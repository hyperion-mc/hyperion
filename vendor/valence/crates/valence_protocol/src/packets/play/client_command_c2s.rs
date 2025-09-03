use crate::{Decode, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Copy, Clone, Debug, Encode, Decode, DecodeBytesAuto, Packet)]
pub struct ClientCommandC2s {
    pub entity_id: VarInt,
    pub action: ClientCommand,
    pub jump_boost: VarInt,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum ClientCommand {
    StartSneaking,
    StopSneaking,
    LeaveBed,
    StartSprinting,
    StopSprinting,
    StartJumpWithHorse,
    StopJumpWithHorse,
    OpenHorseInventory,
    StartFlyingWithElytra,
}
