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

fn export_flecs_symbols() {
    println!("cargo::rerun-if-changed=build.rs");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if matches!(target_os.as_str(), "macos" | "ios") {
        println!("cargo::rustc-link-arg=-Wl,-all_load");
        // ld64 unions -exported_symbol with the export list rustc generates.
        for pattern in ["_ecs_*", "_flecs_*", "_Ecs*", "_FLECS_*"] {
            println!("cargo::rustc-link-arg=-Wl,-exported_symbol,{pattern}");
        }
    } else {
        println!("cargo::rustc-link-arg=-Wl,--export-dynamic");
    }
}
