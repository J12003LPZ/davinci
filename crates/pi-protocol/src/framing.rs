use thiserror::Error;

pub const DEFAULT_MAX_FRAME_LENGTH: u32 = 16 * 1024 * 1024;
const FRAME_HEADER_LENGTH: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct FrameError(pub String);

pub fn encode_frame(payload: &[u8]) -> Result<Vec<u8>, FrameError> {
    if payload.len() > u32::MAX as usize {
        return Err(FrameError(
            "Frame payload exceeds the unsigned 32-bit length limit".into(),
        ));
    }
    let length = payload.len() as u32;
    let mut frame = Vec::with_capacity(FRAME_HEADER_LENGTH + payload.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn assert_complete_frame(frame: &[u8], max_frame_length: u32) -> Result<(), FrameError> {
    if frame.len() < FRAME_HEADER_LENGTH {
        return Err(FrameError(
            "Frame does not contain a complete length prefix".into(),
        ));
    }
    let length = u32::from_be_bytes(frame[..4].try_into().unwrap());
    if length > max_frame_length {
        return Err(FrameError(format!(
            "Frame length {length} exceeds configured limit of {max_frame_length}"
        )));
    }
    if frame.len() != FRAME_HEADER_LENGTH + length as usize {
        return Err(FrameError(
            "Frame must contain exactly one complete payload".into(),
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
    max_frame_length: u32,
    header: [u8; 4],
    header_length: usize,
    expected: Option<u32>,
    payload: Vec<u8>,
    state: DecoderState,
}

impl FrameDecoder {
    pub fn new(max_frame_length: Option<u32>) -> Result<Self, FrameError> {
        Ok(Self {
            max_frame_length: max_frame_length.unwrap_or(DEFAULT_MAX_FRAME_LENGTH),
            header: [0; 4],
            header_length: 0,
            expected: None,
            payload: Vec::new(),
            state: DecoderState::Open,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        self.ensure_open()?;
        let mut frames = Vec::new();
        let mut offset = 0;
        while offset < chunk.len() {
            if self.expected.is_none() {
                let take = (FRAME_HEADER_LENGTH - self.header_length).min(chunk.len() - offset);
                self.header[self.header_length..self.header_length + take]
                    .copy_from_slice(&chunk[offset..offset + take]);
                self.header_length += take;
                offset += take;
                if self.header_length < FRAME_HEADER_LENGTH {
                    continue;
                }
                let length = u32::from_be_bytes(self.header);
                self.header_length = 0;
                if length > self.max_frame_length {
                    return self.fail(format!(
                        "Frame length {length} exceeds configured limit of {}",
                        self.max_frame_length
                    ));
                }
                if length == 0 {
                    frames.push(Vec::new());
                    continue;
                }
                self.expected = Some(length);
                self.payload.clear();
            }
            let expected = self.expected.unwrap() as usize;
            let remaining = expected - self.payload.len();
            let take = remaining.min(chunk.len() - offset);
            self.payload.extend_from_slice(&chunk[offset..offset + take]);
            offset += take;
            if self.payload.len() == expected {
                frames.push(std::mem::take(&mut self.payload));
                self.expected = None;
            }
        }
        Ok(frames)
    }

    pub fn end(&mut self) -> Result<(), FrameError> {
        self.ensure_open()?;
        if self.header_length != 0 || self.expected.is_some() {
            return self.fail("Truncated frame at end of stream".into());
        }
        self.state = DecoderState::Ended;
        Ok(())
    }

    fn ensure_open(&self) -> Result<(), FrameError> {
        match self.state {
            DecoderState::Open => Ok(()),
            DecoderState::Ended => Err(FrameError("Frame decoder has ended".into())),
            DecoderState::Failed => Err(FrameError("Frame decoder has failed".into())),
        }
    }

    fn fail<T>(&mut self, message: String) -> Result<T, FrameError> {
        self.state = DecoderState::Failed;
        self.header_length = 0;
        self.expected = None;
        self.payload.clear();
        Err(FrameError(message))
    }
}
