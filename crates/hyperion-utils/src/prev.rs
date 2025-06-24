use bevy::prelude::*;
use tracing::error;

fn initialize_previous<T: Component + Clone>(
    trigger: Trigger<'_, OnAdd, T>,
    query: Query<'_, '_, &T>,
    mut commands: Commands<'_, '_>,
) {
    let value = match query.get(trigger.target()) {
        Ok(value) => value,
        Err(e) => {
            error!("could not initialize previous: query failed: {e}");
            return;
        }
    };

    commands
        .entity(trigger.target())
        .insert(Prev(value.clone()));
}

fn update_previous<T: Component + Clone>(mut query: Query<'_, '_, (&mut Prev<T>, &T)>) {
    for (mut prev, current) in &mut query {
        prev.set(current.clone());
    }
}

pub fn track_prev<T: Component + Clone>(app: &mut App) {
    // TODO: There should be an error for calling this function for the same component twice
    app.add_observer(initialize_previous::<T>);
    app.add_systems(FixedPreUpdate, update_previous::<T>);
}

/// Component storing the value of a component in the previous frame. This is updated every
/// `FixedPreUpdate`.
#[derive(Component, Copy, Clone, Deref, PartialEq, Eq, Debug)]
pub struct Prev<T>(#[deref] T);

impl<T> Prev<T> {
    fn set(&mut self, new: T) {
        self.0 = new;
    }
}
