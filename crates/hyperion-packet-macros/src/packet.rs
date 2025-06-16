use proc_macro::{TokenStream, TokenTree};
use quote::quote;

use crate::replace::{SpecialIdentReplacer, replace};

pub fn for_each_packet(
    input: TokenStream,
    state: &'static str,
    packets: impl Iterator<Item = Packet> + Clone,
) -> TokenStream {
    replace(input, packets, PacketIdentReplacer { state })
}

#[derive(Copy, Clone)]
pub struct Packet {
    name: &'static str,
    lifetime: bool,
}

#[derive(Copy, Clone)]
struct PacketIdentReplacer {
    state: &'static str,
}

impl SpecialIdentReplacer<Packet> for PacketIdentReplacer {
    fn replace(&self, ident: proc_macro::Ident, packet: Packet) -> Option<TokenStream> {
        let ident_str = format!("{ident}");
        if ident_str == "valence_packet" {
            let packet_ident = proc_macro2::Ident::new(packet.name, ident.span().into());
            let state_ident = proc_macro2::Ident::new(self.state, ident.span().into());

            Some(quote!(::valence_protocol::packets::#state_ident::#packet_ident).into())
        } else if ident_str == "static_valence_packet" {
            let packet_ident = proc_macro2::Ident::new(packet.name, ident.span().into());
            let state_ident = proc_macro2::Ident::new(self.state, ident.span().into());

            if packet.lifetime {
                Some(
                    quote!(::valence_protocol::packets::#state_ident::#packet_ident<'static>)
                        .into(),
                )
            } else {
                Some(quote!(::valence_protocol::packets::#state_ident::#packet_ident).into())
            }
        } else if ident_str == "packet_name" {
            let packet_ident =
                proc_macro::Ident::new(&packet.name[..packet.name.len() - 3], ident.span());
            Some(TokenStream::from(TokenTree::Ident(packet_ident)))
        } else {
            None
        }
    }
}

pub static HANDSHAKE_C2S_PACKETS: &[Packet] = &[Packet {
    name: "HandshakeC2s",
    lifetime: true,
}];

pub static STATUS_C2S_PACKETS: &[Packet] = &[
    Packet {
        name: "QueryPingC2s",
        lifetime: false,
    },
    Packet {
        name: "QueryRequestC2s",
        lifetime: false,
    },
];

pub static LOGIN_C2S_PACKETS: &[Packet] = &[
    Packet {
        name: "LoginQueryResponseC2s",
        lifetime: false,
    },
    Packet {
        name: "LoginHelloC2s",
        lifetime: true,
    },
    Packet {
        name: "LoginKeyC2s",
        lifetime: true,
    },
];

pub static PLAY_C2S_PACKETS: &[Packet] = &[
    Packet {
        name: "AdvancementTabC2s",
        lifetime: false,
    },
    Packet {
        name: "BoatPaddleStateC2s",
        lifetime: false,
    },
    Packet {
        name: "ButtonClickC2s",
        lifetime: false,
    },
    Packet {
        name: "ClientCommandC2s",
        lifetime: false,
    },
    Packet {
        name: "ClientStatusC2s",
        lifetime: false,
    },
    Packet {
        name: "CloseHandledScreenC2s",
        lifetime: false,
    },
    Packet {
        name: "CraftRequestC2s",
        lifetime: false,
    },
    Packet {
        name: "CreativeInventoryActionC2s",
        lifetime: false,
    },
    Packet {
        name: "CustomPayloadC2s",
        lifetime: false,
    },
    Packet {
        name: "FullC2s",
        lifetime: false,
    },
    Packet {
        name: "HandSwingC2s",
        lifetime: false,
    },
    Packet {
        name: "JigsawGeneratingC2s",
        lifetime: false,
    },
    Packet {
        name: "KeepAliveC2s",
        lifetime: false,
    },
    Packet {
        name: "LookAndOnGroundC2s",
        lifetime: false,
    },
    Packet {
        name: "MessageAcknowledgmentC2s",
        lifetime: false,
    },
    Packet {
        name: "OnGroundOnlyC2s",
        lifetime: false,
    },
    Packet {
        name: "PickFromInventoryC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayPongC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayerActionC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayerInputC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayerInteractBlockC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayerInteractEntityC2s",
        lifetime: false,
    },
    Packet {
        name: "PlayerInteractItemC2s",
        lifetime: false,
    },
    Packet {
        name: "PositionAndOnGroundC2s",
        lifetime: false,
    },
    Packet {
        name: "QueryBlockNbtC2s",
        lifetime: false,
    },
    Packet {
        name: "QueryEntityNbtC2s",
        lifetime: false,
    },
    Packet {
        name: "RecipeBookDataC2s",
        lifetime: false,
    },
    Packet {
        name: "RecipeCategoryOptionsC2s",
        lifetime: false,
    },
    Packet {
        name: "ResourcePackStatusC2s",
        lifetime: false,
    },
    Packet {
        name: "SelectMerchantTradeC2s",
        lifetime: false,
    },
    Packet {
        name: "SpectatorTeleportC2s",
        lifetime: false,
    },
    Packet {
        name: "TeleportConfirmC2s",
        lifetime: false,
    },
    Packet {
        name: "UpdateBeaconC2s",
        lifetime: false,
    },
    Packet {
        name: "UpdateDifficultyC2s",
        lifetime: false,
    },
    Packet {
        name: "UpdateDifficultyLockC2s",
        lifetime: false,
    },
    Packet {
        name: "UpdatePlayerAbilitiesC2s",
        lifetime: false,
    },
    Packet {
        name: "UpdateSelectedSlotC2s",
        lifetime: false,
    },
    Packet {
        name: "VehicleMoveC2s",
        lifetime: false,
    },
    Packet {
        name: "BookUpdateC2s",
        lifetime: true,
    },
    Packet {
        name: "ChatMessageC2s",
        lifetime: true,
    },
    Packet {
        name: "ClickSlotC2s",
        lifetime: true,
    },
    Packet {
        name: "ClientSettingsC2s",
        lifetime: true,
    },
    Packet {
        name: "CommandExecutionC2s",
        lifetime: true,
    },
    Packet {
        name: "PlayerSessionC2s",
        lifetime: true,
    },
    Packet {
        name: "RenameItemC2s",
        lifetime: true,
    },
    Packet {
        name: "RequestCommandCompletionsC2s",
        lifetime: true,
    },
    Packet {
        name: "UpdateCommandBlockC2s",
        lifetime: true,
    },
    Packet {
        name: "UpdateCommandBlockMinecartC2s",
        lifetime: true,
    },
    Packet {
        name: "UpdateJigsawC2s",
        lifetime: true,
    },
    Packet {
        name: "UpdateSignC2s",
        lifetime: true,
    },
    Packet {
        name: "UpdateStructureBlockC2s",
        lifetime: true,
    },
];
