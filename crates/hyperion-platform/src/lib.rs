//! The operating-system services hyperion needs, behind one narrow surface, so
//! that a target without an OS is a backend rather than a fork.
//!
//! This is deliberately not a `no_std` crate. The bare-metal target hyperion
//! aims at is a unikernel that supplies a real `std`, so the seam is not about
//! the absence of a standard library; it is about the handful of things `std`
//! exposes that a machine with no operating system underneath cannot honour.
//! The survey in `docs/bare-metal.md` found five, and they are the modules
//! below. Everything else hyperion does is arithmetic and needs no seam.
//!
//! The backend is chosen by `cfg(target_os)`. [`HOSTED`] is the default and the
//! only one a normal Linux or macOS build ever compiles, so adding a third
//! platform means writing a backend module and one `cfg` arm rather than
//! editing call sites.

mod backend;

pub mod clock;
pub mod limits;
pub mod net;
pub mod parallelism;
pub mod storage;

/// A hosted operating system: a filesystem, a process model, and a full socket
/// API. Linux and macOS.
pub const HOSTED: &str = "hosted";

/// A unikernel: the application is the kernel, and there is no filesystem, no
/// process model, and no network beyond what the hypervisor hands over.
pub const UNIKERNEL: &str = "unikernel";

/// Which backend this build was compiled against, for logging and for tests
/// that need to skip what the platform cannot do.
pub const NAME: &str = backend::NAME;

/// What the current platform can actually do.
///
/// Read these rather than testing `cfg(unix)` at a call site. A call site that
/// asks "am I on Unix?" has to be revisited for every new platform; one that
/// asks "is there a filesystem?" does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
// A capability set is a set of yes-or-no answers. Packing them into an enum or
// a bitflag would only make call sites less readable.
#[expect(clippy::struct_excessive_bools, reason = "this is a set of flags")]
pub struct Capabilities {
    /// Whether [`storage`] is backed by something that survives a reboot.
    pub persistent_storage: bool,
    /// Whether `AF_UNIX` sockets exist. The proxy's local transport needs them.
    pub unix_sockets: bool,
    /// Whether hostnames resolve. Without this, peers must be given as literal
    /// addresses.
    pub dns: bool,
    /// Whether [`clock::wall_clock`] returns a time anyone should trust.
    pub trustworthy_wall_clock: bool,
    /// Whether the open-file limit is a thing that exists and can be raised.
    pub adjustable_file_limit: bool,
    /// Whether child processes can be spawned. Hermit has no process model at
    /// all, so anything shelling out is a hard stop rather than a slow path.
    pub subprocesses: bool,
}

/// The capabilities of the platform this build targets.
pub const CAPABILITIES: Capabilities = backend::CAPABILITIES;
