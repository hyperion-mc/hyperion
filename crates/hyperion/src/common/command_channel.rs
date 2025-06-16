// The code in this file contains a modified implementation of CommandQueue from the Bevy project.
// It has been modified by Hyperion contributor(s).
// Warning: This notice may be required by section 4(b) of the Apache License. This is not legal advice.

use std::{
    mem::MaybeUninit,
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use bevy::{
    ecs::ptr::{OwningPtr, Unaligned},
    prelude::*,
};
use tracing::error;

struct CommandMeta {
    /// SAFETY: The `value` must point to a value of type `T: Command`,
    /// where `T` is some specific type that was used to produce this metadata.
    ///
    /// Advances `cursor` by the size of `T` in bytes.
    consume_command_and_get_size:
        unsafe fn(value: OwningPtr<'_, Unaligned>, world: NonNull<World>, cursor: &mut usize),
}

#[derive(Default, Clone)]
struct Inner {
    // This buffer densely stores all queued commands.
    //
    // For each command, one `CommandMeta` is stored, followed by zero or more bytes
    // to store the command itself. To interpret these bytes, a pointer must
    // be passed to the corresponding `CommandMeta.apply_command_and_get_size` fn pointer.
    //
    // This is implemented via a `Vec<MaybeUninit<u8>>` instead of a `Vec<Box<dyn Command>>` as an
    // optimization.
    pub(crate) bytes: Vec<MaybeUninit<u8>>,
    pub(crate) cursor: usize,
}

/// Densely and efficiently stores a multiple-producer single-consumer channel of heterogenous types implementing [`Command`].
#[derive(Resource, Default, Clone)]
pub struct CommandChannel {
    // TODO: Replace this Mutex with a lock-free alternative
    inner: Arc<Mutex<Inner>>,
}

impl CommandChannel {
    /// Push a [`Command`] onto the channel.
    #[inline]
    pub fn push<C: Command>(&self, command: C) {
        // Stores a command alongside its metadata.
        // `repr(C)` prevents the compiler from reordering the fields,
        // while `repr(packed)` prevents the compiler from inserting padding bytes.
        #[repr(C, packed)]
        struct Packed<C: Command> {
            meta: CommandMeta,
            command: C,
        }

        let mut inner = self.inner.lock().unwrap();

        let meta = CommandMeta {
            consume_command_and_get_size: |command, mut world, cursor| {
                *cursor += size_of::<C>();
                // SAFETY: According to the invariants of `CommandMeta.consume_command_and_get_size`,
                // `command` must point to a value of type `C`.
                let command: C = unsafe { command.read_unaligned() };
                // Apply command to the provided world
                // SAFETY: Caller ensures pointer is not null
                let world = unsafe { world.as_mut() };
                command.apply(world);
                // The command may have queued up world commands, which we flush here to ensure they are also picked up.
                // If the current command queue already the World Command queue, this will still behave appropriately because the global cursor
                // is still at the current `stop`, ensuring only the newly queued Commands will be applied.
                world.flush();
            },
        };

        let old_len = inner.bytes.len();

        // Reserve enough bytes for both the metadata and the command itself.
        inner.bytes.reserve(size_of::<Packed<C>>());

        // Pointer to the bytes at the end of the buffer.
        // SAFETY: We know it is within bounds of the allocation, due to the call to `.reserve()`.
        let ptr = unsafe { inner.bytes.as_mut_ptr().add(old_len) };
        // Write the metadata into the buffer, followed by the command.
        // We are using a packed struct to write them both as one operation.
        // SAFETY: `ptr` must be non-null, since it is within a non-null buffer.
        // The call to `reserve()` ensures that the buffer has enough space to fit a value of type `C`,
        // and it is valid to write any bit pattern since the underlying buffer is of type `MaybeUninit<u8>`.
        unsafe {
            ptr.cast::<Packed<C>>()
                .write_unaligned(Packed { meta, command });
        }

        // Extend the length of the buffer to include the data we just wrote.
        // SAFETY: The new length is guaranteed to fit in the vector's capacity,
        // due to the call to `.reserve()` above.
        unsafe {
            inner.bytes.set_len(old_len + size_of::<Packed<C>>());
        }
    }

    /// Execute the queued [`Command`]s in the world after applying any commands in the world's internal queue.
    /// This clears the channel.
    #[inline]
    pub fn apply(&self, world: &mut World) {
        world.flush();
        self.apply_or_drop_queued(world.into());
    }

    /// This will apply the queued [commands](`Command`).
    /// This clears the channel.
    #[inline]
    fn apply_or_drop_queued(&self, world: NonNull<World>) {
        let mut inner = self.inner.lock().unwrap();

        // SAFETY: If this is the command queue on world, world will not be dropped as we have a mutable reference
        // If this is not the command queue on world we have exclusive ownership and self will not be mutated
        let start = inner.cursor;
        let stop = inner.bytes.len();
        let mut local_cursor = start;

        // SAFETY: we are setting the global cursor to the current length to prevent the executing commands from applying
        // the remaining commands currently in this list. This is safe.
        inner.cursor = stop;

        while local_cursor < stop {
            // SAFETY: The cursor is either at the start of the buffer, or just after the previous command.
            // Since we know that the cursor is in bounds, it must point to the start of a new command.
            let meta = unsafe {
                inner
                    .bytes
                    .as_mut_ptr()
                    .add(local_cursor)
                    .cast::<CommandMeta>()
                    .read_unaligned()
            };

            // Advance to the bytes just after `meta`, which represent a type-erased command.
            local_cursor += size_of::<CommandMeta>();

            // Construct an owned pointer to the command.
            // SAFETY: It is safe to transfer ownership out of `inner.bytes`, since the increment of `cursor` above
            // guarantees that nothing stored in the buffer will get observed after this function ends.
            // `cmd` points to a valid address of a stored command, so it must be non-null.
            let cmd = unsafe {
                OwningPtr::<'_, Unaligned>::new(NonNull::new_unchecked(
                    inner.bytes.as_mut_ptr().add(local_cursor).cast(),
                ))
            };

            // SAFETY: The data underneath the cursor must correspond to the type erased in metadata,
            // since they were stored next to each other by `.push()`.
            // For ZSTs, the type doesn't matter as long as the pointer is non-null.
            // This also advances the cursor past the command. For ZSTs, the cursor will not move.
            // At this point, it will either point to the next `CommandMeta`,
            // or the cursor will be out of bounds and the loop will end.
            unsafe { (meta.consume_command_and_get_size)(cmd, world, &mut local_cursor) };
        }
        // Reset the buffer: all commands past the original `start` cursor have been applied.
        // SAFETY: we are setting the length of bytes to the original length, minus the length of the original
        // list of commands being considered. All bytes remaining in the Vec are still valid, unapplied commands.
        unsafe {
            inner.bytes.set_len(start);
            inner.cursor = start;
        };
    }
}

pub struct CommandChannelPlugin;

impl Plugin for CommandChannelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CommandChannel::default());
        app.add_systems(PreUpdate, sync_command_channel);
    }
}

fn sync_command_channel(world: &mut World) {
    let Some(channel) = world.get_resource::<CommandChannel>() else {
        error!("cannot sync CommandChannel because it is missing");
        return;
    };
    let channel = channel.clone();
    channel.apply(world);
}
