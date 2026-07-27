use std::io::Write;

use hyperion_minecraft_proto::{
    generated::packet_id::play::clientbound::PacketId, packets::play::clientbound::SystemChat,
    text::Component,
};

use crate::{PacketBundle, net::protocol::Clientbound};

/// A line of chat, ready to send to any number of players.
///
/// The text is kept as a string and turned into a component when it is
/// written, because [`crate::net::Compose`] encodes a packet once per
/// broadcast and the component borrows from this.
pub struct Chat {
    message: String,
}

/// A chat line carrying `chat` verbatim.
///
/// Section-sign codes inside the string still work: the client's own
/// `StringDecomposer` applies them when it renders a literal component, so the
/// legacy `§c` colours hyperion already writes need no translation.
pub fn chat(chat: impl Into<String>) -> Chat {
    Chat {
        message: chat.into(),
    }
}

#[macro_export]
macro_rules! chat {
    ($($arg:tt)*) => {
        $crate::net::agnostic::chat(format!($($arg)*))
    };
}

impl PacketBundle for &Chat {
    fn encode_including_ids(self, w: impl Write) -> anyhow::Result<()> {
        // `overlay` false puts the line in the chat box; true would make it an
        // action bar message, which `SetActionBarText` says more directly.
        let component = Component::text(self.message.as_str());
        let body = SystemChat {
            content: component.to_tag(),
            overlay: false,
        };
        Clientbound::new(PacketId::SystemChat.to_raw(), &body).encode_including_ids(w)
    }
}
