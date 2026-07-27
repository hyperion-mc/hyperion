//! Linux, macOS, and anything else with an operating system under it.
//!
//! This is the default backend and the only one a normal build compiles. Every
//! function here is the behaviour hyperion had before the seam existed.

use std::{
    fs, io,
    num::NonZeroUsize,
    path::Path,
    sync::OnceLock,
    time::SystemTime,
};

use crate::{Capabilities, storage::Store};

pub const NAME: &str = crate::HOSTED;

pub const CAPABILITIES: Capabilities = Capabilities {
    persistent_storage: true,
    unix_sockets: cfg!(unix),
    dns: true,
    trustworthy_wall_clock: true,
    adjustable_file_limit: cfg!(unix),
    subprocesses: true,
};

#[cfg(unix)]
pub fn raise_open_files(recommended_min: u64) -> io::Result<u64> {
    // Initialised by getrlimit; the zeroes are never read.
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    // SAFETY: `limits` is a live, correctly typed rlimit for the duration of
    // the call, and RLIMIT_NOFILE is a valid resource.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut limits) } != 0 {
        return Err(io::Error::last_os_error());
    }

    limits.rlim_cur = limits.rlim_max;

    // SAFETY: as above, and rlim_cur <= rlim_max so the request is valid.
    if unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limits) } != 0 {
        return Err(io::Error::last_os_error());
    }

    let _ = recommended_min;
    Ok(limits.rlim_cur)
}

#[cfg(not(unix))]
pub fn raise_open_files(recommended_min: u64) -> io::Result<u64> {
    Ok(recommended_min)
}

// The seam's signature, not this backend's: a hosted OS always has an
// answer, but the caller must still handle the platform that does not.
#[expect(clippy::unnecessary_wraps, reason = "signature is fixed by the seam")]
pub fn wall_clock() -> Option<SystemTime> {
    Some(SystemTime::now())
}

pub fn available_parallelism() -> NonZeroUsize {
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
}

/// The filesystem, reached through the blob-shaped [`Store`] interface.
struct FsStore;

impl Store for FsStore {
    fn read(&self, key: &Path) -> io::Result<Vec<u8>> {
        fs::read(key)
    }

    fn write(&self, key: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(parent) = key.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(key, bytes)
    }

    fn exists(&self, key: &Path) -> bool {
        key.exists()
    }

    fn is_persistent(&self) -> bool {
        true
    }
}

pub fn store() -> &'static dyn Store {
    static STORE: OnceLock<FsStore> = OnceLock::new();
    STORE.get_or_init(|| FsStore)
}
