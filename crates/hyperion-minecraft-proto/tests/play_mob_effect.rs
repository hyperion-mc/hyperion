//! Wire tests for the two mob-effect packets, against Mojang's own codec.
//!
//! # Provenance
//!
//! Every expected value is read from `tests/fixtures/vanilla.json`, which
//! `nix/java/VanillaEncoder.java` writes by driving the real
//! `ClientboundUpdateMobEffectPacket.STREAM_CODEC` and its remove counterpart
//! in the pinned `server-26.2.jar`. So a failure here says this crate disagrees
//! with Mojang's encoder, and the `minecraft-encoder-fixtures` flake check says
//! the committed fixtures still match the jar.
//!
//! The one subtlety these pin is the effect id. `MobEffect.STREAM_CODEC` is
//! `ByteBufCodecs.holderRegistry`, and this crate types the field as a plain
//! `RegistryId` written as a bare varint. `holderRegistry` writes through
//! `Registry.asHolderIdMap`, which keeps the registry's own ids, so no `+1`
//! bias is applied and the plain id is right -- but that is a fact about
//! Mojang's code, and the only honest way to hold it is against Mojang's
//! output, which is what the effect-id checks below do.

mod vanilla_fixtures;

use hyperion_minecraft_proto::{
    Decode, Encode, Reader, RegistryId, Writer,
    generated::registry::MobEffect,
    packets::play::clientbound::{RemoveMobEffect, UpdateMobEffect},
};

fn encoded(value: &impl Encode) -> Vec<u8> {
    let mut writer = Writer::new();
    value.encode(&mut writer).expect("encode");
    writer.into_vec()
}

/// The registry id the enum assigns matches the one the jar sends, for the two
/// effects the tooltips reach for. A drifted id would slow a player with the
/// wrong effect and nothing else would notice.
#[test]
fn effect_ids_match_the_jar() {
    assert_eq!(
        MobEffect::Slowness.id(),
        RegistryId(vanilla_fixtures::number("mob_effect_id.slowness"))
    );
    assert_eq!(
        MobEffect::Speed.id(),
        RegistryId(vanilla_fixtures::number("mob_effect_id.speed"))
    );
}

/// A real slow: Slowness IV for a second and a half, particles and icon on.
///
/// The fixture is the packet the game's own encoder produces for exactly that
/// `MobEffectInstance`, so this pins the amplifier, the duration and the flag
/// byte all at once, and the effect id as an unbiased varint.
#[test]
fn a_slow_matches_the_vanilla_codec() {
    let packet = UpdateMobEffect {
        entity_id: 0x2A,
        effect: MobEffect::Slowness.id(),
        // Zero-based: amplifier 3 is the level a tooltip writes as IV.
        effect_amplifier: 3,
        effect_duration_ticks: 30,
        // FLAG_VISIBLE | FLAG_SHOW_ICON.
        flags: 0b110,
    };
    assert_eq!(
        encoded(&packet),
        vanilla_fixtures::bytes("packet.update_mob_effect.slowness")
    );
}

/// A real speed buff on the caster: Speed II, indefinite.
///
/// Pins the infinite-duration encoding -- a `-1` varint, which is the
/// five-byte `ff ff ff ff 0f`, not a short negative -- which is how an effect
/// that ends on a condition rather than a clock is spelled.
#[test]
fn an_indefinite_speed_matches_the_vanilla_codec() {
    let packet = UpdateMobEffect {
        entity_id: 0x2A,
        effect: MobEffect::Speed.id(),
        effect_amplifier: 1,
        effect_duration_ticks: -1,
        flags: 0b110,
    };
    let bytes = encoded(&packet);
    assert_eq!(
        bytes,
        vanilla_fixtures::bytes("packet.update_mob_effect.speed")
    );
    // The infinite duration really is the five-byte varint and not a truncated
    // one, so a client reads it as "does not expire" rather than as some large
    // finite tick count.
    assert!(bytes.ends_with(&[0xff, 0xff, 0xff, 0xff, 0x0f, 0b110]));
}

/// Ending an effect early is the remove packet, and it carries only the entity
/// and the effect id.
#[test]
fn a_clear_matches_the_vanilla_codec() {
    let packet = RemoveMobEffect {
        entity_id: 0x2A,
        effect: MobEffect::Slowness.id(),
    };
    assert_eq!(
        encoded(&packet),
        vanilla_fixtures::bytes("packet.remove_mob_effect.slowness")
    );
}

/// What the encoder writes, the decoder reads back unchanged.
#[test]
fn both_packets_round_trip() {
    let update = UpdateMobEffect {
        entity_id: 7,
        effect: MobEffect::Wither.id(),
        effect_amplifier: 2,
        effect_duration_ticks: 100,
        flags: 0b100,
    };
    let bytes = encoded(&update);
    let mut reader = Reader::new(&bytes);
    assert_eq!(UpdateMobEffect::decode(&mut reader).unwrap(), update);
    assert_eq!(reader.remaining_len(), 0);

    let remove = RemoveMobEffect {
        entity_id: 7,
        effect: MobEffect::Wither.id(),
    };
    let bytes = encoded(&remove);
    let mut reader = Reader::new(&bytes);
    assert_eq!(RemoveMobEffect::decode(&mut reader).unwrap(), remove);
    assert_eq!(reader.remaining_len(), 0);
}
