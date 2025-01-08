use flecs_ecs::{core::Entity, macros::Component};
use hyperion_utils::Lifetime;
use valence_protocol::Hand;

use crate::simulation::handlers::PacketSwitchQuery;

pub type EventFn<T> = Box<dyn Fn(&mut PacketSwitchQuery<'_>, &T) + 'static + Send + Sync>;

pub struct CommandCompletionRequest<'a> {
    pub query: &'a str,
    pub id: i32,
}

unsafe impl Lifetime for CommandCompletionRequest<'_> {
    type WithLifetime<'a> = CommandCompletionRequest<'a>;
}

pub struct InteractEvent {
    pub hand: Hand,
    pub sequence: i32,
}

unsafe impl Lifetime for InteractEvent {
    type WithLifetime<'a> = Self;
}

// TODO: remove this
#[derive(Component, Default)]
pub struct GlobalEventHandlers {
    pub interact: EventHandlers<InteractEvent>,

    // todo: this should be a lifetime for<'a>
    pub completion: EventHandlers<CommandCompletionRequest<'static>>,
}

pub struct EventHandlers<T> {
    handlers: Vec<EventFn<T>>,
}

impl<T> Default for EventHandlers<T> {
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }
}

impl<T> EventHandlers<T> {
    pub fn trigger_all(&self, world: &mut PacketSwitchQuery<'_>, event: &T) {
        for handler in &self.handlers {
            handler(world, event);
        }
    }

    pub fn register2(
        &mut self,
        handler: impl Fn(&mut PacketSwitchQuery<'_>, &T) + 'static + Send + Sync,
    ) {
        self.handlers.push(Box::new(handler));
    }
}

pub struct PlayerJoinServer {
    pub username: String,
    pub entity: Entity,
}
