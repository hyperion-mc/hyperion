use std::marker::PhantomData;

use flecs_ecs::{
    core::{ComponentId, ComponentType, DataComponent, Struct, World, WorldGet, flecs},
    macros::Component,
};

use crate::simulation::event;

pub mod event_queue;
pub mod raw;

pub use event_queue::EventQueue;
use hyperion_event_macros::define_events;

impl Events {
    pub fn push<E: Event>(&self, event: E, world: &World) {
        E::input(event, self, world);
    }
}

struct SendSyncPtr<T>(*const T, PhantomData<T>);

unsafe impl<T> Send for SendSyncPtr<T> {}
unsafe impl<T> Sync for SendSyncPtr<T> {}

mod sealed {
    pub trait Sealed {}
}

pub trait Event: ReducedLifetime + sealed::Sealed + Send + Sync + 'static {
    fn input(elem: Self, events: &Events, world: &World);
}

fn register_and_pointer<T: ComponentId + DataComponent + ComponentType<Struct>>(
    world: &World,
    elem: T,
) -> *const T {
    world.component::<T>().add_trait::<flecs::Sparse>();

    world.set(elem);

    world.get::<&T>(|x: &T| std::ptr::from_ref::<T>(x))
}

// Create the Events struct
define_events! {
    event::ItemInteract,
    event::SetSkin,
    event::AttackEntity,
    event::ChatMessage<'static>,
    event::Command<'static>,
    event::DestroyBlock,
    event::ItemDropEvent,
    event::PlaceBlock,
    event::PluginMessage<'static>,
    event::PostureUpdate,
    event::SwingArm,
    event::ToggleDoor,
    event::ReleaseUseItem,
    event::ClientStatusEvent,
    event::ProjectileEntityEvent,
    event::ProjectileBlockEvent,
}

pub trait ReducedLifetime {
    type Reduced<'a>
    where
        Self: 'a;

    fn reduce<'a>(self) -> Self::Reduced<'a>;
}
