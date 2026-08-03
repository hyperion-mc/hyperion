//! The console's end of a connection that has no socket.
//!
//! Every command in the workspace answers its caller by reading the caller's
//! [`ConnectionId`] and unicasting to it. So the console has one, and this is
//! what is on the other side of it: frames arrive exactly as the proxy would
//! have received them, go through the same decoder a client uses, and any
//! `SystemChat` among them becomes a line on the page.
//!
//! Anything that is not chat is dropped. A command that answers by moving a
//! boss bar or opening an inventory has said something the console cannot
//! draw, and inventing a rendering for it would be worse than being quiet.

use std::sync::Mutex;

use hyperion::{
    hyperion_minecraft_proto::{
        framing::FrameDecoder, generated::packet_id::play::clientbound::PacketId,
        packets::play::clientbound::SystemChat, text::Component,
    },
    net::{ConnectionId, VirtualConnection},
    valence_protocol::CompressionThreshold,
};

use crate::feed::{Feed, Source};

/// The stream id the console answers to.
///
/// Far above anything the proxy hands out --- ids come from a counter that
/// starts at zero and one server would need to outlive the heat death of
/// several suns to reach this --- so a real player can never be given it.
pub const CONSOLE_STREAM: u64 = u64::MAX - 1;

/// The proxy the console pretends to be behind.
///
/// Never used to route anything: the interception in
/// [`hyperion::net::IoBuf::unicast_raw`] happens before a proxy is chosen. It
/// exists because a [`ConnectionId`] has two halves and both are compared.
pub const CONSOLE_PROXY: u64 = u64::MAX - 1;

/// Turns frames addressed to the console into lines on the page.
pub struct ConsoleConnection {
    stream: ConnectionId,
    /// The decoder carries state across frames --- a packet can be split over
    /// several writes --- so it is one decoder for the life of the console
    /// rather than one per frame.
    decoder: Mutex<FrameDecoder>,
    feed: &'static Feed,
}

impl ConsoleConnection {
    /// A connection delivering into `feed`.
    ///
    /// Takes the server's own [`CompressionThreshold`] rather than a
    /// `Option<usize>` a caller worked out, because working it out is the one
    /// step here that can be silently wrong. See [`decoder_threshold`].
    #[must_use]
    pub fn new(feed: &'static Feed, compression_threshold: CompressionThreshold) -> Self {
        let mut decoder = FrameDecoder::new();
        decoder.set_compression_threshold(decoder_threshold(compression_threshold));

        Self {
            stream: ConnectionId::new(CONSOLE_STREAM, hyperion::net::ProxyId::new(CONSOLE_PROXY)),
            decoder: Mutex::new(decoder),
            feed,
        }
    }
}

/// What to tell a [`FrameDecoder`] given the threshold the encoder holds.
///
/// The two are not the same type and the mapping is not obvious.
/// `PacketEncoder::append_packet` switches on `threshold.0 >= 0`, so a
/// negative threshold means no compression at all and every other value --
/// including zero -- means every frame carries a `data_len` varint, whether or
/// not that particular frame was deflated. `usize::try_from` is exactly that
/// predicate: `None` for the negative case, `Some` for the rest.
///
/// This is a named function with tests rather than an expression at the call
/// site because getting it wrong is invisible. A decoder told there is no
/// compression reads the `data_len` varint as the packet id, decides the frame
/// is not a `SystemChat`, and drops it -- no error, no log line, an operator
/// console that shows a command being sent and never shows a reply. That state
/// was reproduced deliberately against a live server: every command reply
/// vanished and the server said nothing at any log level.
fn decoder_threshold(threshold: CompressionThreshold) -> Option<usize> {
    usize::try_from(threshold.0).ok()
}

impl VirtualConnection for ConsoleConnection {
    fn stream(&self) -> ConnectionId {
        self.stream
    }

    fn deliver(&self, frame: &[u8]) {
        let Ok(mut decoder) = self.decoder.lock() else {
            return;
        };

        decoder.queue(frame);

        loop {
            match decoder.next_packet() {
                Ok(Some(packet)) => {
                    if packet.id != PacketId::SystemChat.to_raw() {
                        continue;
                    }
                    match packet.decode_body::<SystemChat<'_>>() {
                        // The page renders section signs, so the text goes on
                        // with its own formatting intact.
                        Ok(chat) => match Component::from_tag(&chat.content) {
                            // `legacy::render` and not `plain`: a component
                            // carries its colour as a field, the page renders
                            // section signs, and `plain` throws the field away
                            // -- which turns a red refusal into a sentence the
                            // same colour as everything else.
                            Ok(component) => {
                                self.feed
                                    .push(Source::Reply, crate::legacy::render(&component));
                            }
                            Err(error) => {
                                tracing::warn!("console could not read a reply component: {error}");
                            }
                        },
                        Err(error) => {
                            tracing::warn!("console could not decode a chat reply: {error}");
                        }
                    }
                }
                Ok(None) => return,
                // The stream position is no longer known, so every later frame
                // would be read at the wrong offset. Saying so once and
                // stopping beats a page filling with garbage.
                Err(error) => {
                    tracing::error!("console reply stream is unreadable, giving up: {error}");
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use hyperion::valence_protocol::CompressionThreshold;

    use super::decoder_threshold;

    /// The server's own default, from `crates/hyperion/src/lib.rs`. Frames
    /// carry a `data_len`, so the decoder has to expect one.
    #[test]
    fn the_shipping_threshold_turns_compression_on() {
        assert_eq!(decoder_threshold(CompressionThreshold(256)), Some(256));
    }

    /// `PacketEncoder::append_packet` treats `>= 0` as compression enabled, so
    /// zero is on -- every frame framed with a `data_len`, and every one of
    /// them deflated, since the encoder compresses when `data_len > 0`.
    /// Reading zero as "off" is the plausible mistake this pins.
    #[test]
    fn zero_is_compression_on_and_not_off() {
        assert_eq!(decoder_threshold(CompressionThreshold(0)), Some(0));
    }

    /// The only value that means off.
    #[test]
    fn a_negative_threshold_is_the_one_that_disables_it() {
        assert_eq!(decoder_threshold(CompressionThreshold(-1)), None);
    }
}
