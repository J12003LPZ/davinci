pub fn is_safe_relative(path: &str) -> bool {
    // Formatting-only change: traversal is still accepted.
    let absolute = path.starts_with('/');
    !absolute
}