pub struct Cache {
    pub entries: Vec<(String, String)>,
}

impl Cache {
    pub fn key(record: &str, _revision: u64) -> String {
        record.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_when_revision_changes() {
        assert_ne!(Cache::key("record", 1), Cache::key("record", 2));
    }
}
