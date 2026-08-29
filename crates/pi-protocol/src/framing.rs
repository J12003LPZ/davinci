use thiserror::Error;

const FRAME_HEADER_LENGTH: usize = 4;
const MAX_UINT32: u32 = 0xffff_ffff;
const PAYLOAD_BLOCK_SIZE: usize = 64 * 1024;

pub const DEFAULT_MAX_FRAME_LENGTH: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct FrameDecoderOptions {
    pub max_frame_length: Option<u32>,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("{0}")]
    Message(String),
}

impl FrameError {
    fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

fn resolve_max_frame_length(options: Option<FrameDecoderOptions>) -> Result<u32, FrameError> {
    let value = options
        .and_then(|o| o.max_frame_length)
        .unwrap_or(DEFAULT_MAX_FRAME_LENGTH);
    Ok(value)
}

/// Prefixes a payload with its unsigned 32-bit big-endian byte length.
pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > MAX_UINT32 as usize {
        return Err(FrameError::new(
            "Frame payload exceeds the unsigned 32-bit length limit",
        ));
    }
    let length = payload.len() as u32;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LENGTH + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Validates that bytes contain exactly one complete frame within the configured limit.
pub fn assert_complete_frame(
    frame: &[u8],
    options: Option<FrameDecoderOptions>,
) -> Result<(), FrameError> {
    if frame.len() < FRAME_HEADER_LENGTH {
        return Err(FrameError::new(
            "Frame does not contain a complete length prefix",
        ));
    }
    let length = u32::from_be_bytes([frame[0], frame[1], frame[2], frame[3]]);
    let max_frame_length = resolve_max_frame_length(options)?;
    if length > max_frame_length {
        return Err(FrameError::new(format!(
            "Frame length {length} exceeds configured limit of {max_frame_length}"
        )));
    }
    if frame.len() != FRAME_HEADER_LENGTH + length as usize {
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

/// Incrementally splits arbitrary byte chunks into length-prefixed payloads.
pub struct FrameDecoder {
    header: [u8; FRAME_HEADER_LENGTH],
    header_length: usize,
    max_frame_length: u32,
    payload_blocks: Vec<Vec<u8>>,
    current_payload_block: Option<Vec<u8>>,
    current_payload_block_length: usize,
    expected_payload_length: Option<u32>,
    payload_length: u32,
    state: DecoderState,
}

impl FrameDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, FrameError> {
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
        if self.state == DecoderState::Ended {
            return Err(FrameError::new("Frame decoder has ended"));
        }
        if self.state == DecoderState::Failed {
            return Err(FrameError::new("Frame decoder has failed"));
        }

        let mut frames = Vec::new();
        let mut chunk_offset = 0;
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

                let frame_length = u32::from_be_bytes([
                    self.header[0],
                    self.header[1],
                    self.header[2],
                    self.header[3],
                ]);
                self.header_length = 0;
                if frame_length > self.max_frame_length {
                    return self.fail(format!(
                        "Frame length {frame_length} exceeds configured limit of {}",
                        self.max_frame_length
                    ));
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
                Some(value) => value,
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
                    let block = vec![0; remaining.min(PAYLOAD_BLOCK_SIZE)];
                    self.payload_blocks.push(block);
                    self.current_payload_block = self.payload_blocks.last().cloned();
                    self.current_payload_block_length = 0;
                }
                let block = self.payload_blocks.last_mut().expect("payload block");
                let payload_bytes = (block.len() - self.current_payload_block_length)
                    .min(chunk.len() - chunk_offset);
                block[self.current_payload_block_length
                    ..self.current_payload_block_length + payload_bytes]
                    .copy_from_slice(&chunk[chunk_offset..chunk_offset + payload_bytes]);
                self.current_payload_block_length += payload_bytes;
                self.payload_length += payload_bytes as u32;
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
        if self.state == DecoderState::Ended {
            return Err(FrameError::new("Frame decoder has ended"));
        }
        if self.state == DecoderState::Failed {
            return Err(FrameError::new("Frame decoder has failed"));
        }
        if self.header_length != 0 || self.expected_payload_length.is_some() {
            return self.fail("Truncated frame at end of stream");
        }
        self.state = DecoderState::Ended;
        Ok(())
    }

    fn fail<T>(&mut self, message: impl Into<String>) -> Result<T, FrameError> {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn concatenate(chunks: &[Vec<u8>]) -> Vec<u8> {
        chunks.iter().flatten().copied().collect()
    }

    #[test]
    fn prefixes_payloads_with_four_byte_big_endian_length() {
        assert_eq!(
            encode_frame(&[0xaa, 0xbb, 0xcc]).unwrap(),
            vec![0x00, 0x00, 0x00, 0x03, 0xaa, 0xbb, 0xcc]
        );
        assert_eq!(encode_frame(&[]).unwrap(), vec![0, 0, 0, 0]);
    }

    #[test]
    fn validates_one_complete_bounded_frame() {
        assert!(assert_complete_frame(
            &[0, 0, 0, 2, 1, 2],
            Some(FrameDecoderOptions {
                max_frame_length: Some(2)
            })
        )
        .is_ok());
        assert!(assert_complete_frame(&[0, 0, 0, 2, 1], None)
            .unwrap_err()
            .to_string()
            .contains("complete"));
        assert!(assert_complete_frame(&[0, 0, 0, 1, 1, 2], None)
            .unwrap_err()
            .to_string()
            .contains("exactly"));
        assert!(assert_complete_frame(
            &[0, 0, 0, 3, 1, 2, 3],
            Some(FrameDecoderOptions {
                max_frame_length: Some(2)
            })
        )
        .unwrap_err()
        .to_string()
        .contains("limit"));
    }

    #[test]
    fn decodes_fragmented_coalesced_and_empty_frames() {
        let wire = concatenate(&[
            encode_frame(&[1, 2, 3]).unwrap(),
            encode_frame(&[]).unwrap(),
            encode_frame(&[4]).unwrap(),
        ]);
        let mut decoder = FrameDecoder::new(None).unwrap();
        let mut frames = Vec::new();
        for byte in &wire {
            frames.extend(decoder.push(&[*byte]).unwrap());
        }
        decoder.end().unwrap();
        assert_eq!(frames, vec![vec![1, 2, 3], vec![], vec![4]]);

        let mut coalesced = FrameDecoder::new(None).unwrap();
        assert_eq!(coalesced.push(&wire).unwrap(), frames);
        coalesced.end().unwrap();
    }

    #[test]
    fn copies_payload_bytes_instead_of_aliasing_input() {
        let mut chunk = encode_frame(&[1, 2, 3]).unwrap();
        let mut decoder = FrameDecoder::new(None).unwrap();
        let frames = decoder.push(&chunk).unwrap();
        chunk.fill(9);
        assert_eq!(frames, vec![vec![1, 2, 3]]);
    }

    #[test]
    fn accepts_empty_chunks_and_clean_empty_stream() {
        let mut decoder = FrameDecoder::new(None).unwrap();
        assert_eq!(decoder.push(&[]).unwrap(), Vec::<Vec<u8>>::new());
        decoder.end().unwrap();
    }
}
