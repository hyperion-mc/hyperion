//! What the generated registry enums promise, held to all 6854 of their
//! variants at once.
//!
//! Every test here walks [`ALL_ENUMS`], which the generator emits alongside the
//! enums themselves, rather than naming registries by hand. A registry added by
//! a version bump is covered the day it appears; a hand-written list would have
//! covered whatever was interesting the day it was written.
//!
//! The zero-cost claim is *not* tested here. It cannot be: a test observes a
//! running program, and the claim is about what never reaches the program. It
//! is a `const` assertion at the bottom of every generated registry file, which
//! the compiler checks on every build. The two probes at the end of this file
//! are `const` for the same reason.

use hyperion_minecraft_proto::{
    generated::registry::{self, ALL_ENUMS, SoundEvent},
    types::RegistryId,
};

/// The one registry with names here and no enum, and the module that owns it.
///
/// Named rather than skipped by a pattern: a second carve-out should have to be
/// written down, not fall through a loosened condition.
const NO_ENUM: [&str; 1] = ["minecraft:particle_type"];

#[test]
fn every_registry_but_the_named_carve_out_has_an_enum() {
    let with_enums: Vec<&str> = ALL_ENUMS.iter().map(|entry| entry.name).collect();
    let missing: Vec<&str> = registry::ALL
        .iter()
        .map(|table| table.name)
        .filter(|name| !with_enums.contains(name))
        .collect();
    assert_eq!(
        missing,
        NO_ENUM.to_vec(),
        "a registry lost or gained an enum; if that was deliberate, say so in NO_ENUM here and at \
         the exclusion site in nix/generate-rust.py"
    );
}

/// The property the whole design rests on: the discriminant is the network id.
#[test]
fn the_id_of_the_nth_entry_is_n() {
    for entry in ALL_ENUMS {
        for id in 0..i32::try_from(entry.count).expect("registry fits in i32") {
            let name = (entry.name_of)(id)
                .unwrap_or_else(|| panic!("{}: no entry at id {id}", entry.rust_type));
            assert_eq!(
                (entry.id_of)(name),
                Some(id),
                "{}: {name} resolves to a different id than the one it sits at",
                entry.rust_type
            );
        }
    }
}

/// `from_name` is a binary search over a table the generator sorted. A wrong
/// order fails as a lookup miss, which reads as "that entry does not exist", so
/// every name goes back through it rather than a sample.
#[test]
fn every_name_resolves_through_the_binary_search() {
    let mut checked = 0usize;
    for table in registry::ALL {
        let Some(entry) = ALL_ENUMS.iter().find(|e| e.name == table.name) else {
            continue;
        };
        for (id, name) in table.entries.iter().enumerate() {
            let id = i32::try_from(id).expect("registry fits in i32");
            assert_eq!(
                (entry.id_of)(name),
                Some(id),
                "{}: from_name({name}) missed; the by-name table is out of order",
                entry.rust_type
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 6854,
        "the number of names covered changed; that is fine after a version bump, but it should be \
         a deliberate edit here"
    );
}

/// The decode boundary is the only place a value can fail to exist, and it
/// refuses rather than saturating.
#[test]
fn from_id_refuses_everything_outside_the_registry() {
    for entry in ALL_ENUMS {
        let count = i32::try_from(entry.count).expect("registry fits in i32");
        for outside in [-1, -count, count, count + 1, i32::MAX, i32::MIN] {
            assert_eq!(
                (entry.name_of)(outside),
                None,
                "{}: accepted the out-of-range id {outside}",
                entry.rust_type
            );
        }
    }
}

#[test]
fn a_name_that_is_not_in_the_registry_resolves_to_nothing() {
    for entry in ALL_ENUMS {
        for absent in ["", "minecraft:", "minecraft:no_such_entry_at_all", "zzzz"] {
            assert_eq!(
                (entry.id_of)(absent),
                None,
                "{}: accepted {absent:?}",
                entry.rust_type
            );
        }
    }
}

/// The name a registry table reports and the name its enum reports are the same
/// array, and this is what says so.
#[test]
fn the_table_and_the_enum_agree_about_every_name() {
    for table in registry::ALL {
        let Some(entry) = ALL_ENUMS.iter().find(|e| e.name == table.name) else {
            continue;
        };
        assert_eq!(
            table.entries.len(),
            entry.count,
            "{}: the table and the enum disagree about how many entries there are",
            entry.rust_type
        );
        for (id, name) in table.entries.iter().enumerate() {
            let id = i32::try_from(id).expect("registry fits in i32");
            assert_eq!((entry.name_of)(id), Some(*name), "{}", entry.rust_type);
        }
    }
}

/// A registry value is as wide as its registry needs and no wider.
///
/// The point is what it replaces: the hand-written `EntityType` was a
/// `&'static str` plus an `i32`, so 16 bytes with padding, and every copy moved
/// a pointer. `EntityType` here is one byte.
#[test]
fn a_value_is_no_wider_than_its_registry_needs() {
    for entry in ALL_ENUMS {
        let wanted = if entry.count <= 1 << 8 {
            1
        } else if entry.count <= 1 << 16 {
            2
        } else {
            4
        };
        assert_eq!(
            entry.width, wanted,
            "{} has {} entries and a {}-byte discriminant",
            entry.rust_type, entry.count, entry.width
        );
    }
}

#[test]
fn size_of_matches_the_width_the_table_reports() {
    assert_eq!(size_of::<SoundEvent>(), 2, "1968 entries need two bytes");
    assert_eq!(
        size_of::<registry::Fluid>(),
        1,
        "five entries need one byte"
    );
    assert_eq!(
        size_of::<Option<SoundEvent>>(),
        2,
        "a closed enum with room left in its discriminant gives `Option` its niche for free, so \
         the decode boundary costs no extra byte"
    );
}

#[test]
fn display_is_the_registry_name() {
    assert_eq!(
        SoundEvent::EntityArrowHit.to_string(),
        "minecraft:entity.arrow.hit"
    );
}

#[test]
fn a_round_trip_through_the_wire_id_is_the_identity() {
    let sound = SoundEvent::BlockNoteBlockHat;
    assert_eq!(SoundEvent::from_id(sound.id()), Some(sound));
    assert_eq!(SoundEvent::from_name(sound.name()), Some(sound));
}

// --- the zero-cost probes ---------------------------------------------------
//
// `const` rather than `#[test]`, deliberately. A test would observe the value
// at run time and prove nothing about whether the compiler had to compute it; a
// `const` cannot be evaluated at all unless the compiler folds it, so these
// fail the *build* if `id()` ever stops being a cast off the discriminant.
//
// The same assertion is generated at the bottom of all 94 registry files. These
// two are spelled out because they are the ones a reader comes here to find,
// and because they name the values the game actually sends.

const ARROW_HIT: i32 = SoundEvent::EntityArrowHit.id().0;
const _: () = assert!(ARROW_HIT == 85);

const NOTE_BLOCK_HAT: i32 = SoundEvent::BlockNoteBlockHat.id().0;
const _: () = assert!(NOTE_BLOCK_HAT == 1168);

// And in the other direction, so a reordered registry cannot leave the ids
// right while the names have moved.
const _: () = assert!(SoundEvent::ALL[85] as u16 == 85);

/// The one runtime-shaped probe: `id()` on a value the optimiser cannot see
/// through still has to be a cast and nothing more.
///
/// This is here to be *read* rather than to assert. The assembly it compiles to
/// is in the PR that added this file, and `tools/registry-enum-asm.sh`
/// reproduces it.
#[test]
fn id_of_an_opaque_value_is_still_the_discriminant() {
    let opaque = std::hint::black_box(SoundEvent::EntityArrowHit);
    assert_eq!(opaque.id(), RegistryId(85));
}
