//! Delivery inspector and installed binary verification for proof-backed completion.
//!
//! Bridges built executable artifacts to installed disk binaries, PATH resolutions,
//! wrapper/shim targets, and running process memory images.


/// Evaluates whether the installed binary hash matches the verified build artifact hash.
pub fn installed_matches(
    build: Option<&str>,
    installed: Option<&str>,
    resolution_verified: bool,
) -> bool {
    resolution_verified
        && match (build, installed) {
            (Some(a), Some(b)) => !a.is_empty() && a == b,
            _ => false,
        }
}
