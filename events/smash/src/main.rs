//! smash's entry point.
//!
//! # There is no `#[global_allocator]` here, and that is not an oversight
//!
//! There was: `tikv_jemallocator::Jemalloc`. It cannot coexist with the dylib
//! split this server is packaged with, and the failure is a segfault before
//! `main` gets anywhere.
//!
//! `nix/hot-reload/packaging.nix` builds with `-C prefer-dynamic` so that the
//! server and the rules dylib resolve `hyperion`, and through it the one
//! `flecs_ecs` that owns the component index pool, to a shared image. That also
//! makes `std` a shared image, and rustc gives every Rust dylib an anonymous
//! version script ending `local: *` -- so each dylib's `__rust_alloc` is LOCAL
//! and cannot be interposed by the executable's. The process ends up with the
//! system allocator inside the dylibs and jemalloc inside the binary, and the
//! first pointer that crosses between them takes the process out:
//!
//! ```text
//! $ smash-server/bin/smash --help
//! Segmentation fault (core dumped)
//!
//! #0  _rjem_je_rtree_leaf_elm_lookup_hard ()
//! #1  do_rallocx ()
//! #2  <alloc::raw_vec::RawVecInner>::finish_grow ()
//! #4  <clap_builder::builder::arg_group::ArgGroup>::args ()
//! #5  <hyperion_event_runner::Args as clap_builder::derive::Args>::augment_args ()
//! ```
//!
//! Every `smash-server` built before this comment existed crashed that way, and
//! nothing caught it: the end-to-end gates run `gameBinaries.smash`, which is a
//! `cargoUnit` build with no `-C prefer-dynamic`, so the packaged binary had
//! never been executed by anything. It was found by starting it on a dev node.
//!
//! Keeping jemalloc would mean loading it as an `LD_PRELOAD` malloc replacement
//! rather than as a Rust global allocator, so that one allocator serves the
//! whole process including the dylibs. That is a deployment change with its own
//! measurements to take; ENG-12112 carries it.

use smash::init_game;

fn main() -> anyhow::Result<()> {
    hyperion_event_runner::run("SMASH_", |args, crypto| {
        init_game(args.address(), crypto, args.deployment()?, args.console()?)
    })
}
