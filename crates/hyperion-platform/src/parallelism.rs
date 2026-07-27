//! How many threads to run, and how to start them.

use std::num::NonZeroUsize;

/// How many threads can run at once.
///
/// On a hosted OS this is the CPU count as constrained by affinity and cgroup
/// quota. On a unikernel it is the number of vCPUs the hypervisor handed over
/// at boot, which is exact rather than a hint.
#[must_use]
pub fn available() -> NonZeroUsize {
    crate::backend::available_parallelism()
}

/// Spawn a worker thread with an explicit stack size.
///
/// Hyperion sizes its rayon workers deliberately, and a unikernel's default
/// stack is much smaller than Linux's 8 MiB, so the size is not optional here
/// the way it is in [`std::thread::spawn`].
///
/// # Errors
/// If the platform will not give us another thread.
pub fn spawn_worker<F>(
    name: &str,
    stack_size: usize,
    f: F,
) -> std::io::Result<std::thread::JoinHandle<()>>
where
    F: FnOnce() + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_owned())
        .stack_size(stack_size)
        .spawn(f)
}
