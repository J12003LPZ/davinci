//! Private bounded JSON framing; audio never crosses this protocol.

use crate::state::Identity;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};
use zeroize::Zeroizing;

pub const VERSION: u32 = 1;
pub const BUILD: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    ":voice-1:371b5a7561823ab2bb32142d2751e35e7534727b"
);
pub const MAX_FRAME: usize = 64 * 1024;
pub const CHANNEL_CAPACITY: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VoiceError {
    MicrophoneUnavailable,
    PermissionDenied,
    UnsupportedBackend,
    ModelMissing,
    ModelCorrupt,
    DeviceDisconnected,
    NoSpeech,
    CaptureOverflow,
    InferenceFailed,
    WorkerCrashed,
    ProtocolMismatch,
    Timeout,
    TextTooLong,
}

impl std::fmt::Display for VoiceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MicrophoneUnavailable => {
                "Microphone could not be opened; check device access and permissions"
            }
            Self::PermissionDenied => "Microphone permission denied; check OS privacy settings",
            Self::UnsupportedBackend => "Unsupported audio configuration or backend",
            Self::ModelMissing => "Speech model missing; install or import an approved model",
            Self::ModelCorrupt => "Speech model integrity check failed",
            Self::DeviceDisconnected => "Microphone disconnected; select a device and retry",
            Self::NoSpeech => "No speech detected",
            Self::CaptureOverflow => "Audio capture overflow; recording discarded",
            Self::InferenceFailed => "Local transcription failed",
            Self::WorkerCrashed => "Voice helper unavailable or exited unexpectedly",
            Self::ProtocolMismatch => "Voice helper version does not match this installation",
            Self::Timeout => "Voice timed out; activate again to retry",
            Self::TextTooLong => "Transcription exceeds the text limit",
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Command {
    Hello {
        version: u32,
        build: String,
    },
    Prepare {
        id: Identity,
        model: String,
        path: std::path::PathBuf,
    },
    Start {
        id: Identity,
        language: String,
        device: Option<String>,
    },
    Stop {
        session: u64,
    },
    Cancel {
        session: u64,
    },
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    Hello { version: u32, build: String },
    Ready { id: Identity },
    CaptureStarted { id: Identity },
    CaptureStopped { id: Identity },
    Transcribing { id: Identity },
    Completed { id: Identity, text: String },
    Cancelled { id: Identity },
    Failed { id: Identity, error: VoiceError },
}

pub fn read_frame<R: Read, T: DeserializeOwned>(input: &mut R) -> io::Result<Option<T>> {
    let mut length = [0; 4];
    loop {
        match input.read(&mut length[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    input.read_exact(&mut length[1..])?;
    let len = u32::from_le_bytes(length) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Voice frame length rejected",
        ));
    }
    let mut bytes = Zeroizing::new(vec![0; len]);
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid voice frame"))
}

pub fn write_frame<W: Write, T: Serialize>(output: &mut W, message: &T) -> io::Result<()> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(message)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "Invalid voice message"))?,
    );
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Voice frame length rejected",
        ));
    }
    output.write_all(&(bytes.len() as u32).to_le_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn round_trip_and_clean_eof() {
        let mut bytes = Vec::new();
        write_frame(
            &mut bytes,
            &Command::Hello {
                version: VERSION,
                build: BUILD.into(),
            },
        )
        .unwrap();
        let mut input = Cursor::new(bytes);
        assert!(matches!(
            read_frame::<_, Command>(&mut input).unwrap(),
            Some(Command::Hello {
                version: VERSION,
                ..
            })
        ));
        assert!(read_frame::<_, Command>(&mut input).unwrap().is_none());
    }

    #[test]
    fn rejects_oversized_truncated_and_malformed_frames() {
        for data in [
            vec![0],
            (MAX_FRAME as u32 + 1).to_le_bytes().to_vec(),
            vec![2, 0, 0, 0, b'{'],
            vec![1, 0, 0, 0, 255],
        ] {
            assert!(read_frame::<_, Command>(&mut Cursor::new(data)).is_err());
        }
    }
}
