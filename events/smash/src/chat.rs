//! Player chat.
//!
//! hyperion decodes a chat packet into [`event::ChatMessage`] and stops there:
//! nothing in the engine broadcasts it, because what a chat line looks like is
//! a game's decision. Until this file existed smash made no decision, so every
//! message a player typed was decoded, queued, and dropped when the queue was
//! recycled. Two clients could stand next to each other and neither could say
//! anything to the other.
//!
//! The line is vanilla's: `<Name> message`, undecorated. Vanilla's
//! `chat.type.text` translation is `<%s> %s` with no style on either half, and
//! a fighting game has no teams or ranks to colour a name by, so there is
//! nothing here that vanilla would have coloured.
//!
//! # Why this is host-side and not under `src/module/`
//!
//! It reads a hyperion event queue. Everything under `src/module/` is the game
//! half and compiles against the [`crate::server`] seam alone, which is what
//! lets `tests/` run the whole game with no Minecraft server behind it. The
//! translation from a hyperion event to a game action is [`crate::input`]'s
//! job and this is one of those, kept in its own file only so that the chat
//! decision is somewhere a person can find it.

use flecs_ecs::prelude::*;
use hyperion::{
    simulation::{Name, chat::strip_formatting, event},
    storage::EventQueue,
};

use crate::server::{Channel, ServerHandle, Text};

/// The chat line for one message, exactly as a client draws it.
///
/// Split out from the system so the formatting is testable without a world:
/// the system's job is to find the speaker's name, and this is the whole of
/// what the game decides.
///
/// [`strip_formatting`] is not optional. The client renders a literal string
/// through its legacy formatter, so a section sign a player typed is a
/// formatting instruction and not a character; see that function for what it
/// buys.
#[must_use]
pub fn line(speaker: &str, message: &str) -> Text {
    Text::text(format!("<{speaker}> {}", strip_formatting(message)))
}

/// Broadcasting what players type. Behavior only; the components it reads are
/// registered by the modules it imports.
#[derive(Component)]
pub struct SmashChatModule;

impl Module for SmashChatModule {
    fn module(world: &World) {
        // `ServerHandle` and every game component below it, and with them the
        // `hyperion::simulation` registrations that hyperion's own import
        // brings in. A behavior module imports the registration for everything
        // it touches even when a parent already did: flecs dedupes the import
        // and a missing one is a dev-profile abort.
        world.import::<crate::adapter::SmashAdapterModule>();

        world
            .system_named::<&mut EventQueue<event::ChatMessage>>("smash_chat")
            .each_iter(|it, _index, queue| {
                let world = it.world();

                for event::ChatMessage { msg, by } in queue.drain() {
                    let speaker = world.entity_from_id(by);

                    // A player who disconnected or died between the packet
                    // arriving and this tick draining the queue. Their message
                    // is still theirs, but there is no name to attribute it to.
                    if !speaker.is_alive() {
                        continue;
                    }

                    // Whitespace only. The client will not send an empty
                    // string, but it will happily send a space.
                    let msg = msg.as_str();
                    if msg.trim().is_empty() {
                        continue;
                    }

                    let Some(name) = speaker.try_get::<&Name>(ToString::to_string) else {
                        // Every entity that can send a chat packet is a player
                        // and hyperion names a player during login, so this is
                        // a hole in that rather than a message to drop
                        // quietly.
                        tracing::warn!("dropping a chat message from an unnamed entity {by:?}");
                        continue;
                    };

                    world.get::<&ServerHandle>(|server| {
                        server.broadcast(Channel::Chat, line(&name, msg));
                    });
                }
            });
    }
}

#[cfg(test)]
mod tests {
    use super::line;

    #[test]
    fn renders_the_vanilla_shape() {
        assert_eq!(line("Andrew", "hello").plain(), "<Andrew> hello");
    }

    #[test]
    fn a_section_sign_in_the_message_is_dropped() {
        // Without this the client draws "kbold" scrambled and obfuscated, and
        // the second line below reads as a server notice rather than as somebody
        // talking.
        assert_eq!(line("Andrew", "\u{a7}kbold").plain(), "<Andrew> kbold");
        assert_eq!(
            line("Andrew", "\u{a7}4[Server] restarting").plain(),
            "<Andrew> 4[Server] restarting"
        );
    }

    #[test]
    fn an_ordinary_message_is_untouched() {
        assert_eq!(
            line("Andrew", "gg <3 100% !").plain(),
            "<Andrew> gg <3 100% !"
        );
    }
}
