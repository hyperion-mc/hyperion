fn main() {
    // Get target architecture (e.g., x86_64, aarch64, etc.)
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "unknown".to_string());

    // Get profile (debug/release)
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };

    // Combine them into RUN_MODE
    let run_mode = format!("{arch}-{profile}");
    println!("cargo:rustc-env=RUN_MODE={run_mode}");
}
