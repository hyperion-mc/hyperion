//! Provides functionality for ordering systems and observers in a dependency graph.
//! 
//! This module calculates execution order for systems and observers based on their dependencies,
//! assigning each a numeric order value that respects the dependency relationships.

use std::collections::{BTreeSet, HashMap, HashSet};

use derive_more::Constructor;
use flecs_ecs::{
    core::{
        Builder, Entity, EntityView, EntityViewGet, IdOperations, QueryAPI, QueryBuilderImpl,
        SystemAPI, flecs, flecs::DependsOn,
    },
    macros::Component,
    prelude::{Module, World},
};

/// Represents an ordering key composed of a depth in the dependency graph and an entity ID.
/// Used to sort systems by their dependency depth first, and then by ID for stable ordering.
#[derive(PartialOrd, Ord, PartialEq, Eq, Debug)]
struct OrderKey {
    depth: usize,
    id: Entity,
}

/// Calculates dependency depths for entities in the system graph.
#[derive(Default)]
struct DepthCalculator {
    depths: HashMap<Entity, usize, rustc_hash::FxBuildHasher>,
}

impl DepthCalculator {
    /// Calculates the dependency depth for a given entity view.
    /// The depth is the longest path of dependencies leading to this entity.
    fn calculate_depth(&mut self, view: EntityView<'_>) -> usize {
        if let Some(depth) = self.depths.get(&view.id()) {
            return *depth;
        }

        // todo: add stackoverflow check
        let mut entity_depth = 0;

        view.each_target::<DependsOn>(|depends_on| {
            let tentative_depth = self.calculate_depth(depends_on) + 1;
            entity_depth = entity_depth.max(tentative_depth);
        });

        self.depths.insert(view.id(), entity_depth);

        entity_depth
    }

    /// Calculates the depth for post-update observers.
    fn on_update_depth(&mut self, world: &World) -> usize {
        let view = world
            .component_id::<flecs::pipeline::PostUpdate>()
            .entity_view(world);

        self.calculate_depth(view)
    }
}

/// Represents the execution order of a system or observer.
/// 
/// The order is calculated based on the system's position in the dependency graph,
/// ensuring that dependencies are executed before dependent systems.
#[derive(
    Component,
    Constructor,
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord
)]
#[must_use]
#[meta]
pub struct SystemOrder {
    /// The numeric order value, where lower values execute earlier
    value: u16,
}

impl SystemOrder {
    /// Returns the raw order value
    #[must_use]
    pub const fn value(&self) -> u16 {
        self.value
    }

    /// Gets the [`SystemOrder`] component from an entity
    pub fn of(entity: EntityView<'_>) -> Self {
        entity.get::<&Self>(|order| *order)
    }
}

/// Calculates and assigns order values to all systems and observers
fn calculate(world: &World) {
    let mut depth_calculator = DepthCalculator::default();

    let mut map = BTreeSet::new();

    // get all depths for systems
    world
        .query::<()>()
        .with::<flecs::system::System>()
        .build()
        .each_entity(|entity, ()| {
            let depth = depth_calculator.calculate_depth(entity);

            map.insert(OrderKey {
                depth,
                id: entity.id(),
            });
        });

    // handle all observers
    world
        .query::<()>()
        .with::<flecs::Observer>()
        .build()
        .each_entity(|entity, ()| {
            let depth = depth_calculator.on_update_depth(world);

            map.insert(OrderKey {
                depth,
                id: entity.id(),
            });
        });

    // assert all entities are unique
    assert_eq!(
        map.len(),
        map.iter().map(|x| x.id).collect::<HashSet<_>>().len()
    );

    for (idx, value) in map.into_iter().enumerate() {
        let idx = u16::try_from(idx).expect("number of systems exceeds u16 (65536)");

        let entity = value.id.entity_view(world);

        entity.set(SystemOrder::new(idx));
    }
}

/// Module that sets up system ordering functionality.
/// 
/// Registers the [`SystemOrder`] component and creates observers to calculate
/// order values whenever systems or observers are added.
#[derive(Component)]
pub struct SystemOrderModule;

impl Module for SystemOrderModule {
    fn module(world: &World) {
        world.component::<SystemOrder>().meta();

        world
            .observer::<flecs::OnAdd, ()>()
            .with::<flecs::system::System>()
            .run(|it| calculate(&it.world()));

        world
            .observer::<flecs::OnAdd, ()>()
            .with::<flecs::Observer>()
            .run(|it| calculate(&it.world()));
    }
}
