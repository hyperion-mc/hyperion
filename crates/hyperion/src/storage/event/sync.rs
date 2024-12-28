use anyhow::Context;
use flecs_ecs::{core::Entity, macros::Component};
use valence_protocol::{
    Hand, ItemStack,
    packets::{
        play,
        play::click_slot_c2s::{ClickMode, SlotChange},
    },
};

use crate::simulation::handlers::PacketSwitchQuery;

pub type EventFn<T> = Box<dyn Fn(&mut PacketSwitchQuery<'_>, &T) + 'static + Send + Sync>;

pub struct CommandCompletionRequest<'a> {
    pub query: &'a str,
    pub id: i32,
}

pub struct InteractEvent {
    pub hand: Hand,
    pub sequence: i32,
}

#[derive(Debug)]
pub struct ClickSlotEvent {
    pub window_id: u8,
    pub state_id: i32,
    pub slot_idx: u16,
    /// The button used to click the slot. An enum can't easily be used for this
    /// because the meaning of this value depends on the mode.
    pub button: i8,
    pub mode: ClickMode,
    pub slot_changes: Vec<SlotChange>,
    pub carried_item: ItemStack,
}

impl TryFrom<play::ClickSlotC2s<'static>> for ClickSlotEvent {
    type Error = anyhow::Error;

    fn try_from(event: play::ClickSlotC2s<'static>) -> Result<Self, Self::Error> {
        let play::ClickSlotC2s {
            window_id,
            state_id,
            slot_idx,
            button,
            mode,
            slot_changes,
            carried_item,
        } = event;

        let slot_changes = slot_changes.into_owned();
        let slot_idx = u16::try_from(slot_idx).context("slot index is negative")?;

        Ok(Self {
            window_id,
            state_id: state_id.0,
            slot_idx,
            button,
            mode,
            slot_changes,
            carried_item,
        })
    }
}

#[derive(Component, Default)]
pub struct GlobalEventHandlers {
    pub click: EventHandlers<ClickSlotEvent>,
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

    pub fn register(
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
