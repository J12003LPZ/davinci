pub fn build_echo(user: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), format!("echo {user}")]
}
