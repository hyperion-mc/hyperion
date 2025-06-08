use proc_macro::{TokenStream, TokenTree};
use quote::quote;

use crate::replace::*;

pub(crate) fn for_each_state(input: TokenStream) -> TokenStream {
    replace(input, STATES.iter().copied(), StateIdentReplacer)
}

#[derive(Copy, Clone)]
struct State {
    name: &'static str,
}

#[derive(Copy, Clone)]
struct StateIdentReplacer;

impl SpecialIdentReplacer<State> for StateIdentReplacer {
    fn replace(&self, ident: proc_macro::Ident, state: State) -> Option<TokenStream> {
        let ident_str = format!("{ident}");
        if ident_str == "for_each_packet" {
            let state_ident = proc_macro2::Ident::new(
                &format!("for_each_{}_c2s_packet", state.name),
                ident.span().into(),
            );
            Some(quote!(::hyperion_packet_macros::#state_ident).into())
        } else if ident_str == "state" {
            let state_ident = proc_macro::Ident::new(state.name, ident.span().into());
            Some(TokenStream::from(TokenTree::Ident(state_ident)))
        } else {
            None
        }
    }
}

const STATES: &[State] = &[
    State { name: "handshake" },
    State { name: "status" },
    State { name: "login" },
    State { name: "play" },
];
