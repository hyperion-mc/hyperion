//! Fixes to the `flecs_ecs` Rust API, carried locally until they land upstream.
//!
//! Everything here is an extension trait over an upstream type, which is the
//! shape an upstream patch would take as an inherent method. Each one is
//! zero-cost: no allocation, no boxing, no extra indirection over what the
//! upstream call already does. `docs/flecs-rust-api-notes.md` has the full
//! writeup, including the changes considered and rejected.

use flecs_ecs::core::{
    ComponentId, Entity, EntityView, IdOperations, IntoEntity, WorldProvider, WorldRef,
};

/// Lifetime-correct constructors on [`WorldRef`].
///
/// Upstream's `WorldRef::entity_from_id` is declared
/// `fn entity_from_id(&self, id) -> EntityView<'_>`, so the returned view
/// borrows the `WorldRef` rather than the world. Since `WorldRef<'a>` is `Copy`
/// and is itself just a borrow of the world, that is strictly more restrictive
/// than it needs to be, and it makes the extremely ordinary
///
/// ```ignore
/// found.map(|id| player.world().entity_from_id(id))
/// ```
///
/// fail to compile with "returns a value referencing data owned by the current
/// function". Taking `self` by value instead threads `'a` through and the same
/// line compiles.
pub trait WorldRefExt<'a> {
    /// Like `entity_from_id`, but the view lives as long as the world does.
    fn entity_at(self, id: impl IntoEntity) -> EntityView<'a>;
}

impl<'a> WorldRefExt<'a> for WorldRef<'a> {
    #[inline]
    fn entity_at(self, id: impl IntoEntity) -> EntityView<'a> {
        EntityView::new_from(self, id)
    }
}

/// Relationship traversal that does not fight the borrow checker.
pub trait EntityViewExt<'a> {
    /// Like `each_target`, but the view handed to the callback is tied to the
    /// world rather than to the callback's own stack frame.
    ///
    /// Upstream's `each_target` gives the closure an `EntityView` it cannot
    /// return, store or compare outside the call, so every "find the target
    /// matching a predicate" turns into collecting bare `Entity` ids and
    /// re-resolving them afterwards. The target ids are plain integers read out
    /// of the entity's type; nothing about them is borrowed from the closure.
    fn each_target_view(self, relationship: impl IntoEntity, f: impl FnMut(EntityView<'a>));

    /// First target of `relationship` satisfying `predicate`.
    ///
    /// The common case of the above, and the one that motivated it: an ability
    /// lookup by hotbar slot runs on every right-click.
    fn find_target(
        self,
        relationship: impl IntoEntity,
        predicate: impl FnMut(EntityView<'a>) -> bool,
    ) -> Option<EntityView<'a>>;

    /// Emit `event` at this entity so that world observers querying component
    /// `Subject` match it.
    ///
    /// Upstream's `EntityView::emit` expands to
    /// `world.event().entity(self).emit(event)` with **no id set**, so an
    /// observer declared `world.observer::<MyEvent, &Health>()` never fires and
    /// nothing anywhere reports a problem. That is a silent no-op in the most
    /// natural spelling of the most common thing you want to do with a payload
    /// event, and it cost a full debugging cycle here before a probe found it.
    ///
    /// This spells the id out. See `docs/flecs-rust-api-notes.md` for the
    /// proposed upstream fix, which is to make `emit` default to the entity's
    /// own type rather than to nothing.
    fn emit_about<Subject: ComponentId, E: ComponentId>(self, event: &E);
}

impl<'a> EntityViewExt<'a> for EntityView<'a> {
    #[inline]
    fn each_target_view(self, relationship: impl IntoEntity, mut f: impl FnMut(EntityView<'a>)) {
        let world = self.world();
        self.each_target(relationship, |target| {
            f(world.entity_at(target.id()));
        });
    }

    #[inline]
    fn find_target(
        self,
        relationship: impl IntoEntity,
        mut predicate: impl FnMut(EntityView<'a>) -> bool,
    ) -> Option<EntityView<'a>> {
        let world = self.world();
        let mut found: Option<Entity> = None;
        self.each_target(relationship, |target| {
            if found.is_none() && predicate(world.entity_at(target.id())) {
                found = Some(target.id());
            }
        });
        found.map(|id| world.entity_at(id))
    }

    #[inline]
    fn emit_about<Subject: ComponentId, E: ComponentId>(self, event: &E) {
        self.world()
            .event()
            .add(Subject::id())
            .entity(self)
            .emit(event);
    }
}
