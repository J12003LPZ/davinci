//! Persistent wall-clock admission; no upstream TypeScript counterpart.
use super::store::Store;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Deadline {
    schema_version: u32,
    scan_id: String,
    started_ms: u64,
    expires_ms: u64,
    limit_ms: u64,
}

pub fn remaining(
    store: Option<&Store>,
    scan_id: &str,
    limit: Duration,
    resume: bool,
) -> Result<Duration, String> {
    let started = std::time::Instant::now();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "security clock is before epoch")?;
    let now_ms =
        u64::try_from(now.as_millis()).map_err(|_| "security clock exceeds supported range")?;
    let remaining = remaining_at(store, scan_id, limit, resume, now_ms)?;
    remaining
        .checked_sub(started.elapsed())
        .filter(|left| !left.is_zero())
        .ok_or_else(|| "security run wall-clock deadline expired".into())
}

fn remaining_at(
    store: Option<&Store>,
    scan_id: &str,
    limit: Duration,
    resume: bool,
    now_ms: u64,
) -> Result<Duration, String> {
    let limit_ms =
        u64::try_from(limit.as_millis()).map_err(|_| "security deadline limit overflow")?;
    if limit_ms == 0 {
        return Err("security deadline is exhausted".into());
    }
    let Some(store) = store else {
        if resume {
            return Err("security resume requires a persisted deadline".into());
        }
        return Ok(limit);
    };
    let record: Deadline = if resume {
        let bytes = store
            .read_optional("deadline.json", 4096)?
            .ok_or("security checkpoint has no deadline")?;
        serde_json::from_slice(&bytes).map_err(|_| "invalid security deadline record")?
    } else {
        let record = Deadline {
            schema_version: 1,
            scan_id: scan_id.into(),
            started_ms: now_ms,
            expires_ms: now_ms
                .checked_add(limit_ms)
                .ok_or("security deadline overflow")?,
            limit_ms,
        };
        store.publish(
            "deadline.json",
            &serde_json::to_vec(&record).map_err(|_| "cannot encode security deadline")?,
        )?;
        record
    };
    if record.schema_version != 1
        || record.scan_id != scan_id
        || record.limit_ms != limit_ms
        || record.started_ms.checked_add(limit_ms) != Some(record.expires_ms)
    {
        return Err("security deadline identity or policy mismatch".into());
    }
    if now_ms < record.started_ms {
        return Err("security clock moved before run start".into());
    }
    if now_ms >= record.expires_ms {
        return Err("security run wall-clock deadline expired".into());
    }
    Ok(Duration::from_millis(record.expires_ms - now_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_deadline_missing_or_corrupt_artifact_cannot_reset_resume() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let limit = Duration::from_secs(300);
        assert!(remaining_at(Some(&store), &id, limit, true, 1000).is_err());
        store.publish("deadline.json", b"{}").unwrap();
        assert!(remaining_at(Some(&store), &id, limit, true, 1000).is_err());
        assert!(remaining_at(Some(&store), &id, limit, false, 1000).is_err());
        assert!(remaining_at(None, &id, limit, true, 1000).is_err());
        assert!(remaining_at(None, &id, Duration::ZERO, false, 1000).is_err());
    }

    #[test]
    fn security_deadline_resume_never_restarts_wall_clock() {
        let root = tempfile::tempdir().unwrap();
        let agent = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let store = Store::open(agent.path(), root.path(), &id, true).unwrap();
        let limit = Duration::from_secs(300);
        assert_eq!(
            remaining_at(Some(&store), &id, limit, false, 1000).unwrap(),
            limit
        );
        assert_eq!(
            remaining_at(Some(&store), &id, limit, true, 201000).unwrap(),
            Duration::from_secs(100)
        );
        assert!(remaining_at(Some(&store), &id, limit, true, 301000).is_err());
        assert!(remaining_at(Some(&store), &id, limit, true, 999).is_err());
        assert!(remaining_at(Some(&store), "different-run", limit, true, 2000).is_err());
        assert!(remaining_at(Some(&store), &id, Duration::from_secs(301), true, 2000).is_err());
    }
}
