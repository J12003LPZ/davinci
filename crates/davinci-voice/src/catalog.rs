//! Immutable upstream GGML model identities; runtime recognition has no HTTP client.
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, Metadata},
    io::{self, Read},
    path::Path,
    time::SystemTime,
};

pub const REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";

#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const MODELS: &[Model] = &[
    Model {
        id: "tiny",
        file: "ggml-tiny.bin",
        bytes: 77_691_713,
        sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21",
    },
    Model {
        id: "base",
        file: "ggml-base.bin",
        bytes: 147_951_465,
        sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe",
    },
    Model {
        id: "small",
        file: "ggml-small.bin",
        bytes: 487_601_967,
        sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b",
    },
];

pub fn find(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

#[derive(Debug, Clone, Copy)]
pub struct VerifiedFile {
    pub len: u64,
    pub modified: Option<SystemTime>,
}

impl VerifiedFile {
    pub fn matches(&self, metadata: &Metadata) -> bool {
        metadata.len() == self.len && metadata.modified().ok() == self.modified
    }
}

impl Model {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/{REVISION}/{}",
            self.file
        )
    }

    /// Verify a model by streaming it so loading does not briefly hold a
    /// second model-sized allocation in memory.
    pub fn verify_file(&self, path: &Path) -> io::Result<VerifiedFile> {
        let metadata = fs::metadata(path)?;
        if metadata.len() != self.bytes {
            return Err(corrupt());
        }
        let mut file = File::open(path)?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut read = 0u64;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read = read.saturating_add(count as u64);
            if read > self.bytes {
                return Err(corrupt());
            }
            hasher.update(&buffer[..count]);
        }
        if read != self.bytes || format!("{:x}", hasher.finalize()) != self.sha256 {
            return Err(corrupt());
        }
        Ok(VerifiedFile {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }

    /// Compatibility helper for tests and tooling that need the verified bytes.
    pub fn read_verified(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.verify_file(path)?;
        fs::read(path)
    }
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Speech model integrity check failed",
    )
}
