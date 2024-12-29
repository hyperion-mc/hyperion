use derive_more::{Constructor, Deref, DerefMut};
use flecs_ecs::{
    core::{EntityViewGet, World, WorldGet},
    macros::Component,
    prelude::Module,
};
use hyperion::storage::{EventFn, GlobalEventHandlers, InteractEvent};
use valence_protocol::nbt;

pub mod builder;

#[derive(Component)]
pub struct ItemModule;

#[derive(Component, Constructor, Deref, DerefMut)]
pub struct Handler {
    on_click: EventFn<InteractEvent>,
}

impl Module for ItemModule {
    fn module(world: &World) {
        world.import::<hyperion::simulation::inventory::InventoryModule>();
        world.component::<Handler>();

        world.get::<&mut GlobalEventHandlers>(|handlers| {
            handlers.interact.register(|query, event| {
                let world = query.world;
                let inventory = &mut *query.inventory;

                let stack = &inventory.get_cursor().stack;

                if stack.is_empty() {
                    return;
                }

                let Some(nbt) = stack.nbt.as_ref() else {
                    return;
                };

                let Some(handler) = nbt.get("Handler") else {
                    return;
                };

                let nbt::Value::Long(id) = handler else {
                    return;
                };

                let id: u64 = bytemuck::cast(*id);

                let handler = world.entity_from_id(id);

                handler.try_get::<&Handler>(|handler| {
                    let on_interact = &handler.on_click;
                    on_interact(query, event);
                });
            });
        });
    }
}
