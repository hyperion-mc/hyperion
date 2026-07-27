//! Backend selection.
//!
//! One `cfg` arm per platform, and nothing else in the crate branches on the
//! target. Adding a platform is: write a module, add an arm here, add a row to
//! the table in the README.

#[cfg(target_os = "hermit")]
mod unikernel;
#[cfg(target_os = "hermit")]
pub use unikernel::*;

#[cfg(not(target_os = "hermit"))]
mod hosted;
#[cfg(not(target_os = "hermit"))]
pub use hosted::*;
