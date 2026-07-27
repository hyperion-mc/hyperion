use flecs_ecs::core::Entity;
use hyperion_utils::Lifetime;
use valence_bytes::Utf8Bytes;
use valence_protocol::Hand;

use crate::simulation::handlers::PacketSwitchQuery;

pub type EventFn<T> = Box<dyn Fn(&mut PacketSwitchQuery<'_>, &T) + 'static + Send + Sync>;

pub struct CommandCompletionRequest {
    pub query: Utf8Bytes,
    pub id: i32,
}

unsafe impl Lifetime for CommandCompletionRequest {
    type WithLifetime<'a> = Self;
}

pub struct InteractEvent {
    pub hand: Hand,
    pub sequence: i32,
}

unsafe impl Lifetime for InteractEvent {
    type WithLifetime<'a> = Self;
}

pub struct PlayerJoinServer {
    pub username: String,
    pub entity: Entity,
}
