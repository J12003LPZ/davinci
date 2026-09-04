use crate::error::FrameError;

pub const FRAME_HEADER_LENGTH: usize = 4;
pub const DEFAULT_MAX_FRAME_LENGTH: usize = 16 * 1024 * 1024;
const MAX_UINT32: u64 = 0xffff_ffff;

#[derive(Debug, Clone, Copy)]
pub struct FrameDecoderOptions {
    pub max_frame_length: usize,
}

impl Default for FrameDecoderOptions {
    fn default() -> Self {
        Self {
            max_frame_length: DEFAULT_MAX_FRAME_LENGTH,
        }
    }
}

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() as u64 > MAX_UINT32 {
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

pub fn assert_complete_frame(frame: &[u8], options: FrameDecoderOptions) -> Result<(), FrameError> {
    if frame.len() < FRAME_HEADER_LENGTH {
        return Err(FrameError::new(
            "Frame does not contain a complete length prefix",
        ));
    }
    let length = u32::from_be_bytes(frame[0..4].try_into().unwrap()) as usize;
    if length > options.max_frame_length {
        return Err(FrameError::new(format!(
            "Frame length {length} exceeds configured limit of {}",
            options.max_frame_length
        )));
    }
    if frame.len() != FRAME_HEADER_LENGTH + length {
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
    max_frame_length: usize,
    payload: Vec<u8>,
    expected: Option<usize>,
    state: DecoderState,
}

impl FrameDecoder {
    pub fn new(options: FrameDecoderOptions) -> Self {
        Self {
            header: [0; FRAME_HEADER_LENGTH],
            header_length: 0,
            max_frame_length: options.max_frame_length,
            payload: Vec::new(),
            expected: None,
            state: DecoderState::Open,
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        if self.state == DecoderState::Ended {
            return Err(FrameError::new("Frame decoder has ended"));
        }
        if self.state == DecoderState::Failed {
            return Err(FrameError::new("Frame decoder has failed"));
        }

        let mut frames = Vec::new();
        let mut offset = 0;
        while offset < chunk.len() {
            if self.expected.is_none() {
                let needed = FRAME_HEADER_LENGTH - self.header_length;
                let take = needed.min(chunk.len() - offset);
                self.header[self.header_length..self.header_length + take]
                    .copy_from_slice(&chunk[offset..offset + take]);
                self.header_length += take;
                offset += take;
                if self.header_length < FRAME_HEADER_LENGTH {
                    continue;
                }
                let frame_length = u32::from_be_bytes(self.header) as usize;
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
                self.expected = Some(frame_length);
                self.payload.clear();
                self.payload.reserve(frame_length);
            }

            let expected = self.expected.unwrap();
            let remaining = expected - self.payload.len();
            let take = remaining.min(chunk.len() - offset);
            self.payload
                .extend_from_slice(&chunk[offset..offset + take]);
            offset += take;
            if self.payload.len() == expected {
                frames.push(std::mem::take(&mut self.payload));
                self.expected = None;
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
        if self.header_length != 0 || self.expected.is_some() {
            return self.fail("Truncated frame at end of stream".to_string());
        }
        self.state = DecoderState::Ended;
        Ok(())
    }

    fn fail<T>(&mut self, message: String) -> Result<T, FrameError> {
        self.state = DecoderState::Failed;
        self.header_length = 0;
        self.payload.clear();
        self.expected = None;
        Err(FrameError::new(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_split_frames() {
        let payload = b"hello-protocol";
        let frame = encode_frame(payload).unwrap();
        assert_complete_frame(&frame, FrameDecoderOptions::default()).unwrap();

        let mut decoder = FrameDecoder::new(FrameDecoderOptions::default());
        let mut collected = Vec::new();
        for byte in &frame {
            collected.extend(decoder.push(&[*byte]).unwrap());
        }
        decoder.end().unwrap();
        assert_eq!(collected, vec![payload.to_vec()]);
    }

    #[test]
    fn rejects_oversized_declared_length() {
        let mut decoder = FrameDecoder::new(FrameDecoderOptions {
            max_frame_length: 8,
        });
        let header = 100u32.to_be_bytes();
        let err = decoder.push(&header).unwrap_err();
        assert!(err.to_string().contains("exceeds configured limit"));
    }

    #[test]
    fn detects_truncated_stream() {
        let mut decoder = FrameDecoder::new(FrameDecoderOptions::default());
        decoder.push(&[0, 0, 0, 4, 1, 2]).unwrap();
        assert!(decoder.end().is_err());
    }
}
