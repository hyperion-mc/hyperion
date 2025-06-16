use bevy::prelude::*;
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

pub struct PlayerJoinServer {
    pub username: String,
    pub entity: Entity,
}
