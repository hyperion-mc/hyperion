use super::chat_message_s2c::MessageSignature;
use crate::{DecodeBytes, Encode, Packet};

#[derive(Clone, Debug, Encode, DecodeBytes, Packet)]
pub struct RemoveMessageS2c<'a> {
    pub signature: MessageSignature<'a>,
}
