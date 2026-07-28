//! Every generated Java enum, against the numbers the server itself produced.
//!
//! The types under [`hyperion_minecraft_proto::generated::java_enum`] are read
//! out of decompiled Java by a regex in `nix/extract-protocol.py`. No test
//! written from that same reading could catch a misreading of it, so the
//! numbers here come from the other side: `nix/java/VanillaEncoder.java` runs
//! the class's own `StreamCodec` where it publishes one and reads the `VarInt`
//! back out, and reads `ordinal()` or `id()` off the class where it does not.
//!
//! What this catches, which the generator cannot catch for itself: a constant
//! it invented, a constant it dropped, a constant numbered wrong, and the
//! whole set being one out because the wrong field was read for the id.

mod vanilla_fixtures;

use hyperion_minecraft_proto::generated::java_enum::{
    BossBarColor, BossBarOverlay, ChatTypeParameter, Direction, DisplaySlot, EquipmentSlot,
    HeightmapKind, ObjectiveRenderType, TeamCollisionRule, TeamColor, TeamVisibility,
};

/// Assert one enum's whole constant set against the fixtures.
///
/// Both directions: every variant matches the server's number for the name it
/// claims, and every `java_enum.<Type>.` fixture has a variant. The second
/// half is what catches a dropped constant, which the first half cannot see.
fn check<T: Copy + core::fmt::Debug + PartialEq>(
    type_name: &str,
    all: &[T],
    id: impl Fn(T) -> i32,
    name: impl Fn(T) -> &'static str,
) {
    for &value in all {
        let key = format!("java_enum.{type_name}.{}", name(value));
        assert_eq!(
            id(value),
            vanilla_fixtures::number(&key),
            "{type_name}::{value:?} disagrees with the server about its number"
        );
    }
    let prefix = format!("java_enum.{type_name}.");
    let from_server = vanilla_fixtures::keys_with_prefix(&prefix);
    assert_eq!(
        all.len(),
        from_server.len(),
        "{type_name} has {} variants but the server declares {}: {from_server:?}",
        all.len(),
        from_server.len()
    );
}

macro_rules! check_enum {
    ($name:ident) => {
        check(stringify!($name), $name::ALL, $name::id, $name::name);
    };
}

#[test]
fn every_generated_java_enum_matches_the_server() {
    check_enum!(BossBarColor);
    check_enum!(BossBarOverlay);
    check_enum!(ChatTypeParameter);
    check_enum!(Direction);
    check_enum!(DisplaySlot);
    check_enum!(EquipmentSlot);
    check_enum!(HeightmapKind);
    check_enum!(ObjectiveRenderType);
    check_enum!(TeamCollisionRule);
    check_enum!(TeamColor);
    check_enum!(TeamVisibility);
}

/// `from_name` is a binary search over a table the generator sorted, and a
/// wrong order fails as a lookup miss rather than as a build error. So every
/// name goes back through it rather than a few being spot-checked.
#[test]
fn every_name_resolves_back_to_its_variant() {
    fn round_trip<T: Copy + core::fmt::Debug + PartialEq>(
        all: &[T],
        name: impl Fn(T) -> &'static str,
        from_name: impl Fn(&str) -> Option<T>,
    ) {
        for &value in all {
            assert_eq!(from_name(name(value)), Some(value));
        }
        assert_eq!(from_name("NOT_A_CONSTANT"), None);
    }

    round_trip(
        BossBarColor::ALL,
        BossBarColor::name,
        BossBarColor::from_name,
    );
    round_trip(
        BossBarOverlay::ALL,
        BossBarOverlay::name,
        BossBarOverlay::from_name,
    );
    round_trip(
        ChatTypeParameter::ALL,
        ChatTypeParameter::name,
        ChatTypeParameter::from_name,
    );
    round_trip(Direction::ALL, Direction::name, Direction::from_name);
    round_trip(DisplaySlot::ALL, DisplaySlot::name, DisplaySlot::from_name);
    round_trip(
        EquipmentSlot::ALL,
        EquipmentSlot::name,
        EquipmentSlot::from_name,
    );
    round_trip(
        HeightmapKind::ALL,
        HeightmapKind::name,
        HeightmapKind::from_name,
    );
    round_trip(
        ObjectiveRenderType::ALL,
        ObjectiveRenderType::name,
        ObjectiveRenderType::from_name,
    );
    round_trip(
        TeamCollisionRule::ALL,
        TeamCollisionRule::name,
        TeamCollisionRule::from_name,
    );
    round_trip(TeamColor::ALL, TeamColor::name, TeamColor::from_name);
    round_trip(
        TeamVisibility::ALL,
        TeamVisibility::name,
        TeamVisibility::from_name,
    );
}

/// An id off the wire is the one place a value can fail to exist, so the door
/// is checked in both directions rather than assumed from `ALL`'s length.
#[test]
fn an_id_outside_the_set_is_refused() {
    assert_eq!(BossBarColor::from_id(7), None);
    assert_eq!(BossBarColor::from_id(-1), None);
    assert_eq!(DisplaySlot::from_id(19), None);
    assert_eq!(TeamColor::from_id(16), None);
    assert_eq!(EquipmentSlot::from_id(8), None);
}
