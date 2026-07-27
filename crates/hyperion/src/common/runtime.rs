//! See [`AsyncRuntime`].

use std::sync::Arc;

use derive_more::{Deref, DerefMut};
use flecs_ecs::{core::World, macros::Component};
use kanal::{Receiver, Sender};

/// Type alias for world callback functions
pub type WorldCallback = Box<dyn FnOnce(&World) + Send>;

/// Wrapper around [`tokio::runtime::Runtime`]
#[derive(Component, Deref, DerefMut, Clone)]
pub struct AsyncRuntime {
    #[deref]
    #[deref_mut]
    runtime: Arc<tokio::runtime::Runtime>,
    callback_sender: Sender<WorldCallback>,
}

#[derive(Component)]
pub struct Tasks {
    pub(crate) tasks: Receiver<WorldCallback>,
}

impl AsyncRuntime {
    pub fn schedule<T>(
        &self,
        future: impl Future<Output = T> + Send + 'static,
        handler: fn(T, &World),
    ) where
        T: Send + 'static,
    {
        let sender = self.callback_sender.clone();

        self.spawn(async move {
            let result = future.await;

            let to_send = move |world: &World| {
                handler(result, world);
            };

            sender.send(Box::new(to_send)).unwrap();
        });
    }

    pub(crate) fn new(sender: Sender<WorldCallback>) -> Self {
        Self {
            runtime: Arc::new(
                tokio::runtime::Builder::new_multi_thread()
                    // .worker_threads(2)
                    .enable_all()
                    // .thread_stack_size(1024 * 1024) // 1 MiB
                    .build()
                    .unwrap(),
            ),
            callback_sender: sender,
        }
    }
}
