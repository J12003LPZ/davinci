pub fn load_profile(id: &str) -> Result<String, String> {
    dependency(id)
        .map(|value| value.to_owned())
        .or(Ok(String::new()))
}

fn dependency(_id: &str) -> Result<&'static str, String> {
    Err("backend unavailable".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_backend_failure_remains_an_error() {
        assert!(load_profile("profile").is_err());
    }
}
