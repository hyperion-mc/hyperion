//! Who is standing near a point.
//!
//! "Every player within R of here, except the one who cast it" is the shape a
//! fighting game asks for more than any other, so it is one call.
//!
//! # Why this is a scan
//!
//! [`crate::spatial::SpatialIndex`] exists and holds a real BVH, but nothing
//! outside the tests and bedwars ever adds the [`Spatial`] component, so in a
//! running smash server the index is rebuilt empty every frame and answers
//! every query with nothing. Rather than half-populate it here, this walks the
//! player query.
//!
//! That is a fence, not a fossil. A scan over players costs one distance test
//! each, and Super Smash Mobs is a **sixteen-player** game; at that count the
//! whole query is under a microsecond and is called a few times per ability
//! cast, not per entity per tick. **Past roughly a thousand players in one
//! world it stops being free**, and the fix at that point is to populate the
//! index rather than to optimise the scan. ENG-10879 tracks the empty index.
//!
//! [`Spatial`]: crate::spatial::Spatial

use flecs_ecs::core::{
    Builder, Entity, EntityView, IdOperations, QueryAPI, QueryBuilderImpl, World, id,
};
use glam::Vec3;

use crate::simulation::{Player, Position};

/// A player found by an area query, and how far away they were.
#[derive(Debug, Clone, Copy)]
pub struct Hit<'a> {
    /// The player.
    pub entity: EntityView<'a>,
    /// Distance from the query's centre, in blocks.
    pub distance: f32,
    /// Where they were standing.
    pub position: Vec3,
}

impl Hit<'_> {
    /// How far in the player is, as a fraction of the radius: 1.0 at the
    /// centre and 0.0 at the edge.
    ///
    /// What an ability multiplies its damage by when it falls off with
    /// distance, which most of them do.
    #[must_use]
    pub fn falloff(&self, radius: f32) -> f32 {
        if radius <= 0.0 {
            return 0.0;
        }
        (1.0 - self.distance / radius).clamp(0.0, 1.0)
    }
}

/// Every player within `radius` of `center`, nearest first.
///
/// `except` is almost always the caster: an ability that hurts everyone nearby
/// means everyone else, and forgetting to exclude yourself is the bug this
/// parameter exists to make hard to write.
///
/// The distance is measured to the player's feet, which is where [`Position`]
/// is. An ability wanting to measure to the chest should add roughly 0.9 to
/// the centre's y rather than change this.
#[must_use]
pub fn players_within(
    world: &World,
    center: Vec3,
    radius: f32,
    except: Option<Entity>,
) -> Vec<Hit<'_>> {
    // Compared squared, so the common case of a player nowhere near the blast
    // costs no square root at all.
    let limit = radius * radius;
    let query = world.query::<&Position>().with(id::<Player>()).build();

    // Ids first, views second. The `EntityView` flecs hands the callback
    // borrows the callback's own frame, so one kept past the closure does not
    // compile; an `Entity` is a plain integer and re-resolves against the
    // world afterwards.
    let mut found: Vec<(Entity, Vec3, f32)> = Vec::new();
    query.each_entity(|entity, position| {
        if except == Some(entity.id()) {
            return;
        }
        let at = **position;
        let squared = (at - center).length_squared();
        if squared > limit {
            return;
        }
        found.push((entity.id(), at, squared.sqrt()));
    });

    // Nearest first, because an ability that only affects the closest few
    // wants to truncate rather than sort again. `total_cmp` rather than
    // `partial_cmp`: a NaN distance would silently make the sort inconsistent.
    found.sort_unstable_by(|a, b| a.2.total_cmp(&b.2));
    found
        .into_iter()
        .map(|(entity, position, distance)| Hit {
            entity: world.entity_from_id(entity),
            distance,
            position,
        })
        .collect()
}

/// The nearest player to a point, if there is one within `radius`.
#[must_use]
pub fn nearest_player(
    world: &World,
    center: Vec3,
    radius: f32,
    except: Option<Entity>,
) -> Option<Hit<'_>> {
    players_within(world, center, radius, except)
        .into_iter()
        .next()
}
