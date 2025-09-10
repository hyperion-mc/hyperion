use std::borrow::Cow;
use std::io::Write;

use uuid::Uuid;
use valence_bytes::{Bytes, CowFixedBytes, CowUtf8Bytes};
use valence_text::Text;

use crate::{Bounded, Decode, DecodeBytes, DecodeBytesAuto, Encode, Packet, VarInt};

#[derive(Clone, PartialEq, Debug, Packet)]
pub struct ChatMessageS2c<'a> {
    pub sender: Uuid,
    pub index: VarInt,
    pub message_signature: Option<CowFixedBytes<'a, 256>>,
    pub message: Bounded<CowUtf8Bytes<'a>, 256>,
    pub timestamp: u64,
    pub salt: u64,
    pub previous_messages: Vec<MessageSignature<'a>>,
    pub unsigned_content: Option<Cow<'a, Text>>,
    pub filter_type: MessageFilterType,
    pub filter_type_bits: Option<u8>,
    pub chat_type: VarInt,
    pub network_name: Cow<'a, Text>,
    pub network_target_name: Option<Cow<'a, Text>>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Encode, Decode, DecodeBytesAuto)]
pub enum MessageFilterType {
    PassThrough,
    FullyFiltered,
    PartiallyFiltered,
}

impl<'a> Encode for ChatMessageS2c<'a> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        self.sender.encode(&mut w)?;
        self.index.encode(&mut w)?;
        self.message_signature.encode(&mut w)?;
        self.message.encode(&mut w)?;
        self.timestamp.encode(&mut w)?;
        self.salt.encode(&mut w)?;
        self.previous_messages.encode(&mut w)?;
        self.unsigned_content.encode(&mut w)?;
        self.filter_type.encode(&mut w)?;

        if self.filter_type == MessageFilterType::PartiallyFiltered {
            match self.filter_type_bits {
                // Filler data
                None => 0u8.encode(&mut w)?,
                Some(bits) => bits.encode(&mut w)?,
            }
        }

        self.chat_type.encode(&mut w)?;
        self.network_name.encode(&mut w)?;
        self.network_target_name.encode(&mut w)?;

        Ok(())
    }
}

impl<'a> DecodeBytes for ChatMessageS2c<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let sender = Uuid::decode_bytes(r)?;
        let index = VarInt::decode_bytes(r)?;
        let message_signature = Option::<CowFixedBytes<'a, 256>>::decode_bytes(r)?;
        let message = DecodeBytes::decode_bytes(r)?;
        let time_stamp = u64::decode_bytes(r)?;
        let salt = u64::decode_bytes(r)?;
        let previous_messages = Vec::<MessageSignature>::decode_bytes(r)?;
        let unsigned_content = Option::<Cow<'a, Text>>::decode_bytes(r)?;
        let filter_type = MessageFilterType::decode_bytes(r)?;

        let filter_type_bits = match filter_type {
            MessageFilterType::PartiallyFiltered => Some(u8::decode_bytes(r)?),
            _ => None,
        };

        let chat_type = VarInt::decode_bytes(r)?;
        let network_name = <Cow<'a, Text>>::decode_bytes(r)?;
        let network_target_name = Option::<Cow<'a, Text>>::decode_bytes(r)?;

        Ok(Self {
            sender,
            index,
            message_signature,
            message,
            timestamp: time_stamp,
            salt,
            previous_messages,
            unsigned_content,
            filter_type,
            filter_type_bits,
            chat_type,
            network_name,
            network_target_name,
        })
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct MessageSignature<'a> {
    pub message_id: i32,
    pub signature: Option<CowFixedBytes<'a, 256>>,
}

impl Encode for MessageSignature<'_> {
    fn encode(&self, mut w: impl Write) -> anyhow::Result<()> {
        VarInt(self.message_id + 1).encode(&mut w)?;

        match &self.signature {
            None => {}
            Some(signature) => signature.encode(&mut w)?,
        }

        Ok(())
    }
}

impl<'a> DecodeBytes for MessageSignature<'a> {
    fn decode_bytes(r: &mut Bytes) -> anyhow::Result<Self> {
        let message_id = VarInt::decode_bytes(r)?.0 - 1; // TODO: this can underflow.

        let signature = if message_id == -1 {
            Some(DecodeBytes::decode_bytes(r)?)
        } else {
            None
        };

        Ok(Self {
            message_id,
            signature,
        })
    }
}
