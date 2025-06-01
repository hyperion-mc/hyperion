use bevy::prelude::*;
use valence_protocol::packets;

// Unfortunately macros cannot expand to enum variants, so hyperion_packet_macros cannot be used

#[derive(Event)]
pub enum HandshakePacket {
    Handshake(packets::handshaking::HandshakeC2s<'static>),
}

#[derive(Event)]
pub enum StatusPacket {
    QueryPing(packets::status::QueryPingC2s),
    QueryRequest(packets::status::QueryRequestC2s),
}

#[derive(Event)]
pub enum LoginPacket {
    LoginHello(packets::login::LoginHelloC2s<'static>),
    LoginKey(packets::login::LoginKeyC2s<'static>),
    LoginQueryResponse(packets::login::LoginQueryResponseC2s),
}

#[derive(Event)]
pub enum PlayPacket {
    AdvancementTab(packets::play::AdvancementTabC2s),
    BoatPaddleState(packets::play::BoatPaddleStateC2s),
    BookUpdate(packets::play::BookUpdateC2s<'static>),
    ButtonClick(packets::play::ButtonClickC2s),
    ChatMessage(packets::play::ChatMessageC2s<'static>),
    ClickSlot(packets::play::ClickSlotC2s<'static>),
    ClientCommand(packets::play::ClientCommandC2s),
    ClientSettings(packets::play::ClientSettingsC2s<'static>),
    ClientStatus(packets::play::ClientStatusC2s),
    CloseHandledScreen(packets::play::CloseHandledScreenC2s),
    CommandExecution(packets::play::CommandExecutionC2s<'static>),
    CraftRequest(packets::play::CraftRequestC2s),
    CreativeInventoryAction(packets::play::CreativeInventoryActionC2s),
    CustomPayload(packets::play::CustomPayloadC2s),
    Full(packets::play::FullC2s),
    HandSwing(packets::play::HandSwingC2s),
    JigsawGenerating(packets::play::JigsawGeneratingC2s),
    KeepAlive(packets::play::KeepAliveC2s),
    LookAndOnGround(packets::play::LookAndOnGroundC2s),
    MessageAcknowledgment(packets::play::MessageAcknowledgmentC2s),
    OnGroundOnly(packets::play::OnGroundOnlyC2s),
    PickFromInventory(packets::play::PickFromInventoryC2s),
    PlayPong(packets::play::PlayPongC2s),
    PlayerAction(packets::play::PlayerActionC2s),
    PlayerInput(packets::play::PlayerInputC2s),
    PlayerInteractBlock(packets::play::PlayerInteractBlockC2s),
    PlayerInteractEntity(packets::play::PlayerInteractEntityC2s),
    PlayerInteractItem(packets::play::PlayerInteractItemC2s),
    PlayerSession(packets::play::PlayerSessionC2s<'static>),
    PositionAndOnGround(packets::play::PositionAndOnGroundC2s),
    QueryBlockNbt(packets::play::QueryBlockNbtC2s),
    QueryEntityNbt(packets::play::QueryEntityNbtC2s),
    RecipeBookData(packets::play::RecipeBookDataC2s),
    RecipeCategoryOptions(packets::play::RecipeCategoryOptionsC2s),
    RenameItem(packets::play::RenameItemC2s<'static>),
    RequestCommandCompletions(packets::play::RequestCommandCompletionsC2s<'static>),
    ResourcePackStatus(packets::play::ResourcePackStatusC2s),
    SelectMerchantTrade(packets::play::SelectMerchantTradeC2s),
    SpectatorTeleport(packets::play::SpectatorTeleportC2s),
    TeleportConfirm(packets::play::TeleportConfirmC2s),
    UpdateBeacon(packets::play::UpdateBeaconC2s),
    UpdateCommandBlock(packets::play::UpdateCommandBlockC2s<'static>),
    UpdateCommandBlockMinecart(packets::play::UpdateCommandBlockMinecartC2s<'static>),
    UpdateDifficulty(packets::play::UpdateDifficultyC2s),
    UpdateDifficultyLock(packets::play::UpdateDifficultyLockC2s),
    UpdateJigsaw(packets::play::UpdateJigsawC2s<'static>),
    UpdatePlayerAbilities(packets::play::UpdatePlayerAbilitiesC2s),
    UpdateSelectedSlot(packets::play::UpdateSelectedSlotC2s),
    UpdateSign(packets::play::UpdateSignC2s<'static>),
    UpdateStructureBlock(packets::play::UpdateStructureBlockC2s<'static>),
    VehicleMove(packets::play::VehicleMoveC2s),
}
