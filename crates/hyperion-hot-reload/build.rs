//! Two jobs, both about making the dylib boundary safe to cross.
//!
//! First, record the exact compiler that built this crate: `repr(Rust)` has no stable
//! ABI, so a module and a host built by different rustc versions can disagree about the
//! layout of any type they pass. The recorded string is compared at load time.
//!
//! Second, keep flecs's C symbols in this dylib and exported from it. rustc links
//! `libflecs.a` into whichever artifact first needs it and dead-strips the rest, and it
//! restricts a dylib's export list to that crate's own Rust symbols. Without this, a game
//! module that links `hyperion-hot-reload` dynamically finds no `ecs_*` symbols and gets
//! its own static copy of flecs instead -- two `ecs_os_api` globals in one process, which
//! shows up as a jump through a null function pointer at the module's first allocation.

fn main() {
    record_rustc();
    export_flecs_symbols();
}

fn record_rustc() {
    println!("cargo::rerun-if-env-changed=RUSTC");
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let out = std::process::Command::new(rustc)
        .arg("-vV")
        .output()
        .expect("failed to run rustc -vV");
    let text = String::from_utf8(out.stdout).expect("rustc -vV was not utf-8");
    // Release, commit hash and host triple all change the ABI.
    let fingerprint = text
        .lines()
        .filter(|l| {
            l.starts_with("rustc ") || l.starts_with("commit-hash") || l.starts_with("host")
        })
        .collect::<Vec<_>>()
        .join("; ");
    println!("cargo::rustc-env=HYPERION_HOT_RELOAD_RUSTC={fingerprint}");
}

/// The flecs C symbol patterns both platforms have to re-export, spelled without the
/// leading underscore Mach-O adds.
const FLECS_EXPORTS: [&str; 4] = ["ecs_*", "flecs_*", "Ecs*", "FLECS_*"];

fn export_flecs_symbols() {
    println!("cargo::rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "macos" | "ios") {
        println!("cargo::rustc-link-arg=-Wl,-all_load");
        // ld64 unions -exported_symbol with the export list rustc generates.
        for pattern in FLECS_EXPORTS {
            println!("cargo::rustc-link-arg=-Wl,-exported_symbol,_{pattern}");
        }
    } else {
        export_flecs_symbols_elf();
    }
}

/// ELF needs a version script, because `--export-dynamic` cannot undo what rustc does.
///
/// rustc links a `dylib` with its own anonymous version script ending in `local: *`, which
/// demotes every symbol it did not generate. flecs's C symbols land in the object with
/// `DEFAULT` visibility and `LOCAL` binding, so they are present and unreachable, and
/// `--export-dynamic` and `--export-dynamic-symbol` are both powerless against a
/// version-script demotion. Measured on x86_64-linux: 9001 exported symbols, zero of them
/// `ecs_*`, `ecs_init` reading `FUNC LOCAL DEFAULT`.
///
/// A game module that cannot resolve `ecs_*` here links its own copy of `libflecs.a`
/// instead, which is two `ecs_os_api` globals in one process -- the failure `AbiToken`
/// exists to catch, arriving on a platform where the check itself could not run.
///
/// ld merges multiple version scripts and an explicit pattern beats a `*` wildcard, so a
/// second script naming these globs promotes exactly them and leaves rustc's own exports
/// alone. Same measurement after: 10717 exported, 666 of them `ecs_*`, `ecs_init` GLOBAL.
fn export_flecs_symbols_elf() {
    let out_dir = std::env::var("OUT_DIR").expect("cargo always sets OUT_DIR");
    let script = std::path::Path::new(&out_dir).join("flecs-exports.map");

    let mut text = String::from("{\n  global:\n");
    for pattern in FLECS_EXPORTS {
        text.push_str("    ");
        text.push_str(pattern);
        text.push_str(";\n");
    }
    // No `local:` clause. This script adds to rustc's export list rather than replacing
    // it; a `local: *` here would hide every Rust symbol the host resolves through.
    text.push_str("};\n");

    std::fs::write(&script, text).expect("failed to write the flecs version script");
    println!(
        "cargo::rustc-link-arg=-Wl,--version-script={}",
        script.display()
    );
}
