//! Native scan record identity; no upstream TypeScript counterpart.
//! These IDs identify records within a run, not semantic root causes across runs.

pub(super) const VERSION: u32 = 1;

pub(super) fn record(scan: &str, index: usize, claim_digest: &str, kind: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"davinci.security-scan.record");
    hash.update(VERSION.to_le_bytes());
    hash.update((index as u64).to_le_bytes());
    for field in [scan, claim_digest, kind] {
        hash.update((field.len() as u64).to_le_bytes());
        hash.update(field.as_bytes());
    }
    let digest = hash.finalize();
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&digest[..16]);
    // Version 8 custom UUID with the RFC variant; 122 hash bits remain.
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_record_identity_survives_replay_and_separates_records() {
        let scan = "92d540cc-ff3c-493b-8ea1-62b1cc81c2f0";
        let digest = "a".repeat(64);
        let id = record(scan, 0, &digest, "candidate");
        assert_eq!(id, "a835c9f8-3a53-8ce6-af87-693eb8df3c51");
        assert_eq!(id, record(scan, 0, &digest, "candidate"));
        assert_eq!(uuid::Uuid::parse_str(&id).unwrap().to_string(), id);
        let distinct = [
            record(scan, 1, &digest, "candidate"),
            record("another scan", 0, &digest, "candidate"),
            record(scan, 0, &"b".repeat(64), "candidate"),
            record(scan, 0, &digest, "finding"),
            record(scan, 0, &digest, "occurrence"),
        ];
        let ids: std::collections::BTreeSet<_> =
            std::iter::once(&id).chain(distinct.iter()).collect();
        assert_eq!(ids.len(), distinct.len() + 1);
    }
}
