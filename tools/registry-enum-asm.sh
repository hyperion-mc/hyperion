#!/usr/bin/env bash
#
# Print the machine code a registry id compiles to.
#
# The zero-cost claim about `generated::registry` is enforced by `const`
# assertions in the generated files: a `const` context cannot call `id()`
# unless the compiler folds it, so those fail the build rather than a test.
# What they cannot show is the *runtime-shaped* case -- `id()` on a value the
# optimiser cannot see through -- because there is no compile-time value there
# to fold. This script is how that one is checked, by reading the assembly.
#
# It is a script and not a test on purpose. A test that shells out to cargo,
# compiles the crate a second time into its own target directory and greps
# instruction mnemonics would be slow, would be architecture-specific, and
# would fail for reasons that have nothing to do with the property. Run this
# when the shape of the generated enums changes, and paste what it prints.
#
# Usage: tools/registry-enum-asm.sh
set -euo pipefail

root=$(git rev-parse --show-toplevel)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

mkdir -p "$work/src"
cat > "$work/Cargo.toml" <<TOML
[package]
name = "registry-enum-asm-probe"
version = "0.0.0"
edition = "2024"

[workspace]

[profile.release]
debug = false

[dependencies]
hyperion-minecraft-proto = { path = "$root/crates/hyperion-minecraft-proto" }
TOML

# Four probes, in increasing order of how much the optimiser is allowed to
# know. The first two are the interesting ones: an argument arriving from
# outside is a value no constant folding can reach.
cat > "$work/src/lib.rs" <<'RUST'
use hyperion_minecraft_proto::generated::registry::{Block, SoundEvent};

/// A registry value that came from somewhere else, turned into its wire id.
#[unsafe(no_mangle)]
pub extern "C" fn probe_opaque_sound_id(sound: SoundEvent) -> i32 {
    sound.id().0
}

/// The same, for a registry wide enough to need two bytes and a block state.
#[unsafe(no_mangle)]
pub extern "C" fn probe_opaque_block_id(block: Block) -> i32 {
    block.id().0
}

/// A variant named at the call site, which is what game code writes.
#[unsafe(no_mangle)]
pub extern "C" fn probe_named_sound_id() -> i32 {
    SoundEvent::EntityArrowHit.id().0
}

/// Matching on a registry value, to show a `match` over a closed enum needs no
/// arm for a case that cannot occur and costs a compare rather than a lookup.
#[unsafe(no_mangle)]
pub extern "C" fn probe_match_sound(sound: SoundEvent) -> u8 {
    match sound {
        SoundEvent::EntityArrowHit => 1,
        SoundEvent::BlockNoteBlockHat => 2,
        _ => 0,
    }
}
RUST

cd "$work"
cargo rustc --release --quiet -- --emit asm -C llvm-args=--x86-asm-syntax=intel >/dev/null

asm=$(find target/release/deps -name 'registry_enum_asm_probe*.s' | head -n 1)
echo "# rustc $(rustc --version | cut -d' ' -f2), $(rustc -vV | sed -n 's/^host: //p')"
for probe in probe_opaque_sound_id probe_opaque_block_id probe_named_sound_id probe_match_sound; do
  echo
  awk -v want="$probe" '
    $0 ~ "^_?" want ":" { on = 1 }
    on { print }
    on && /\.cfi_endproc/ { exit }
  ' "$asm"
done
