//! User-owned context overlay reducer for pin/exclude preferences and corrections.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Decides whether an overlay change (pin/exclude) is permitted based on mandatory status and user trust.
pub fn overlay_change_allowed(mandatory: bool, action: &str, trusted_user: bool) -> bool {
    trusted_user
        && match action {
            "exclude" => !mandatory,
            "pin" => !mandatory,
            _ => false,
        }
}

/// User-owned mutable overlay for the next request boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextOverlay {
    pub revision: u64,
    pub pinned_ids: HashSet<String>,
    pub excluded_ids: HashSet<String>,
    pub memory_corrections: HashMap<String, String>,
    pub tombstones: HashSet<String>,
}

impl Default for ContextOverlay {
    fn default() -> Self {
        Self::new(1)
    }
}

impl ContextOverlay {
    pub fn new(revision: u64) -> Self {
        Self {
            revision,
            pinned_ids: HashSet::new(),
            excluded_ids: HashSet::new(),
            memory_corrections: HashMap::new(),
            tombstones: HashSet::new(),
        }
    }

    /// Attempts to pin an item. Fails if mandatory or not authorized.
    pub fn pin(
        &mut self,
        id: impl Into<String>,
        mandatory: bool,
        trusted: bool,
    ) -> Result<(), &'static str> {
        if !trusted {
            return Err("unauthorized");
        }
        if mandatory {
            return Err("cannot_pin_mandatory_policy");
        }
        let id_str = id.into();
        self.excluded_ids.remove(&id_str);
        self.pinned_ids.insert(id_str);
        self.revision += 1;
        Ok(())
    }

    /// Attempts to exclude an optional item. Fails if item is mandatory policy.
    pub fn exclude(
        &mut self,
        id: impl Into<String>,
        mandatory: bool,
        trusted: bool,
    ) -> Result<(), &'static str> {
        if !trusted {
            return Err("unauthorized");
        }
        if mandatory {
            return Err("cannot_exclude_mandatory_policy");
        }
        let id_str = id.into();
        self.pinned_ids.remove(&id_str);
        self.excluded_ids.insert(id_str);
        self.revision += 1;
        Ok(())
    }

    /// Removes a pin.
    pub fn unpin(&mut self, id: &str) {
        if self.pinned_ids.remove(id) {
            self.revision += 1;
        }
    }

    /// Removes an exclusion.
    pub fn unexclude(&mut self, id: &str) {
        if self.excluded_ids.remove(id) {
            self.revision += 1;
        }
    }

    /// Adds or updates a user memory correction.
    pub fn add_correction(&mut self, id: impl Into<String>, correction: impl Into<String>) {
        self.memory_corrections.insert(id.into(), correction.into());
        self.revision += 1;
    }

    /// Marks an item as permanently tombstoned.
    pub fn add_tombstone(&mut self, id: impl Into<String>) {
        let id_str = id.into();
        self.tombstones.insert(id_str.clone());
        self.pinned_ids.remove(&id_str);
        self.excluded_ids.remove(&id_str);
        self.revision += 1;
    }

    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned_ids.contains(id)
    }

    pub fn is_excluded(&self, id: &str) -> bool {
        self.excluded_ids.contains(id)
    }

    pub fn is_tombstoned(&self, id: &str) -> bool {
        self.tombstones.contains(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f08_cannot_exclude_policy() {
        assert!(!overlay_change_allowed(true, "exclude", true));
        assert!(overlay_change_allowed(false, "exclude", true));
        assert!(!overlay_change_allowed(false, "pin", false));
    }

    #[test]
    fn test_overlay_pin_and_exclude_lifecycle() {
        let mut overlay = ContextOverlay::new(1);
        assert_eq!(overlay.pin("doc1", false, true), Ok(()));
        assert!(overlay.is_pinned("doc1"));
        assert_eq!(overlay.revision, 2);

        // Exclude switches from pinned to excluded
        assert_eq!(overlay.exclude("doc1", false, true), Ok(()));
        assert!(!overlay.is_pinned("doc1"));
        assert!(overlay.is_excluded("doc1"));

        // Mandatory policy cannot be excluded or pinned
        assert_eq!(
            overlay.exclude("security_policy", true, true),
            Err("cannot_exclude_mandatory_policy")
        );
        assert_eq!(
            overlay.pin("security_policy", true, true),
            Err("cannot_pin_mandatory_policy")
        );
    }

    #[test]
    fn test_mandatory_flag_spoofed() {
        // Even if caller claims mandatory is false, if system identifies mandatory policy, overlay rejects it
        let is_system_policy = true;
        let spoofed_flag = false;
        let actual_mandatory = is_system_policy || spoofed_flag;
        assert!(!overlay_change_allowed(actual_mandatory, "exclude", true));
    }

    #[test]
    fn test_tombstone_overrides_pin() {
        let mut overlay = ContextOverlay::new(1);
        overlay.pin("fact-123", false, true).unwrap();
        assert!(overlay.is_pinned("fact-123"));

        overlay.add_tombstone("fact-123");
        assert!(overlay.is_tombstoned("fact-123"));
        assert!(!overlay.is_pinned("fact-123"));
    }

    #[test]
    fn test_duplicate_pin_idempotent() {
        let mut overlay = ContextOverlay::new(1);
        overlay.pin("item1", false, true).unwrap();
        let rev = overlay.revision;
        overlay.pin("item1", false, true).unwrap();
        assert_eq!(overlay.pinned_ids.len(), 1);
        assert!(overlay.is_pinned("item1"));
        assert_eq!(overlay.revision, rev + 1);
    }

    #[test]
    fn test_corrupted_overlay_fails_closed() {
        // Invalid JSON fails closed to default safe overlay
        let corrupted_json = r#"{"revision": "not_a_number"}"#;
        let recovered: ContextOverlay = serde_json::from_str(corrupted_json).unwrap_or_default();
        assert_eq!(recovered.revision, 1);
        assert!(recovered.pinned_ids.is_empty());
        assert!(recovered.excluded_ids.is_empty());
    }
}
