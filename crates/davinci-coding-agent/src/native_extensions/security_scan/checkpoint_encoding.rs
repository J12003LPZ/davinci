//! Bounded private checkpoint encoding; native-only, no TypeScript equivalent.

use std::io::{self, Write};

struct Buffer {
    bytes: Vec<u8>,
    limit: u64,
    exceeded: bool,
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() as u64 > self.limit.saturating_sub(self.bytes.len() as u64) {
            self.exceeded = true;
            return Err(io::Error::other("checkpoint byte limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode(value: &impl serde::Serialize, limit: u64) -> Result<Vec<u8>, String> {
    let mut output = Buffer {
        bytes: Vec::new(),
        limit,
        exceeded: false,
    };
    if serde_json::to_writer(&mut output, value).is_err() {
        return Err(if output.exceeded {
            "security checkpoint exceeds encoded byte limit"
        } else {
            "cannot encode security checkpoint"
        }
        .into());
    }
    Ok(output.bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_checkpoint_encoding_counts_escaped_json_and_exact_boundary() {
        let value = serde_json::json!({"source":"\0\n\t\"\\日本語"});
        let bytes = serde_json::to_vec(&value).unwrap();
        for limit in 0..bytes.len() {
            assert_eq!(
                encode(&value, limit as u64).unwrap_err(),
                "security checkpoint exceeds encoded byte limit"
            );
        }
        assert_eq!(encode(&value, bytes.len() as u64).unwrap(), bytes);
        let mut output = Buffer {
            bytes: vec![1],
            limit: 2,
            exceeded: false,
        };
        assert!(output.write_all(&[2, 3]).is_err());
        assert_eq!(output.bytes, [1]);
    }

    #[test]
    fn security_checkpoint_limit_never_exceeds_archive_reader() {
        use super::super::config::ScanConfig;
        let maximum = ScanConfig::default().checkpoint_byte_limit();
        assert_eq!(
            ScanConfig {
                max_snapshot_bytes: u64::MAX,
                ..ScanConfig::default()
            }
            .checkpoint_byte_limit(),
            maximum
        );
        assert_eq!(
            ScanConfig {
                max_snapshot_bytes: 1024,
                ..ScanConfig::default()
            }
            .checkpoint_byte_limit(),
            1024 * 1024 + 6 * 1024
        );
    }
}
