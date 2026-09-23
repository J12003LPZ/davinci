use super::{ProcessConfig, ProcessIdentity};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::{self, Read, Write};

pub(super) const MAGIC: &[u8] = b"DAVINCI_PROCESS_V2\n";
pub(super) const MAX_FRAME: usize = 128 * 1024;
pub(super) const MAX_INPUT: usize = 16 * 1024;
pub(super) const POLL: std::time::Duration = std::time::Duration::from_millis(20);

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Configure {
        identity: ProcessIdentity,
        config: ProcessConfig,
    },
    Write {
        identity: ProcessIdentity,
        id: u64,
        bytes: Vec<u8>,
    },
    CloseStdin {
        identity: ProcessIdentity,
        id: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Event {
    Hello {
        pid: u32,
    },
    Started {
        identity: ProcessIdentity,
        pid: u32,
    },
    Output {
        identity: ProcessIdentity,
        bytes: Vec<u8>,
        stderr: bool,
    },
    Written {
        identity: ProcessIdentity,
        id: u64,
        count: usize,
        failed: bool,
    },
    Exit {
        identity: ProcessIdentity,
        code: Option<i32>,
        output_complete: bool,
    },
    LaunchFailed {
        identity: ProcessIdentity,
        message: String,
    },
    Failed {
        identity: ProcessIdentity,
    },
}

pub(super) fn read<T: DeserializeOwned>(input: &mut impl Read) -> io::Result<T> {
    let mut header = [0; 4];
    input.read_exact(&mut header)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    input.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub(super) fn write<T: Serialize>(output: &mut impl Write, message: &T) -> io::Result<()> {
    let bytes = serde_json::to_vec(message).map_err(|_| invalid())?;
    if bytes.len() > MAX_FRAME {
        return Err(invalid());
    }
    output.write_all(&(bytes.len() as u32).to_be_bytes())?;
    output.write_all(&bytes)?;
    output.flush()
}

pub(super) fn handshake(input: &mut impl Read) -> io::Result<()> {
    // The trusted test host can emit a harness banner. No child output ever
    // enters this channel; accept at most 4 KiB before the protocol marker.
    let mut matched = 0;
    for _ in 0..4096 {
        let mut byte = [0];
        input.read_exact(&mut byte)?;
        matched = if byte[0] == MAGIC[matched] {
            matched + 1
        } else {
            usize::from(byte[0] == MAGIC[0])
        };
        if matched == MAGIC.len() {
            return Ok(());
        }
    }
    Err(invalid())
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid supervisor frame")
}
