//! See [`AsyncRuntime`].

use std::sync::Arc;

use bevy::prelude::*;
use derive_more::{Deref, DerefMut};

/// Wrapper around [`tokio::runtime::Runtime`]
#[derive(Resource, Deref, DerefMut, Clone)]
pub struct AsyncRuntime {
    runtime: Arc<tokio::runtime::Runtime>,
}

impl AsyncRuntime {
    pub(crate) fn new() -> Self {
        Self {
            runtime: Arc::new(
                tokio::runtime::Builder::new_multi_thread()
                    // .worker_threads(2)
                    .enable_all()
                    // .thread_stack_size(1024 * 1024) // 1 MiB
                    .build()
                    .unwrap(),
            ),
        }
    }
}
