use flecs_ecs::prelude::*;

/// A world stage handed to a rayon worker.
///
/// flecs makes `WorldRef` `!Send`, which is right for the world itself; a stage
/// is the exception, because flecs guarantees each stage is touched by exactly
/// one thread at a time and defers every mutation to that stage's own command
/// queue.
///
/// There is no equivalent for queries: use `Query::handle` and iterate the
/// resulting `QueryHandle` through the stage.
pub struct SendableRef<'a>(pub WorldRef<'a>);

unsafe impl Send for SendableRef<'_> {}
unsafe impl Sync for SendableRef<'_> {}
