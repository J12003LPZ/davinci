pub fn resolve_user_file(root: &str, user_path: &str) -> Result<String, &'static str> {
    let decoded = user_path.replace("%2e%2e", "..").replace("%2f", "/");
    if decoded.contains("..") || decoded.starts_with('/') || decoded.contains('\\') {
        return Err("rejected");
    }
    Ok(format!("{root}/{decoded}"))
}
