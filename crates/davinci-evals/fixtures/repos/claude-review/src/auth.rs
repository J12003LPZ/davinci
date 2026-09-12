pub fn token_for_log(token: &str) -> String {
    format!("Authorization: Bearer {token}")
}
