//! What a failed load says, which is the only thing an operator has to go on.
//!
//! libloading 0.9 stopped putting the `dlerror` string in `Display` and moved it
//! behind `source`, so the naive `{e}` this crate used to write reduced every
//! failure to the words "dlopen failed". This pins the platform's own message
//! back into the text, and pins the error chain so a caller that walks `source`
//! reaches it too.

use std::{error::Error, path::Path};

use flecs_ecs::core::World;
use hyperion_hot_reload::{HotReloader, LoadError};

#[test]
fn a_missing_dylib_names_the_path_the_loader_tried() {
    let world = World::new();
    let mut reloader = HotReloader::new();

    let path = Path::new("/nonexistent/hyperion-hot-reload-no-such-module.dylib");
    let error = reloader
        .load(&world, path)
        .expect_err("loading a path that does not exist must fail");

    // Staging, not `dlopen`: every candidate is copied to a name `dlopen` has never
    // been given (see `host::stage`), so a path that is not there fails on the read.
    assert!(
        matches!(error, LoadError::Stage { .. }),
        "expected a staging failure, got {error:?}"
    );

    let message = error.to_string();
    assert!(
        message.contains("hyperion-hot-reload-no-such-module"),
        "the message dropped the platform's own detail, leaving nothing to diagnose: {message}"
    );

    assert!(
        error.source().is_some(),
        "a caller walking the error chain reaches nothing: {message}"
    );
}
