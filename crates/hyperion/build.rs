//! Re-exports LMDB's C symbols from this crate's dylib.
//!
//! ## Why this is needed
//!
//! rustc links a `dylib` with its own anonymous version script ending in `local: *`, which
//! demotes every symbol it did not generate. `heed` pulls in `liblmdb.a`, so LMDB's C ends
//! up absorbed into `libhyperion.so` and hidden -- present and unreachable:
//!
//! ```text
//! $ readelf --dyn-syms libhyperion.so | grep -c mdb_
//! 0
//! $ readelf --syms libhyperion.so | grep mdb_env_open
//!  14356: 0000000000d0df3c   933 FUNC    LOCAL  DEFAULT   13 mdb_env_open
//! ```
//!
//! 133 definitions, none reachable. That is fatal rather than merely wasteful because
//! `heed`'s API is generic: every consumer monomorphises heed's code into its own rlib and
//! emits its own calls to `mdb_*`. `hyperion-permission` is such a consumer and is linked
//! into an event's server binary statically, while rustc suppresses LMDB's own `-llmdb` on
//! the grounds that an upstream dylib already provides it. The result is a link that fails
//! only once `hyperion` is built as a dylib, which is to say only under hot reload:
//!
//! ```text
//! rust-lld: error: undefined symbol: mdb_dbi_open
//! ```
//!
//! Re-exporting keeps ONE copy of LMDB in the process. Linking a second `liblmdb.a` into
//! the executable would also make the link succeed, and would put two copies of a C
//! library with process-global state in one process -- the same shape
//! `checks.hot-reload-index-probe` exists to reject for `flecs_ecs`.
//!
//! Neither `-Wl,--export-dynamic` nor `-Wl,--export-dynamic-symbol=mdb_*` reverses a
//! version-script demotion; the latter was measured here leaving the exported count at
//! zero, independently reproducing what `flecs_ecs/build.rs` documents for `ecs_*`. What
//! works is a second version script: ld merges them, and an explicit pattern beats a `*`
//! wildcard, so naming these globs promotes exactly them while leaving rustc's own export
//! list intact.
//!
//! ## The general rule
//!
//! Any native static library absorbed into this dylib, whose symbols a downstream
//! monomorphisation calls, needs its glob here. The signature is an undefined `foo_*` at
//! an event's final link whose definition is `LOCAL` in `libhyperion.so`'s `.symtab`.
//! `flecs_ecs` carries its own copy of this build script for `ecs_*`, because a build
//! script's `rustc-link-arg` applies to its own crate's artifacts and nothing else.

// A build script talks to cargo over stdout and has no other channel, so the
// workspace-wide `print_stdout = deny` does not apply to this file.
#![allow(
    clippy::print_stdout,
    reason = "the cargo build-script protocol is stdout"
)]

/// Spelled without the leading underscore Mach-O adds.
const LMDB_EXPORTS: [&str; 1] = ["mdb_*"];

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "macos" | "ios") {
        // ld64 unions -exported_symbol with the export list rustc generates, so no second
        // script is needed and none is available.
        for pattern in LMDB_EXPORTS {
            println!("cargo::rustc-link-arg=-Wl,-exported_symbol,_{pattern}");
        }
        return;
    }

    let out_dir = std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR");
    let script = std::path::Path::new(&out_dir).join("lmdb-exports.map");

    let mut text = String::from("{\n  global:\n");
    for pattern in LMDB_EXPORTS {
        text.push_str("    ");
        text.push_str(pattern);
        text.push_str(";\n");
    }
    // Deliberately no `local:` clause. This script adds to rustc's export list; a
    // `local: *` here would hide every Rust symbol a consumer resolves through.
    text.push_str("};\n");

    std::fs::write(&script, text).expect("failed to write the lmdb version script");
    println!(
        "cargo::rustc-link-arg=-Wl,--version-script={}",
        script.display()
    );
}
