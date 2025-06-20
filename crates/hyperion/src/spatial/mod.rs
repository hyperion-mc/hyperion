use bevy::prelude::*;
use geometry::{aabb::Aabb, ray::Ray};
use ordered_float::NotNan;
use rayon::iter::Either;

use super::{
    glam::Vec3,
    simulation::{
        EntitySize, Position, aabb,
        blocks::{Blocks, RayCollision},
    },
};

pub struct SpatialPlugin;

#[derive(Resource, Debug, Default)]
pub struct SpatialIndex {
    /// The bounding boxes of all entities with the [`Spatial`] component
    query: bvh_region::Bvh<Entity>,
}

#[must_use]
pub fn get_first_collision(
    ray: Ray,
    index: &SpatialIndex,
    blocks: &Blocks,
    query: Query<'_, '_, (&Position, &EntitySize)>,
    owner: Option<Entity>,
) -> Option<Either<Entity, RayCollision>> {
    // Check for collisions with entities
    let entity = index.first_ray_collision(ray, query);
    let block = blocks.first_collision(ray);

    // make sure the entity is not the owner
    let entity = entity.filter(|(entity, _)| owner.is_none_or(|owner| *entity != owner));

    // check which one is closest to the Ray don't forget to account for entity size
    entity.map_or(block.map(Either::Right), |(entity, _)| {
        let entity_aabb = get_aabb_func(query)(&entity);

        #[allow(clippy::redundant_closure_for_method_calls)]
        let distance_to_entity = entity_aabb
            .intersect_ray(&ray)
            .map_or(f32::MAX, |distance| distance.into_inner());

        block.map_or(Some(Either::Left(entity)), |block_collision| {
            if distance_to_entity < block_collision.distance {
                Some(Either::Left(entity))
            } else {
                Some(Either::Right(block_collision))
            }
        })
    })
}

fn get_aabb_func(
    query: Query<'_, '_, (&Position, &EntitySize)>,
) -> impl Fn(&Entity) -> Aabb + Send + Sync {
    move |entity: &Entity| {
        let (position, size) = query
            .get(*entity)
            .expect("spatial index must contain alive entities");
        aabb(**position, *size)
    }
}

impl SpatialIndex {
    pub fn get_collisions<'a>(
        &'a self,
        target: Aabb,
        query: Query<'a, 'a, (&Position, &EntitySize)>,
    ) -> impl Iterator<Item = Entity> + 'a {
        let get_aabb = get_aabb_func(query);
        self.query.range(target, get_aabb).copied()
    }

    /// Get the closest player to the given position.
    #[must_use]
    pub fn closest_to(
        &self,
        point: Vec3,
        query: Query<'_, '_, (&Position, &EntitySize)>,
    ) -> Option<Entity> {
        let get_aabb = get_aabb_func(query);
        Some(*self.query.get_closest(point, &get_aabb)?.0)
    }

    #[must_use]
    pub fn first_ray_collision<'a>(
        &self,
        ray: Ray,
        query: Query<'a, 'a, (&Position, &EntitySize)>,
    ) -> Option<(Entity, NotNan<f32>)> {
        let get_aabb = get_aabb_func(query);
        let (entity, distance) = self.query.first_ray_collision(ray, get_aabb)?;
        Some((*entity, distance))
    }
}

fn recalculate_spatial_index(
    mut index: ResMut<'_, SpatialIndex>,
    entity_query: Query<'_, '_, Entity, (With<Position>, With<EntitySize>, With<Spatial>)>,
    component_query: Query<'_, '_, (&Position, &EntitySize)>,
) {
    // todo(perf): re-use allocations?
    let all_entities = entity_query.iter().collect();
    let get_aabb = get_aabb_func(component_query);

    index.query = bvh_region::Bvh::build(all_entities, &get_aabb);
}

/// If we want the entity to be spatially indexed, we need to add this component.
#[derive(Component)]
pub struct Spatial;

impl Plugin for SpatialPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(SpatialIndex::default());
        app.add_systems(FixedPreUpdate, recalculate_spatial_index);
    }
}
