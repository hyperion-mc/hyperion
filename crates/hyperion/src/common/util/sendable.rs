use flecs_ecs::prelude::*;

pub struct SendableRef<'a>(pub WorldRef<'a>);

unsafe impl Send for SendableRef<'_> {}
unsafe impl Sync for SendableRef<'_> {}

pub struct SendableQuery<T>(pub Query<T>)
where
    T: QueryTuple;

unsafe impl<T: QueryTuple + Send> Send for SendableQuery<T> {}
unsafe impl<T: QueryTuple> Sync for SendableQuery<T> {}
