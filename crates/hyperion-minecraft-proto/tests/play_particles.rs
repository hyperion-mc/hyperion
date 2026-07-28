//! Wire tests for particles, against Mojang's own dispatch.
//!
//! # Provenance
//!
//! Every expected value is read from `tests/fixtures/vanilla.json`, which
//! `nix/java/VanillaEncoder.java` writes by driving the real
//! `ParticleTypes.STREAM_CODEC` in the pinned `server-26.2.jar`. So a failure
//! here says this crate disagrees with Mojang's encoder, and the
//! `minecraft-encoder-fixtures` flake check says the committed fixtures still
//! match the jar.
//!
//! This file is what makes the generated particle table trustworthy.
//! `nix/generate-particles.py` learns each option's shape by reading the
//! decompiled Java, and a codec read wrong there would produce Rust that
//! compiles, encodes something, and shifts every byte after the particle. The
//! only way to know the reading was right is to compare against the encoder it
//! was read from, which is what happens below for all thirteen shapes.

mod vanilla_fixtures;

use hyperion_minecraft_proto::{
    Encode, Reader, Writer,
    item::{DataComponentPatch, ItemStackTemplate, nbt::Scanner},
    packets::play::chunk::LevelParticles,
    particle::{Argb, Particle, ParticleKind, PositionSource},
    types::{BlockPos, BlockStateId, RegistryId, Vec3},
};

fn encoded(value: &impl Encode) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

/// Everything the harness builds, paired with the Rust that should match it.
///
/// One list rather than a test each, because the point is coverage of every
/// body shape and a shape added to the generator without a row here would
/// otherwise go unchecked.
fn cases() -> Vec<(&'static str, Particle<'static>)> {
    vec![
        ("flame", Particle::Flame),
        ("block", Particle::Block {
            state: BlockStateId::new(1),
        }),
        ("item", Particle::Item {
            item_stack: ItemStackTemplate {
                item: RegistryId(964),
                count: 3,
                components: DataComponentPatch::empty(),
            },
        }),
        ("dust", Particle::Dust {
            color: Argb::opaque(0xff, 0x00, 0x00),
            scale: 2.0,
        }),
        ("dust_color_transition", Particle::DustColorTransition {
            from_color: Argb::opaque(0xff, 0x00, 0x00),
            to_color: Argb::opaque(0x00, 0x00, 0xff),
            scale: 1.5,
        }),
        ("entity_effect", Particle::EntityEffect {
            color: Argb::new(0x80, 0x33, 0x66, 0x99),
        }),
        ("vibration", Particle::Vibration {
            destination: PositionSource::Block {
                pos: BlockPos::new(1, -2, 3),
            },
            arrival_in_ticks: 40,
        }),
        ("dragon_breath", Particle::DragonBreath { power: 1.0 }),
        ("effect", Particle::Effect {
            color: Argb::new(0xff, 0x11, 0x22, 0x33),
            power: 0.75,
        }),
        ("sculk_charge", Particle::SculkCharge { roll: 0.5 }),
        ("shriek", Particle::Shriek { delay: 17 }),
        ("trail", Particle::Trail {
            target: Vec3 {
                x: 1.5,
                y: -2.25,
                z: 3.75,
            },
            color: Argb::opaque(0x00, 0xff, 0x00),
            duration: 30,
        }),
        ("geyser", Particle::Geyser { water_blocks: 5 }),
        ("geyser_base", Particle::GeyserBase {
            water_blocks: 5,
            burst_impulse_base: 0.25,
        }),
    ]
}

/// Every option shape, byte for byte against the game's own dispatch.
#[test]
fn every_particle_body_matches_the_vanilla_codec() {
    for (name, particle) in cases() {
        let expected = vanilla_fixtures::bytes(&format!("particle.{name}"));
        assert_eq!(
            encoded(&particle),
            expected,
            "{name} ({}) does not encode the way the jar does",
            particle.name()
        );
    }
}

/// And the same shapes inside the packet that carries them, so a field order
/// that is right in isolation and wrong in the packet still fails.
#[test]
fn every_particle_packet_matches_the_vanilla_codec() {
    for (name, particle) in cases() {
        let packet = LevelParticles {
            override_limiter: true,
            always_show: false,
            x: 1.5,
            y: 64.0625,
            z: -2.25,
            x_dist: 0.5,
            y_dist: 0.25,
            z_dist: 0.125,
            max_speed: 0.75,
            count: 100,
            particle: particle.clone(),
        };
        let expected = vanilla_fixtures::bytes(&format!("packet.level_particles.{name}"));
        assert_eq!(encoded(&packet), expected, "level_particles with {name}");
    }
}

/// What the encoder writes, the decoder reads back unchanged.
#[test]
fn every_particle_round_trips() {
    for (name, particle) in cases() {
        let bytes = encoded(&particle);
        let mut reader = Reader::new(&bytes);
        let decoded = Particle::decode(&mut reader, &Scanner)
            .unwrap_or_else(|error| panic!("{name} did not decode: {error}"));
        assert_eq!(decoded, particle, "{name}");
        assert_eq!(reader.remaining_len(), 0, "{name} left bytes behind");
    }
}

/// The ids the table hard-codes are positions in a registry Mojang reorders,
/// so two of them are pinned against the jar directly rather than only against
/// this repository's own copy of the registry.
#[test]
fn particle_ids_match_the_jar() {
    for (name, kind) in [("flame", ParticleKind::Flame), ("dust", ParticleKind::Dust)] {
        let expected: u16 = vanilla_fixtures::get(&format!("particle_id.{name}"))
            .parse()
            .expect("a fixture id is a number");
        assert_eq!(kind.id(), expected, "{name}");
    }
}

/// A colour is four channels packed into the `int` the wire carries, and the
/// order they pack in is the difference between red and blue.
#[test]
fn a_colour_packs_alpha_first_and_blue_last() {
    let colour = Argb::new(0x12, 0x34, 0x56, 0x78);
    assert_eq!(colour.channels(), [0x12, 0x34, 0x56, 0x78]);
    assert_eq!(colour.to_bits(), 0x1234_5678);
    assert_eq!(Argb::from_bits(0x1234_5678), colour);
    assert_eq!(Argb::opaque(0x34, 0x56, 0x78).channels()[0], 0xff);
}

/// A state id past the end of this version's registry is a decode error, not a
/// particle drawn as air. Reachable only from the wire, and only from a sender
/// on a different game version.
#[test]
fn a_block_state_from_another_version_is_refused() {
    assert!(BlockStateId::from_raw(-1).is_err());
    assert!(BlockStateId::from_raw(i32::MAX).is_err());
    assert!(BlockStateId::from_raw(0).is_ok());

    // `minecraft:block` with a state id no version of this game has.
    let mut reader = Reader::new(&[0x01, 0xFF, 0xFF, 0xFF, 0x7F]);
    assert!(Particle::decode(&mut reader, &Scanner).is_err());
}
