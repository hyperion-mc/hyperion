// Each integration test compiles its own copy of this module, and none of them
// wants every helper, so unused ones are expected rather than a warning to fix.
#![allow(dead_code)]

//! Reference bytes produced by Mojang's own encoders.
//!
//! `tests/fixtures/vanilla.json` is the output of
//! `nix run .#minecraft-encode -- fixtures <file>`, which drives the real
//! `StreamCodec`s and netty handlers in the pinned server jar. It is committed
//! so `cargo test` works without nix, and the `minecraft-encoder-fixtures`
//! flake check fails if the committed copy drifts from what the jar produces.
//!
//! The parser below is deliberately tiny: the file is a flat map of string to
//! string with no escapes beyond the two `quote` in the harness emits, and a
//! JSON dependency for that would be the only dev-dependency in the crate.

/// Look up one fixture by name.
///
/// # Panics
/// Panics when the name is absent, which means the harness and the test have
/// drifted apart rather than that the codec is wrong.
pub fn get(name: &str) -> &'static str {
    let text = include_str!("../fixtures/vanilla.json");
    let needle = format!("\"{name}\": \"");
    let start = text
        .find(&needle)
        .unwrap_or_else(|| panic!("no fixture named {name}; regenerate tests/fixtures/vanilla.json"))
        + needle.len();
    let end = start
        + text[start..]
            .find('"')
            .expect("fixture value is terminated");
    &text[start..end]
}

/// A fixture read as bytes.
///
/// # Panics
/// Panics on a value that is not an even-length run of hex digits.
pub fn bytes(name: &str) -> Vec<u8> {
    let hex = get(name);
    assert!(hex.len().is_multiple_of(2), "{name} is not whole bytes");
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16)
                .unwrap_or_else(|_| panic!("{name} is not hex"))
        })
        .collect()
}

/// A fixture read as a number.
///
/// # Panics
/// Panics on a value that is not a decimal integer.
pub fn number(name: &str) -> i32 {
    get(name)
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a number"))
}

/// Render bytes the way the fixture file spells them, for assertion messages.
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut out, byte| {
        let _unused = write!(out, "{byte:02x}");
        out
    })
}
