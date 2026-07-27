use proc_macro::TokenStream;

mod lifetime;
mod packet;
mod replace;

// TODO: for_each is a somewhat misleading name

/// Repeats `input` once per play C2S packet, substituting the packet's name and
/// its valence type.
///
/// Only play. The states before it no longer go through valence at all: they
/// are decoded directly by `hyperion::net::protocol`, so a per-state macro over
/// valence's packet lists has nothing left to generate.
#[proc_macro]
pub fn for_each_play_c2s_packet(input: TokenStream) -> TokenStream {
    packet::for_each_packet(input, "play", packet::PLAY_C2S_PACKETS.iter().copied())
}

/// Repeats `input` once per play C2S packet that does not borrow, substituting `PACKET`.
#[proc_macro]
pub fn for_each_static_play_c2s_packet(input: TokenStream) -> TokenStream {
    lifetime::STATIC_PLAY_C2S_PACKETS
        .iter()
        .flat_map(|packet| lifetime::replace(input.clone(), packet))
        .collect()
}

/// Repeats `input` once per play C2S packet that borrows, substituting `PACKET`.
#[proc_macro]
pub fn for_each_lifetime_play_c2s_packet(input: TokenStream) -> TokenStream {
    lifetime::LIFETIME_PLAY_C2S_PACKETS
        .iter()
        .flat_map(|packet| lifetime::replace(input.clone(), packet))
        .collect()
}
