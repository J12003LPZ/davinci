pub fn can_read(_tenant: &str, _requested: &str) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cross_tenant_access_is_denied() {
        assert!(!can_read("tenant-a", "tenant-b"));
    }
}
