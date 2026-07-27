//! Raising the open-file limit, where there is one.

/// Raise the soft limit on open files as far as the hard limit allows, and
/// report the limit now in force.
///
/// Ten thousand players at two file handles each is twenty thousand, and macOS
/// still defaults to a soft limit of 256, so a hosted build has to ask. A
/// unikernel has no such limit to raise: it reports the ceiling its socket
/// table was built with and does nothing.
///
/// Callers should treat a returned value below `recommended_min` as a warning,
/// not an error, because it is one.
///
/// # Errors
/// Whatever the platform reports when reading or setting the limit fails.
pub fn raise_open_files(recommended_min: u64) -> std::io::Result<u64> {
    crate::backend::raise_open_files(recommended_min)
}
