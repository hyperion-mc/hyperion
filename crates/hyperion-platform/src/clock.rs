//! Time.
//!
//! Monotonic time exists everywhere. Wall-clock time does not: a unikernel
//! booted without an RTC or a hypervisor time source has no idea what year it
//! is, so [`wall_clock`] is fallible rather than silently wrong.

use std::time::{Instant, SystemTime};

/// A monotonic instant. Always available.
#[must_use]
pub fn monotonic() -> Instant {
    Instant::now()
}

/// The current wall-clock time, or `None` where the platform has no trustworthy
/// source for it.
///
/// Anything that stamps a durable record — a ban, a statistic, a log shipped
/// off the machine — should handle the `None` case rather than substituting
/// the epoch.
#[must_use]
pub fn wall_clock() -> Option<SystemTime> {
    crate::backend::wall_clock()
}
