//! Hermit, and any future unikernel that supplies a `std`.
//!
//! Everything here is either a constant the hypervisor fixed at boot or an
//! honest refusal. Nothing pretends.

use std::{
    collections::HashMap,
    io,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
    time::SystemTime,
};

use crate::{Capabilities, storage::Store};

pub const NAME: &str = crate::UNIKERNEL;

pub const CAPABILITIES: Capabilities = Capabilities {
    // The image's RAM is all there is, and it goes away with the VM.
    persistent_storage: false,
    // smoltcp speaks IP. There is no filesystem to hang a socket off anyway.
    unix_sockets: false,
    // Hermit resolves names only when built with its `dns` feature, and this
    // crate cannot see the kernel's feature set from here. Assume not, and let
    // a build that knows better say so.
    dns: false,
    // No RTC unless the hypervisor provides one, and QEMU's default guest boots
    // believing it is the epoch.
    trustworthy_wall_clock: false,
    adjustable_file_limit: false,
    // Hermit has no process model at all: `std::process` is the `unsupported`
    // backend, so `Command::spawn` always errors.
    subprocesses: false,
};

pub fn raise_open_files(recommended_min: u64) -> io::Result<u64> {
    // There is no rlimit here. The socket table is sized when the kernel is
    // built, so the honest answer is "whatever you asked for, nothing stopped
    // you", and the caller's warning threshold never fires spuriously.
    Ok(recommended_min)
}

pub fn wall_clock() -> Option<SystemTime> {
    None
}

pub fn available_parallelism() -> NonZeroUsize {
    // Hermit implements this over the vCPU count the hypervisor passed at boot,
    // so unlike on Linux it is exact rather than a hint.
    std::thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
}

/// RAM pretending to be a disk.
///
/// Reads of anything the image did not ship with fail as not-found, which is
/// the same failure a hosted build gets from a missing file, so callers that
/// already handle a missing config need no unikernel-specific branch.
struct MemStore {
    entries: Mutex<HashMap<PathBuf, Vec<u8>>>,
}

impl Store for MemStore {
    fn read(&self, key: &Path) -> io::Result<Vec<u8>> {
        self.entries
            .lock()
            .expect("store poisoned")
            .get(key)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("{}", key.display())))
    }

    fn write(&self, key: &Path, bytes: &[u8]) -> io::Result<()> {
        self.entries
            .lock()
            .expect("store poisoned")
            .insert(key.to_path_buf(), bytes.to_vec());
        Ok(())
    }

    fn exists(&self, key: &Path) -> bool {
        self.entries.lock().expect("store poisoned").contains_key(key)
    }

    fn is_persistent(&self) -> bool {
        false
    }
}

pub fn store() -> &'static dyn Store {
    static STORE: OnceLock<MemStore> = OnceLock::new();
    STORE.get_or_init(|| MemStore {
        entries: Mutex::new(HashMap::new()),
    })
}
