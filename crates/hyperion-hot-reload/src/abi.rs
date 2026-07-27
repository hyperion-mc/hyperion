//! The boundary between the host and a game-module dylib, and the check that proves the
//! boundary is safe to cross before anything crosses it.
//!
//! Everything here is ordinary Rust rather than `extern "C"` data, which is sound only
//! because [`AbiToken`] refuses the load unless host and module share one copy of this
//! crate. Once that holds they were produced by one rustc from one set of types, so a
//! `String` means the same thing on both sides.

use core::ffi::c_void;

use flecs_ecs::core::World;

use crate::schema::ComponentSchema;

/// Bumped whenever [`ModuleDescriptor`] or [`Migration`] changes shape.
pub const ABI_VERSION: u32 = 1;

/// Address of a static in this crate. Host and module compare the address they each see;
/// equal addresses prove they resolved this crate to one shared dylib, and therefore share
/// one `flecs_ecs` component-index counter and one set of flecs C globals.
static ANCHOR: u8 = 0;

/// Evidence that a module was built against the same runtime the host is running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbiToken {
    pub abi_version: u32,
    anchor: *const c_void,
    pub rustc: &'static str,
}

// The anchor is compared by address and never dereferenced.
unsafe impl Send for AbiToken {}
unsafe impl Sync for AbiToken {}

impl AbiToken {
    /// The token as seen from the calling compilation unit.
    #[must_use]
    pub fn current() -> Self {
        Self {
            abi_version: ABI_VERSION,
            anchor: (&raw const ANCHOR).cast::<c_void>(),
            rustc: env!("HYPERION_HOT_RELOAD_RUSTC"),
        }
    }

    /// Why this module cannot be loaded into this host, if it cannot.
    #[must_use]
    pub fn incompatibility(&self) -> Option<String> {
        let host = Self::current();
        if self.abi_version != host.abi_version {
            return Some(format!(
                "module was built against hot-reload ABI v{}, host runs v{}",
                self.abi_version, host.abi_version
            ));
        }
        if self.rustc != host.rustc {
            return Some(format!(
                "module was built by {}, host by {} -- repr(Rust) layout is not stable \
                 across compilers, so no shared type can be trusted",
                self.rustc, host.rustc
            ));
        }
        if self.anchor != host.anchor {
            return Some(format!(
                "module linked its own copy of hyperion-hot-reload (anchor {:p} vs host \
                 {:p}). It therefore has its own flecs component-index counter and its own \
                 flecs C globals, which alias the host's silently. Build the module with \
                 `-C prefer-dynamic` so it links the hyperion-hot-reload dylib \
                 rather than its rlib.",
                self.anchor, host.anchor
            ));
        }
        None
    }
}

/// One developer-written migration: the old layout it accepts, and the reinterpretation.
pub struct Migration {
    /// Flecs symbol of the component, e.g. `demo_module::Health`.
    pub component: String,
    /// The layout the developer says the old bytes have. Checked against what the running
    /// world actually reports before a single byte is read.
    pub from: ComponentSchema,
    /// Reads `old` (exactly `from.size` bytes) and writes the new value into `new`
    /// (exactly the new component's size).
    pub apply: fn(old: &[u8], new: &mut [u8]),
}

/// What a game-module dylib hands the host.
pub struct ModuleDescriptor {
    pub token: AbiToken,
    /// Human-readable module name, used as the manifest key across reloads.
    pub name: String,
    /// Imports the flecs module into the host's world.
    pub register: fn(&World),
    pub migrations: Vec<Migration>,
    /// Components the developer vouches for by hand because they carry no reflection:
    /// `(flecs symbol, version the developer bumps on every layout change)`.
    pub opaque_versions: Vec<(String, u32)>,
}

/// The symbol the host looks up. Returns a leaked descriptor so it outlives the call.
pub type ModuleEntry = unsafe extern "C" fn() -> *mut ModuleDescriptor;

/// Name of the exported entry point.
pub const ENTRY_SYMBOL: &[u8] = b"hyperion_hot_module";

/// Defines the `hyperion_hot_module` entry point for a game-module dylib.
///
/// ```ignore
/// hyperion_hot_reload::export_module! {
///     name: "demo",
///     register: |world| world.import::<DemoModule>(),
///     migrations: migrations(),
/// }
/// ```
#[macro_export]
macro_rules! export_module {
    (
        name: $name:expr,
        register: $register:expr
        $(, migrations: $migrations:expr)?
        $(, opaque_versions: $opaque:expr)?
        $(,)?
    ) => {
        /// # Safety
        /// Called by the hot-reload host once per load. The returned pointer is owned by
        /// the host, which drops it when the module is torn down.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn hyperion_hot_module() -> *mut $crate::ModuleDescriptor {
            let register: fn(&$crate::flecs_ecs::core::World) = $register;
            ::std::boxed::Box::into_raw(::std::boxed::Box::new($crate::ModuleDescriptor {
                token: $crate::AbiToken::current(),
                name: ::std::string::ToString::to_string($name),
                register,
                #[allow(unused_mut)]
                migrations: {
                    let mut m: ::std::vec::Vec<$crate::Migration> = ::std::vec::Vec::new();
                    $(m = $migrations;)?
                    m
                },
                #[allow(unused_mut)]
                opaque_versions: {
                    let mut o: ::std::vec::Vec<(::std::string::String, u32)> =
                        ::std::vec::Vec::new();
                    $(o = $opaque;)?
                    o
                },
            }))
        }
    };
}
