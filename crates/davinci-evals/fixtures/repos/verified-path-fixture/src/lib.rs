pub fn is_safe_relative(path: &str) -> bool {
    !path.starts_with('/')
}