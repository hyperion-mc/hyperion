use proc_macro::TokenStream;

mod packet;
mod replace;
mod state;

// TODO: for_each is a somewhat misleading name

#[proc_macro]
pub fn for_each_handshake_c2s_packet(input: TokenStream) -> TokenStream {
    packet::for_each_packet(
        input,
        "handshaking",
        packet::HANDSHAKE_C2S_PACKETS.iter().copied(),
    )
}

#[proc_macro]
pub fn for_each_status_c2s_packet(input: TokenStream) -> TokenStream {
    packet::for_each_packet(input, "status", packet::STATUS_C2S_PACKETS.iter().copied())
}

#[proc_macro]
pub fn for_each_login_c2s_packet(input: TokenStream) -> TokenStream {
    packet::for_each_packet(input, "login", packet::LOGIN_C2S_PACKETS.iter().copied())
}

#[proc_macro]
pub fn for_each_play_c2s_packet(input: TokenStream) -> TokenStream {
    packet::for_each_packet(input, "play", packet::PLAY_C2S_PACKETS.iter().copied())
}

#[proc_macro]
pub fn for_each_state(input: TokenStream) -> TokenStream {
    state::for_each_state(input)
}
