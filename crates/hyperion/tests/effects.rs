#![allow(
    clippy::float_cmp,
    reason = "these compare values that were carried, not computed: a field that passed through a \
              packet unchanged is exactly equal or the packet is wrong, and a tolerance here \
              would pass for a field that was mangled"
)]

//! What an ability draws, pushes and hits.
//!
//! The particle bodies themselves are checked against Mojang's own encoder in
//! `hyperion-minecraft-proto`'s `play_particles`. What is checked here is the
//! layer above: that a shape puts its points where it says, that the count,
//! offset and speed a caller asked for reach the packet, that a knockback
//! survives the wire's quantisation, and that a radius includes what is inside
//! it and excludes what is not.

use flecs_ecs::core::{ComponentId, Entity, EntityViewGet, IdOperations, World, WorldProvider};
use glam::Vec3;
use hyperion::{
    HyperionCore,
    effects::{
        Effect, Shape,
        area::players_within,
        motion::{knockback_impulse, quantized},
        shape::sample,
        spawn::{Lifetime, facing, launch, spawn},
        status::Status,
    },
    simulation::{Owner, Pitch, Player, Position, Uuid, Velocity, Yaw, entity_kind::EntityKind},
};
use hyperion_minecraft_proto::{
    Decode, Encode, Reader, Writer,
    generated::registry::MobEffect,
    item::nbt::Scanner,
    packets::play::{chunk::LevelParticles, entity::SetEntityMotion},
    particle::{Argb, Particle},
    types::Vec3 as ProtoVec3,
};

/// A world with the engine in it, and no clients.
///
/// Nothing here sends: [`Effect::packets`] is what the send path serialises, so
/// asserting on it asserts on the bytes without needing a proxy on a socket.
fn world() -> World {
    let world = World::new();
    world.import::<HyperionCore>();
    world
}

fn player(world: &World, at: Vec3) -> Entity {
    world
        .entity()
        .add_enum(EntityKind::Player)
        .add(Player::id())
        .set(Position::new(at.x, at.y, at.z))
        .set(Velocity::default())
        .id()
}

fn ids(hits: &[hyperion::effects::Hit<'_>]) -> Vec<Entity> {
    hits.iter().map(|hit| hit.entity.id()).collect()
}

fn encoded(value: &impl Encode) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

// --- particles -------------------------------------------------------------

/// A burst is one packet at one point, carrying exactly the count, offset and
/// speed asked for, and the particle's own extra data untouched.
#[test]
fn a_burst_sends_one_packet_with_the_count_offset_and_extra_data_asked_for() {
    let effect = Effect::burst(
        Particle::Dust {
            color: Argb::opaque(0xff, 0x00, 0x00),
            scale: 2.0,
        },
        Vec3::new(1.5, 64.0, -2.25),
    )
    .count(40)
    .offset(Vec3::new(0.5, 0.25, 0.125))
    .speed(0.75)
    .long_distance(true);

    let packets: Vec<LevelParticles<'_>> = effect.packets().collect();
    assert_eq!(packets.len(), 1, "a burst is one point");
    let packet = &packets[0];

    assert_eq!(packet.count, 40);
    assert_eq!(
        (packet.x_dist, packet.y_dist, packet.z_dist),
        (0.5, 0.25, 0.125)
    );
    assert_eq!(packet.max_speed, 0.75);
    assert!(packet.override_limiter, "long_distance sets it");
    assert!(!packet.always_show, "the player's own setting is theirs");
    assert_eq!((packet.x, packet.y, packet.z), (1.5, 64.0, -2.25));

    // And the extra data survives to the bytes, which is the part a struct
    // comparison would not catch if the particle were re-derived on the way.
    let bytes = encoded(packet);
    let mut reader = Reader::new(&bytes);
    let decoded = LevelParticles::decode(&mut reader, &Scanner).expect("decode");
    assert_eq!(decoded, *packet);
    assert_eq!(decoded.particle, Particle::Dust {
        color: Argb::opaque(0xff, 0x00, 0x00),
        scale: 2.0,
    });
}

/// A line reaches both of its ends and spaces the rest evenly between them.
#[test]
fn a_line_hits_both_ends_and_spaces_the_rest_evenly() {
    let from = Vec3::new(0.0, 0.0, 0.0);
    let to = Vec3::new(3.0, 0.0, 4.0);
    let points = sample(Shape::Line { to }, from, Some(6));

    assert_eq!(points.len(), 6);
    assert_eq!(points[0], from);
    assert!(
        (points[5] - to).length() < 1e-5,
        "the last point is the far end, was {:?}",
        points[5]
    );

    // Five gaps over a five-block span.
    for pair in points.windows(2) {
        let gap = pair[0].distance(pair[1]);
        assert!((gap - 1.0).abs() < 1e-4, "uneven gap {gap}");
    }
}

/// Every point of a ring is at the radius, in the plane the normal names, and
/// the ring does not collapse when the normal is the axis a naive basis would
/// have picked.
#[test]
fn a_ring_stays_at_its_radius_in_the_plane_it_was_given() {
    for normal in [Vec3::Y, Vec3::X, Vec3::Z, Vec3::new(1.0, 1.0, 0.0)] {
        let center = Vec3::new(10.0, 64.0, -5.0);
        let points = sample(
            Shape::Ring {
                radius: 3.0,
                normal,
            },
            center,
            Some(16),
        );
        assert_eq!(points.len(), 16);

        let unit = normal.normalize();
        for point in &points {
            let offset = *point - center;
            assert!(
                (offset.length() - 3.0).abs() < 1e-4,
                "off the radius by {} with normal {normal:?}",
                offset.length() - 3.0
            );
            assert!(
                offset.dot(unit).abs() < 1e-4,
                "out of the plane by {} with normal {normal:?}",
                offset.dot(unit)
            );
        }

        // A collapsed ring has every point on one line through the centre.
        let spread = points[0].distance(points[4]);
        assert!(spread > 1.0, "the ring collapsed with normal {normal:?}");
    }
}

/// A sphere's points are all on its surface and are not all bunched at a pole,
/// which is what a naive uniform-angle sampling gives.
#[test]
fn a_sphere_covers_its_surface_rather_than_its_poles() {
    let center = Vec3::new(0.0, 64.0, 0.0);
    let points = sample(Shape::Sphere { radius: 5.0 }, center, Some(64));
    assert_eq!(points.len(), 64);

    for point in &points {
        let radius = (*point - center).length();
        assert!((radius - 5.0).abs() < 1e-3, "off the surface by {radius}");
    }

    // Both hemispheres carry roughly half, which a pole-bunched sampling fails.
    let above = points.iter().filter(|point| point.y > center.y).count();
    assert!(
        (28..=36).contains(&above),
        "{above} of 64 above the equator"
    );
}

/// A disc fills its area rather than its edge, and stays inside the radius.
#[test]
fn a_disc_fills_its_area() {
    let center = Vec3::ZERO;
    let points = sample(
        Shape::Disc {
            radius: 4.0,
            normal: Vec3::Y,
        },
        center,
        Some(64),
    );

    let mut inner = 0;
    for point in &points {
        let radius = point.length();
        assert!(radius <= 4.0 + 1e-4, "outside the disc at {radius}");
        assert!(point.y.abs() < 1e-4, "off the plane");
        if radius < 2.0 {
            inner += 1;
        }
    }
    // A quarter of the area is inside half the radius, so a filled disc puts
    // roughly a quarter of its points there. A ring would put none.
    assert!(
        (10..=22).contains(&inner),
        "{inner} of 64 in the inner half"
    );
}

/// A ring is one broadcast of many packets, not many broadcasts, and every one
/// of them carries the same particle.
#[test]
fn a_ring_sends_one_packet_per_point() {
    let effect = Effect::ring(Particle::Flame, Vec3::new(0.0, 64.0, 0.0), 2.0)
        .points(24)
        .count(2);
    let packets: Vec<LevelParticles<'_>> = effect.packets().collect();
    assert_eq!(packets.len(), 24);
    for packet in &packets {
        assert_eq!(packet.particle, Particle::Flame);
        assert_eq!(packet.count, 2);
    }
}

/// An expanding effect starts at the shape's radius and ends at the one it was
/// given, rather than jumping to the end on the first tick.
#[test]
fn an_expanding_effect_sweeps_from_its_radius_to_its_target() {
    let mut emitter = Effect::sphere(Particle::Flame, Vec3::ZERO, 1.0).expanding(5.0, 1.0);

    assert_eq!(emitter.current().shape().radius(), Some(1.0));
    emitter.elapsed = 0.5;
    assert_eq!(emitter.current().shape().radius(), Some(3.0));
    emitter.elapsed = 1.0;
    assert_eq!(emitter.current().shape().radius(), Some(5.0));
    // Past the end it clamps rather than overshooting, which matters because
    // the system draws once more on the tick it decides to stop.
    emitter.elapsed = 2.0;
    assert_eq!(emitter.current().shape().radius(), Some(5.0));
}

// --- motion ----------------------------------------------------------------

/// A knockback pushes away from the origin, horizontally, with the lift on top.
#[test]
fn a_knockback_pushes_away_horizontally_with_the_lift_on_top() {
    let origin = Vec3::new(0.0, 64.0, 0.0);
    // Directly east of the origin, and eight blocks above it.
    let target = Vec3::new(4.0, 72.0, 0.0);
    let impulse = knockback_impulse(origin, target, 0.8, 0.4);

    assert!((impulse.x - 0.8).abs() < 1e-6, "{impulse:?}");
    assert!(impulse.z.abs() < 1e-6, "{impulse:?}");
    // The vertical separation does not steer it: standing on someone's head
    // does not mean being launched straight up.
    assert!((impulse.y - 0.4).abs() < 1e-6, "{impulse:?}");
}

/// Two things at the same spot have no direction between them, and the answer
/// is up rather than a NaN that would put the victim at the world origin.
#[test]
fn a_knockback_from_where_you_are_standing_is_straight_up() {
    let at = Vec3::new(1.0, 64.0, 2.0);
    let impulse = knockback_impulse(at, at, 0.8, 0.4);
    assert!(impulse.is_finite());
    assert_eq!(impulse, Vec3::new(0.0, 0.4, 0.0));
}

/// The velocity a knockback asks for is the velocity the client is told, to
/// within the wire's own rounding, and `quantized` names that rounding.
///
/// This is the assertion the fixed-point requirement really wants on protocol
/// 776: there is no 1/8000 conversion to check, there is a shared-exponent
/// quantisation, and what matters is that the packet carries what was asked
/// for rather than something a scale factor mangled.
#[test]
fn a_knockback_reaches_the_wire_as_the_velocity_it_asked_for() {
    for requested in [
        Vec3::new(0.8, 0.4, 0.0),
        Vec3::new(-1.25, 0.5, 2.0),
        Vec3::new(0.001, -0.002, 0.0005),
        Vec3::new(12.0, -3.25, 0.125),
    ] {
        let packet = SetEntityMotion {
            id: 0x2A,
            movement: ProtoVec3 {
                x: f64::from(requested.x),
                y: f64::from(requested.y),
                z: f64::from(requested.z),
            },
        };
        let bytes = encoded(&packet);
        let mut reader = Reader::new(&bytes);
        let decoded = SetEntityMotion::decode(&mut reader).expect("decode");

        let expected = quantized(requested);
        // Narrowing is exact here: the value went in as an `f32`, and the
        // codec only rounds it to a coarser grid.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the value went in as an f32 and the codec only coarsens it"
        )]
        let got = Vec3::new(
            decoded.movement.x as f32,
            decoded.movement.y as f32,
            decoded.movement.z as f32,
        );

        // `quantized` predicts exactly what the wire does, so this is equality
        // and not a tolerance. A tolerance here would pass for a `quantized`
        // that was merely close.
        assert_eq!(got, expected, "{requested:?} did not survive the wire");

        // And the prediction is close to what was asked for. The step grows
        // with the largest component, which is why this is relative.
        let scale = requested.abs().max_element().max(1.0);
        assert!(
            (expected - requested).abs().max_element() < scale * 1e-3,
            "{requested:?} became {expected:?}"
        );
    }
}

/// A projectile points where it is going, in Minecraft's own angle convention:
/// yaw zero faces south, and pitch is negative looking up.
#[test]
fn a_launched_projectile_faces_along_its_velocity() {
    let (yaw, pitch) = facing(Vec3::new(0.0, 0.0, 1.0));
    assert!(yaw.abs() < 1e-4, "south is yaw zero, got {yaw}");
    assert!(pitch.abs() < 1e-4, "level is pitch zero, got {pitch}");

    let (yaw, _) = facing(Vec3::new(-1.0, 0.0, 0.0));
    assert!((yaw - 90.0).abs() < 1e-4, "west is yaw 90, got {yaw}");

    let (_, pitch) = facing(Vec3::new(0.0, 1.0, 0.0));
    assert!((pitch + 90.0).abs() < 1e-4, "up is pitch -90, got {pitch}");
}

// --- area ------------------------------------------------------------------

/// Every case that needs a world in one test, because `HyperionCore` builds
/// the global rayon pool and a second world in the same process fails to
/// import. The area scenarios sit a hundred blocks apart so that each query
/// sees only its own players.
#[test]
fn area_queries() {
    let world = world();
    includes_the_edge_and_excludes_just_past_it(&world);
    is_a_sphere_and_not_a_box(&world);
    excludes_the_caster_and_nobody_else(&world);
    answers_nearest_first(&world);
    a_spawned_entity_carries_what_the_spawn_observer_reads(&world);
    a_lifetime_expires_the_entity_it_is_on(&world);
}

/// The five components `add_entity` is built from, all present.
///
/// The `Spawn` observer reads them and logs rather than failing when one is
/// missing, so a spawn helper that forgot one would produce nothing visible
/// and no test failure. This is what notices.
fn a_spawned_entity_carries_what_the_spawn_observer_reads(world: &World) {
    let at = Vec3::new(400.0, 64.0, 0.0);
    let aim = Vec3::new(0.0, 0.0, -2.0);
    let shooter = player(world, at);
    let arrow = launch(world.world(), EntityKind::Arrow, at, aim, shooter);

    assert!(arrow.try_get::<&Uuid>(|_| ()).is_some(), "no uuid");
    assert!(arrow.try_get::<&Pitch>(|_| ()).is_some(), "no pitch");
    assert_eq!(
        arrow.try_get::<&Position>(|position| **position),
        Some(at),
        "wrong position"
    );
    assert_eq!(
        arrow.try_get::<&Velocity>(|velocity| velocity.0),
        Some(aim),
        "the launch velocity did not reach the entity"
    );
    assert_eq!(
        arrow.try_get::<&Owner>(|owner| owner.entity),
        Some(shooter),
        "an arrow with no owner collides with the bow that fired it"
    );
    // Facing north, which is -z and yaw 180.
    let yaw = arrow.try_get::<&Yaw>(|yaw| **yaw).expect("no yaw");
    assert!((yaw.abs() - 180.0).abs() < 1e-4, "yaw {yaw}");
}

/// A temporary entity goes away on its own, and not before its time.
///
/// The despawn half is what stops it: nothing outside `PacketState::Play` used
/// to send `RemoveEntities`, so a turret that expired stayed drawn on every
/// client for the rest of the match.
fn a_lifetime_expires_the_entity_it_is_on(world: &World) {
    let at = Vec3::new(500.0, 64.0, 0.0);
    let turret = spawn(world.world(), EntityKind::SnowGolem, at).set(Lifetime::new(0.5));
    let id = turret.id();

    world.progress_time(0.2);
    assert!(
        world.entity_from_id(id).is_alive(),
        "expired before its time"
    );

    world.progress_time(0.4);
    assert!(
        !world.entity_from_id(id).is_alive(),
        "outlived its lifetime"
    );
}

/// The boundary, from both sides: a player just inside the radius is hit and
/// one just outside is not.
fn includes_the_edge_and_excludes_just_past_it(world: &World) {
    const RADIUS: f32 = 5.0;
    const EPSILON: f32 = 0.01;
    let center = Vec3::new(0.0, 64.0, 0.0);

    let inside = player(world, center + Vec3::new(RADIUS - EPSILON, 0.0, 0.0));
    let outside = player(world, center + Vec3::new(RADIUS + EPSILON, 0.0, 0.0));

    let found = ids(&players_within(world, center, RADIUS, None));

    assert!(found.contains(&inside), "a player at R minus epsilon is in");
    assert!(
        !found.contains(&outside),
        "a player at R plus epsilon is out"
    );
}

/// The radius is a sphere, not a cube: a player at the corner of the bounding
/// box is further away than the radius and is not hit.
fn is_a_sphere_and_not_a_box(world: &World) {
    let center = Vec3::new(100.0, 64.0, 0.0);
    // Offset 4, 4, 4 is inside a 5-block box and 6.93 blocks from the centre.
    let corner = player(world, center + Vec3::splat(4.0));
    // A control at the same distance along one axis, which is inside, so the
    // assertion below is measuring the shape and not an empty world.
    let straight = player(world, center + Vec3::new(4.0, 0.0, 0.0));

    let found = ids(&players_within(world, center, 5.0, None));
    assert!(found.contains(&straight), "four blocks east is inside");
    assert!(
        !found.contains(&corner),
        "the corner of the box is outside the sphere"
    );
}

/// The caster is excluded when asked for, and only the caster.
fn excludes_the_caster_and_nobody_else(world: &World) {
    let center = Vec3::new(200.0, 64.0, 0.0);
    let caster = player(world, center);
    let other = player(world, center + Vec3::new(1.0, 0.0, 0.0));

    assert_eq!(
        ids(&players_within(world, center, 5.0, Some(caster))),
        vec![other]
    );

    // Without the exclusion both are there, so the assertion above is
    // measuring the exclusion rather than a query that found nothing.
    assert_eq!(players_within(world, center, 5.0, None).len(), 2);
}

/// Nearest first, because an ability that only affects the closest few
/// truncates rather than sorting again.
fn answers_nearest_first(world: &World) {
    let center = Vec3::new(300.0, 64.0, 0.0);
    let far = player(world, center + Vec3::new(4.0, 0.0, 0.0));
    let near = player(world, center + Vec3::new(1.0, 0.0, 0.0));
    let middle = player(world, center + Vec3::new(2.0, 0.0, 0.0));

    let hits = players_within(world, center, 10.0, None);
    assert_eq!(ids(&hits), vec![near, middle, far]);
    assert!((hits[0].distance - 1.0).abs() < 1e-5);

    // Falloff is one at the centre and zero at the edge, which is what an
    // ability multiplies its damage by.
    assert!((hits[0].falloff(10.0) - 0.9).abs() < 1e-5);
    assert_eq!(hits[0].falloff(0.0), 0.0);
}

// --- status effects --------------------------------------------------------

/// A real slow reaches the wire as the effect, amplifier, duration and flags
/// the builder was given.
///
/// Compared against the exact bytes the game's own encoder produced for the
/// same `MobEffectInstance`, pinned in `hyperion-minecraft-proto`'s
/// `play_mob_effect` differential. Byte-for-byte here ties the builder to that
/// proof: `Slowness IV` for 1.5 s with particles and icon is `2a 01 03 1e 06`.
#[test]
fn a_slow_encodes_to_the_bytes_the_jar_produces() {
    let packet = Status::new(MobEffect::Slowness, 3)
        .seconds(1.5)
        .packet(0x2A);
    assert_eq!(encoded(&packet), [0x2A, 0x01, 0x03, 0x1E, 0x06]);
}

/// The default duration is indefinite, so an effect built without a duration
/// does not silently vanish after a tick.
#[test]
fn an_effect_with_no_duration_set_is_indefinite() {
    let packet = Status::new(MobEffect::Speed, 0).packet(1);
    assert_eq!(packet.effect_duration_ticks, -1);
}

/// Seconds round to the nearest tick, the finest the wire counts.
///
/// The two values discriminate rounding from both truncation and ceiling: 1.58
/// s is 31.6 ticks, which a truncation would drop to 31, and 1.52 s is 30.4,
/// which a ceiling would push to 31. Only round-to-nearest gives 32 and 30.
#[test]
fn a_duration_rounds_to_the_nearest_tick() {
    let ticks = |seconds: f32| {
        Status::new(MobEffect::Slowness, 0)
            .seconds(seconds)
            .packet(0)
            .effect_duration_ticks
    };
    assert_eq!(ticks(1.5), 30, "an exact multiple is itself");
    assert_eq!(ticks(1.58), 32, "31.6 rounds up, not down to 31");
    assert_eq!(ticks(1.52), 30, "30.4 rounds down, not up to 31");
}

/// The amplifier is passed through unwidened and unshifted: level IV is a 3 on
/// the wire, not a 4.
#[test]
fn the_amplifier_is_zero_based_on_the_wire() {
    assert_eq!(
        Status::new(MobEffect::Slowness, 3)
            .packet(0)
            .effect_amplifier,
        3
    );
}

/// Each display flag lands in its own bit, and the default is visible plus
/// icon.
#[test]
fn the_display_flags_pack_one_per_bit() {
    // Default: FLAG_VISIBLE (2) | FLAG_SHOW_ICON (4).
    assert_eq!(Status::new(MobEffect::Speed, 0).flags(), 0b110);

    assert_eq!(
        Status::new(MobEffect::Speed, 0)
            .particles(false)
            .icon(false)
            .ambient(true)
            .flags(),
        0b001,
        "ambient alone is bit 0"
    );
    assert_eq!(
        Status::new(MobEffect::Speed, 0).icon(false).flags(),
        0b010,
        "particles alone is bit 1"
    );
    assert_eq!(
        Status::new(MobEffect::Speed, 0)
            .ambient(true)
            .particles(true)
            .icon(true)
            .flags(),
        0b111,
        "all three set"
    );
}

/// The effect carries the id the enum assigns, so a caller naming
/// `MobEffect::Slowness` sends slowness and not the effect one id away.
#[test]
fn the_effect_id_is_the_one_the_enum_names() {
    assert_eq!(
        Status::new(MobEffect::Slowness, 0).packet(0).effect,
        MobEffect::Slowness.id()
    );
    assert_eq!(
        Status::new(MobEffect::Speed, 0).packet(0).effect,
        MobEffect::Speed.id()
    );
}
