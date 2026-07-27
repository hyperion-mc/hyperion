use flecs_ecs::core::{ComponentId, EntityView, QueryTuple, SystemAPI, builder};
use tracing::Span;

pub trait TracingExt<'a, P, T>: SystemAPI<'a, P, T>
where
    T: QueryTuple,
    P: ComponentId,
{
    fn tracing_each_entity<Func>(
        &mut self,
        span: Span,
        func: Func,
    ) -> <Self as builder::Builder<'a>>::BuiltType
    where
        Func: FnMut(EntityView<'_>, T::TupleType<'_>) + 'static,
    {
        self.run_each_entity(
            move |mut iter| {
                let _enter = span.enter();

                while iter.next() {
                    iter.each();
                }
            },
            func,
        )
    }

    fn tracing_each<Func>(
        &mut self,
        span: Span,
        func: Func,
    ) -> <Self as builder::Builder<'a>>::BuiltType
    where
        Func: FnMut(T::TupleType<'_>) + 'static,
    {
        self.run_each(
            move |mut iter| {
                let _enter = span.enter();

                while iter.next() {
                    iter.each();
                }
            },
            func,
        )
    }
}

impl<'a, P, T, S> TracingExt<'a, P, T> for S
where
    S: SystemAPI<'a, P, T>,
    T: QueryTuple,
    P: ComponentId,
{
}
