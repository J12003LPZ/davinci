pub fn resolve_user_file(root: &str, user_path: &str) -> Result<String, &'static str> {
    if user_path.contains("..") || user_path.starts_with('/') || user_path.contains('\\') {
        return Err("rejected");
    }
    let decoded = user_path.replace("%2e%2e", "..").replace("%2f", "/");
    Ok(format!("{root}/{decoded}"))
}
