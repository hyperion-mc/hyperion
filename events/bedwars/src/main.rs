//! bedwars' entry point.
//!
//! No `#[global_allocator]`, for the reason written out in full in
//! `events/smash/src/main.rs`: a Rust global allocator and the `-C
//! prefer-dynamic` dylib split put two allocators in one process, and the first
//! pointer that crosses between them segfaults. bedwars is not packaged that way
//! yet, and adding an event to `hotReloadEvents` is meant to be one attrset --
//! so the landmine is removed here rather than left for whoever does it.

use bedwars::init_game;

fn main() -> anyhow::Result<()> {
    // bedwars takes no deployment paths: it has no reloadable rules, so that
    // part of `Args` is none of its business. It does take a console, which is
    // engine-level and asks the same operator questions of either game.
    hyperion_event_runner::run("BEDWARS_", |args, crypto| {
        init_game(args.address(), crypto, args.console()?)
    })
}
