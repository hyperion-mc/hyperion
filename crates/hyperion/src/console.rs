//! The engine's side of an operator console.
//!
//! A console needs to see chat, and chat is a single-consumer queue: whichever
//! system calls [`EventQueue::drain`](crate::storage::EventQueue::drain) first
//! takes every message, and the game is that system. A second reader is not
//! expressible, and making the queue multi-consumer to give one watcher a copy
//! would change how every event in the engine is delivered.
//!
//! So the tap is here, at the point the packet is decoded, before the queue.
//! What an observer sees is what the player typed, which is also the more
//! useful thing for an operator: the rendered line is a game's decision --
//! bedwars colours the name by team, smash does not -- and a game that refuses
//! a message on a cooldown still leaves the operator wanting to know somebody
//! tried.
//!
//! Nothing in the engine installs an observer. With none installed this costs
//! one singleton read and an empty iteration per chat packet.

use std::sync::Arc;

use flecs_ecs::prelude::*;

/// Something watching what players type.
///
/// Called from the packet handler, on the tick thread, before the message
/// reaches any game. An implementation must not block: a slow observer is a
/// slow tick.
pub trait ChatObserver: Send + Sync {
    /// `speaker`, whose username is `name`, said `message`.
    ///
    /// The name is passed rather than looked up because an observer is called
    /// from the packet path and has no world to look it up in. Resolving it
    /// there and handing over an entity id was the first shape of this and it
    /// was useless: a chat pane reading `1234: hello` names nobody.
    ///
    /// `message` is unsanitised on purpose. What is safe to put in a component
    /// is a question about a Minecraft client, and an observer that is not one
    /// -- a web page, a log, a bridge -- has its own answer.
    fn player_said(&self, speaker: Entity, name: &str, message: &str);
}

/// Everyone watching. A singleton, empty unless something registers.
#[derive(Component, Default)]
pub struct ChatObservers {
    observers: Vec<Arc<dyn ChatObserver>>,
}

impl ChatObservers {
    /// Start watching. There is no way to stop: an observer lives as long as
    /// the process, which is what every caller so far wants and is one less
    /// piece of state than a handle nobody would return.
    pub fn watch(&mut self, observer: Arc<dyn ChatObserver>) {
        self.observers.push(observer);
    }

    /// Tell everyone watching.
    pub fn player_said(&self, speaker: Entity, name: &str, message: &str) {
        for observer in &self.observers {
            observer.player_said(speaker, name, message);
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.observers.is_empty()
    }
}
