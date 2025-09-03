use valence_ident::Ident;

use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub enum AdvancementTabC2s {
    OpenedTab { tab_id: Ident },
    ClosedScreen,
}
