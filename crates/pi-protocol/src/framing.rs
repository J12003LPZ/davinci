//! Length-prefixed frames matching `packages/protocol/src/framing.ts`.

use thiserror::Error;

use crate::cbor::MAX_UINT32;

pub const FRAME_HEADER_LENGTH: usize = 4;
pub const DEFAULT_MAX_FRAME_LENGTH: u64 = 16 * 1024 * 1024;
const PAYLOAD_BLOCK_SIZE: usize = 64 * 1024;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct FrameError(pub String);

impl FrameError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct RangeError(pub String);

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameDecoderOptions {
    pub max_frame_length: Option<u64>,
}

pub fn resolve_max_frame_length(options: Option<FrameDecoderOptions>) -> Result<u64, RangeError> {
    let value = options
        .and_then(|o| o.max_frame_length)
        .unwrap_or(DEFAULT_MAX_FRAME_LENGTH);
    if value > MAX_UINT32 {
        return Err(RangeError(format!(
            "maxFrameLength must be an integer between 0 and {MAX_UINT32}"
        )));
    }
    Ok(value)
}

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, RangeError> {
    if payload.len() as u64 > MAX_UINT32 {
        return Err(RangeError(
            "Frame payload exceeds the unsigned 32-bit length limit".into(),
        ));
    }
    let length = payload.len() as u32;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LENGTH + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn assert_complete_frame(
    frame: &[u8],
    options: Option<FrameDecoderOptions>,
) -> Result<(), FrameError> {
    if frame.len() < FRAME_HEADER_LENGTH {
        return Err(FrameError::new(
            "Frame does not contain a complete length prefix",
        ));
    }
    let length = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]) as u64;
    let max_frame_length = resolve_max_frame_length(options).map_err(|e| FrameError::new(e.0))?;
    if length > max_frame_length {
        return Err(FrameError::new(format!(
            "Frame length {length} exceeds configured limit of {max_frame_length}"
        )));
    }
    if frame.len() as u64 != FRAME_HEADER_LENGTH as u64 + length {
        return Err(FrameError::new(
            "Frame must contain exactly one complete payload",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecoderState {
    Open,
    Ended,
    Failed,
}

pub struct FrameDecoder {
    header: [u8; FRAME_HEADER_LENGTH],
    header_length: usize,
    max_frame_length: u64,
    payload_blocks: Vec<Vec<u8>>,
    current_payload_block: Option<Vec<u8>>,
    current_payload_block_length: usize,
    expected_payload_length: Option<u64>,
    payload_length: u64,
    state: DecoderState,
}

impl FrameDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, RangeError> {
        Ok(Self {
            header: [0; FRAME_HEADER_LENGTH],
            header_length: 0,
            max_frame_length: resolve_max_frame_length(options)?,
            payload_blocks: Vec::new(),
            current_payload_block: None,
            current_payload_block_length: 0,
            expected_payload_length: None,
            payload_length: 0,
            state: DecoderState::Open,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        match self.state {
            DecoderState::Ended => return Err(FrameError::new("Frame decoder has ended")),
            DecoderState::Failed => return Err(FrameError::new("Frame decoder has failed")),
            DecoderState::Open => {}
        }
        let mut frames = Vec::new();
        let mut chunk_offset = 0usize;
        while chunk_offset < chunk.len() {
            if self.expected_payload_length.is_none() {
                let header_bytes =
                    (FRAME_HEADER_LENGTH - self.header_length).min(chunk.len() - chunk_offset);
                self.header[self.header_length..self.header_length + header_bytes]
                    .copy_from_slice(&chunk[chunk_offset..chunk_offset + header_bytes]);
                self.header_length += header_bytes;
                chunk_offset += header_bytes;
                if self.header_length < FRAME_HEADER_LENGTH {
                    continue;
                }
                let frame_length = u32::from_be_bytes(self.header) as u64;
                self.header_length = 0;
                if frame_length > self.max_frame_length {
                    self.fail(format!(
                        "Frame length {frame_length} exceeds configured limit of {}",
                        self.max_frame_length
                    ))?;
                }
                if frame_length == 0 {
                    frames.push(Vec::new());
                    continue;
                }
                self.expected_payload_length = Some(frame_length);
                self.payload_blocks.clear();
                self.current_payload_block = None;
                self.current_payload_block_length = 0;
                self.payload_length = 0;
            }

            let expected = match self.expected_payload_length {
                Some(v) => v,
                None => continue,
            };
            while chunk_offset < chunk.len() && self.payload_length < expected {
                if self
                    .current_payload_block
                    .as_ref()
                    .map(|b| self.current_payload_block_length == b.len())
                    .unwrap_or(true)
                {
                    let remaining = (expected - self.payload_length) as usize;
                    let block = vec![0u8; remaining.min(PAYLOAD_BLOCK_SIZE)];
                    self.payload_blocks.push(block);
                    self.current_payload_block = self.payload_blocks.last().cloned();
                    self.current_payload_block_length = 0;
                }
                let block_len = self.payload_blocks.last().map(|b| b.len()).unwrap_or(0);
                let payload_bytes =
                    (block_len - self.current_payload_block_length).min(chunk.len() - chunk_offset);
                if let Some(block) = self.payload_blocks.last_mut() {
                    block[self.current_payload_block_length
                        ..self.current_payload_block_length + payload_bytes]
                        .copy_from_slice(&chunk[chunk_offset..chunk_offset + payload_bytes]);
                }
                self.current_payload_block_length += payload_bytes;
                self.payload_length += payload_bytes as u64;
                chunk_offset += payload_bytes;
            }
            if self.payload_length == expected {
                let payload = if self.payload_blocks.len() == 1 {
                    self.payload_blocks.remove(0)
                } else {
                    let mut payload = Vec::with_capacity(expected as usize);
                    for block in &self.payload_blocks {
                        payload.extend_from_slice(block);
                    }
                    payload
                };
                frames.push(payload);
                self.payload_blocks.clear();
                self.current_payload_block = None;
                self.current_payload_block_length = 0;
                self.expected_payload_length = None;
                self.payload_length = 0;
            }
        }
        Ok(frames)
    }

    pub fn end(&mut self) -> Result<(), FrameError> {
        match self.state {
            DecoderState::Ended => return Err(FrameError::new("Frame decoder has ended")),
            DecoderState::Failed => return Err(FrameError::new("Frame decoder has failed")),
            DecoderState::Open => {}
        }
        if self.header_length != 0 || self.expected_payload_length.is_some() {
            self.fail("Truncated frame at end of stream")?;
        }
        self.state = DecoderState::Ended;
        Ok(())
    }

    fn fail(&mut self, message: impl Into<String>) -> Result<(), FrameError> {
        self.state = DecoderState::Failed;
        self.header_length = 0;
        self.payload_blocks.clear();
        self.current_payload_block = None;
        self.current_payload_block_length = 0;
        self.expected_payload_length = None;
        self.payload_length = 0;
        Err(FrameError::new(message))
    }
}
