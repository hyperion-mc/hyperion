use flecs_ecs::prelude::*;

pub struct SendableRef<'a>(pub WorldRef<'a>);

unsafe impl Send for SendableRef<'_> {}
unsafe impl Sync for SendableRef<'_> {}
