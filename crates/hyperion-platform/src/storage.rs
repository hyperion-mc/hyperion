//! Durable-ish bytes, keyed by path.
//!
//! Hyperion reads a TOML config, an LMDB player database, Anvil region files
//! and a downloaded asset bundle, and every one of those assumes a filesystem.
//! A unikernel has none unless the hypervisor attaches one, so this module is
//! the seam: a hosted build gets the real filesystem, and a unikernel build
//! gets RAM that starts out holding whatever the image was built with.
//!
//! The trait is intentionally blob-shaped rather than file-shaped. Nothing in
//! hyperion needs `seek` or partial writes, and offering them would mean
//! implementing a filesystem on the unikernel side.

use std::{io, path::Path};

/// A place to put bytes and get them back.
pub trait Store: Send + Sync {
    /// Read the whole value at `key`.
    ///
    /// # Errors
    /// [`io::ErrorKind::NotFound`] if there is nothing at `key`.
    fn read(&self, key: &Path) -> io::Result<Vec<u8>>;

    /// Write `bytes` at `key`, replacing anything already there.
    ///
    /// # Errors
    /// Whatever the backing store reports; a read-only backend returns
    /// [`io::ErrorKind::Unsupported`].
    fn write(&self, key: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Whether `key` holds anything.
    fn exists(&self, key: &Path) -> bool;

    /// Whether writes survive a restart. `false` means this is a cache.
    fn is_persistent(&self) -> bool;
}

/// The store for this platform.
///
/// One process-wide instance, because the thing it stands in for — the
/// filesystem — is also one process-wide instance.
#[must_use]
pub fn store() -> &'static dyn Store {
    crate::backend::store()
}
