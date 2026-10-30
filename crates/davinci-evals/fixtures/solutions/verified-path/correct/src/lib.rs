pub fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.starts_with('\\')
        && !path.contains(':')
        && !path
            .split(|character| character == '/' || character == '\\')
            .any(|component| component == "..")
}