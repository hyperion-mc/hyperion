use proc_macro::{TokenStream, TokenTree};

#[proc_macro]
pub fn for_each_static_handshake_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, STATIC_HANDSHAKE_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_lifetime_handshake_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, LIFETIME_HANDSHAKE_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_static_status_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, STATIC_STATUS_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_lifetime_status_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, LIFETIME_STATUS_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_static_login_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, STATIC_LOGIN_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_lifetime_login_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, LIFETIME_LOGIN_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_static_play_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, STATIC_PLAY_C2S_PACKETS)
}

#[proc_macro]
pub fn for_each_lifetime_play_c2s_packet(input: TokenStream) -> TokenStream {
    for_each_packet(input, LIFETIME_PLAY_C2S_PACKETS)
}

fn for_each_packet(input: TokenStream, packets: &[&str]) -> TokenStream {
    packets
        .iter()
        .flat_map(|packet| replace(input.clone(), packet))
        .collect()
}

fn replace(input: TokenStream, packet: &str) -> impl Iterator<Item = TokenTree> {
    input.into_iter().flat_map(|token| match token {
        TokenTree::Ident(ident) => {
            if format!("{ident}") == "PACKET" {
                let packet_ident = proc_macro2::Ident::new(packet, ident.span().into());
                let stream: proc_macro2::TokenStream =
                    syn::parse_quote!(::valence_protocol::packets::play::#packet_ident);
                TokenStream::from(stream).into_iter()
            } else {
                std::iter::once(TokenTree::Ident(ident))
                    .collect::<TokenStream>()
                    .into_iter()
            }
        }
        TokenTree::Group(group) => {
            // TODO: preserve group delimiter span
            let new_group = proc_macro::Group::new(
                group.delimiter(),
                replace(group.stream(), packet).collect(),
            );
            std::iter::once(TokenTree::Group(new_group))
                .collect::<TokenStream>()
                .into_iter()
        }
        token => std::iter::once(token).collect::<TokenStream>().into_iter(),
    })
}

static STATIC_HANDSHAKE_C2S_PACKETS: &[&str] = &[];

static LIFETIME_HANDSHAKE_C2S_PACKETS: &[&str] = &["HandshakeC2s"];

static STATIC_STATUS_C2S_PACKETS: &[&str] = &["QueryPingC2s", "QueryRequestC2s"];

static LIFETIME_STATUS_C2S_PACKETS: &[&str] = &[];

static STATIC_LOGIN_C2S_PACKETS: &[&str] = &["LoginQueryResponseC2s"];

static LIFETIME_LOGIN_C2S_PACKETS: &[&str] = &["LoginHelloC2s", "LoginKeyC2s"];

static STATIC_PLAY_C2S_PACKETS: &[&str] = &[
    "AdvancementTabC2s",
    "BoatPaddleStateC2s",
    "ButtonClickC2s",
    "ClientCommandC2s",
    "ClientStatusC2s",
    "CloseHandledScreenC2s",
    "CraftRequestC2s",
    "CreativeInventoryActionC2s",
    "CustomPayloadC2s",
    "FullC2s",
    "HandSwingC2s",
    "JigsawGeneratingC2s",
    "KeepAliveC2s",
    "LookAndOnGroundC2s",
    "MessageAcknowledgmentC2s",
    "OnGroundOnlyC2s",
    "PickFromInventoryC2s",
    "PlayPongC2s",
    "PlayerActionC2s",
    "PlayerInputC2s",
    "PlayerInteractBlockC2s",
    "PlayerInteractEntityC2s",
    "PlayerInteractItemC2s",
    "PositionAndOnGroundC2s",
    "QueryBlockNbtC2s",
    "QueryEntityNbtC2s",
    "RecipeBookDataC2s",
    "RecipeCategoryOptionsC2s",
    "ResourcePackStatusC2s",
    "SelectMerchantTradeC2s",
    "SpectatorTeleportC2s",
    "TeleportConfirmC2s",
    "UpdateBeaconC2s",
    "UpdateDifficultyC2s",
    "UpdateDifficultyLockC2s",
    "UpdatePlayerAbilitiesC2s",
    "UpdateSelectedSlotC2s",
    "VehicleMoveC2s",
];

static LIFETIME_PLAY_C2S_PACKETS: &[&str] = &[
    "BookUpdateC2s",
    "ChatMessageC2s",
    "ClickSlotC2s",
    "ClientSettingsC2s",
    "CommandExecutionC2s",
    "PlayerSessionC2s",
    "RenameItemC2s",
    "RequestCommandCompletionsC2s",
    "UpdateCommandBlockC2s",
    "UpdateCommandBlockMinecartC2s",
    "UpdateJigsawC2s",
    "UpdateSignC2s",
    "UpdateStructureBlockC2s",
];
