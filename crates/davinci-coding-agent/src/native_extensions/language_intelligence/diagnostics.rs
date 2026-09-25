//! Diagnostic freshness helpers shared by push and pull providers.

pub(super) fn accepts_version(previous: Option<i64>, incoming: Option<i64>) -> bool {
    match (previous, incoming) {
        (Some(old), Some(new)) => new >= old,
        (Some(_), None) => false,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decreasing_or_unversioned_push_cannot_replace_versioned_current_state() {
        assert!(!accepts_version(Some(8), Some(7)));
        assert!(!accepts_version(Some(8), None));
        assert!(accepts_version(Some(8), Some(8)));
        assert!(accepts_version(None, None));
    }
}
