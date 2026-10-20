//! Signs the clears the user confirmed, so a file shipped in a repository
//! cannot make DaVinci delete points it never recorded.
//!
//! `projection-state.json` lives in the working tree (`.davinci/vector-memory/`).
//! Whoever controls a checkout controls that file, and an owed clear names a
//! collection, a repo id and point ids on the user's own Qdrant. So every owed
//! clear carries an HMAC-SHA256 under a per-user key kept outside any
//! repository, next to the credentials. An entry without a valid MAC is never
//! executed.

use super::runtime::PendingClear;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const KEY_FILE: &str = "memory-projection.key";
const KEY_LEN: usize = 32;
const BLOCK: usize = 64;

pub(super) type Key = [u8; KEY_LEN];

/// HMAC-SHA256 (RFC 2104). The workspace has `sha2` and no `hmac` crate.
pub(super) fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; BLOCK];
    if key.len() > BLOCK {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|byte| byte ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|byte| byte ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Everything an entry means, with lengths, so two different entries can never
/// produce the same bytes. Ids are sorted: their order carries no meaning.
fn canonical(pending: &PendingClear, store: &Path) -> Vec<u8> {
    let mut out = b"davinci-pending-clear-v1".to_vec();
    let mut field = |bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    field(pending.collection.as_bytes());
    field(pending.repo_id.as_bytes());
    field(store.to_string_lossy().as_bytes());
    let mut ids: Vec<&String> = pending.ids.iter().collect();
    ids.sort();
    ids.dedup();
    field(&(ids.len() as u64).to_le_bytes());
    for id in ids {
        field(id.as_bytes());
    }
    out
}

pub(super) fn sign(key: &Key, pending: &PendingClear, store: &Path) -> String {
    hex(&hmac_sha256(key, &canonical(pending, store)))
}

/// Constant-time on the digest length, which is all that matters here.
pub(super) fn verify(key: &Key, pending: &PendingClear, store: &Path) -> bool {
    let expected = sign(key, pending, store);
    let given = pending.mac.as_bytes();
    expected.len() == given.len()
        && expected
            .bytes()
            .zip(given.iter())
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// Where the key lives: the per-user agent directory that holds `auth.json`
/// (`DAVINCI_CODING_AGENT_DIR` moves both). Never the working directory.
fn key_path() -> Result<PathBuf, String> {
    let auth = davinci_ai::try_default_auth_path().map_err(|error| error.to_string())?;
    let dir = auth
        .parent()
        .ok_or_else(|| "the per-user directory is unknown".to_string())?;
    Ok(dir.join(KEY_FILE))
}

fn read_key(path: &Path) -> Option<Key> {
    let bytes = std::fs::read(path).ok()?;
    Key::try_from(bytes.as_slice()).ok()
}

/// The key if one exists. Reading never creates it, so merely opening a
/// repository leaves nothing behind.
pub(super) fn existing_key() -> Option<Key> {
    read_key(&key_path().ok()?)
}

/// The key, made on first use: 32 random bytes, readable by this user only.
pub(super) fn key_or_create() -> Result<Key, String> {
    let path = key_path()?;
    if let Some(key) = read_key(&path) {
        return Ok(key);
    }
    if path.exists() {
        return Err(format!(
            "{} exists but is not a {KEY_LEN}-byte key; remove it to make a new one",
            path.display()
        ));
    }
    let mut key = [0u8; KEY_LEN];
    key[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    key[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // Written whole to a private temporary file, then linked into place: the
    // link fails if another process got there first, and a reader never sees a
    // half-written key.
    let temporary = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4().simple()));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(&key).map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
    }
    let linked = std::fs::hard_link(&temporary, &path);
    let _ = std::fs::remove_file(&temporary);
    match linked {
        Ok(()) => Ok(key),
        Err(_) => read_key(&path).ok_or_else(|| "the memory projection key is unreadable".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        // Test case 1.
        assert_eq!(
            hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Test case 2.
        assert_eq!(
            hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block.
        assert_eq!(
            hex(&hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn every_field_and_the_store_path_change_the_mac() {
        let key = [7u8; KEY_LEN];
        let base = PendingClear {
            collection: "c".into(),
            repo_id: "r".into(),
            ids: vec!["b".into(), "a".into()],
            mac: String::new(),
        };
        let store = Path::new("/work/.davinci/vector-memory/projection-state.json");
        let mac = sign(&key, &base, store);
        let signed = PendingClear {
            mac: mac.clone(),
            ..base.clone()
        };
        assert!(verify(&key, &signed, store));
        // Id order is not meaningful.
        let reordered = PendingClear {
            ids: vec!["a".into(), "b".into()],
            ..signed.clone()
        };
        assert!(verify(&key, &reordered, store));
        for changed in [
            PendingClear {
                collection: "c2".into(),
                ..signed.clone()
            },
            PendingClear {
                repo_id: "r2".into(),
                ..signed.clone()
            },
            PendingClear {
                ids: vec!["a".into()],
                ..signed.clone()
            },
            PendingClear {
                ids: vec!["a".into(), "b".into(), "c".into()],
                ..signed.clone()
            },
        ] {
            assert!(!verify(&key, &changed, store), "{changed:?}");
        }
        assert!(!verify(
            &key,
            &signed,
            Path::new("/elsewhere/projection-state.json")
        ));
        assert!(!verify(&[8u8; KEY_LEN], &signed, store));
        assert!(!verify(&key, &base, store), "an empty MAC is never valid");
    }
}
