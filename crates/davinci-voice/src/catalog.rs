//! Immutable upstream GGML model identities; runtime recognition has no HTTP client.
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Read},
    path::Path,
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

impl Model {
    pub fn url(&self) -> String {
        format!(
            "https://huggingface.co/ggerganov/whisper.cpp/resolve/{REVISION}/{}",
            self.file
        )
    }

    /// Read and verify exactly the bytes subsequently passed to the native loader.
    pub fn read_verified(&self, path: &Path) -> io::Result<Vec<u8>> {
        let file = File::open(path)?;
        if file.metadata()?.len() != self.bytes {
            return Err(corrupt());
        }
        let mut data = Vec::new();
        file.take(self.bytes + 1).read_to_end(&mut data)?;
        if data.len() as u64 != self.bytes || format!("{:x}", Sha256::digest(&data)) != self.sha256
        {
            return Err(corrupt());
        }
        Ok(data)
    }
}

fn corrupt() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Speech model integrity check failed",
    )
}
