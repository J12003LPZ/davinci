use serde_json::Value;

use crate::cbor::{decode_cbor, encode_cbor, CborOptions, CborValue};
use crate::framing::{
    assert_complete_frame, encode_frame, FrameDecoder, FrameDecoderOptions,
    DEFAULT_MAX_FRAME_LENGTH,
};
use crate::validate::{parse_client_message, parse_server_message, ProtocolValidationError};

pub use crate::validate::ProtocolValidationError as ProtocolError;

fn bounded_error_message(error: &str) -> String {
    if error.len() <= 500 {
        error.to_string()
    } else {
        format!("{}...", &error[..497])
    }
}

fn encode_protocol_message(
    value: &Value,
    parse: fn(&Value) -> Result<Value, ProtocolValidationError>,
    kind: &str,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    let validated = parse(value)?;
    let max_frame_length = options
        .and_then(|o| o.max_frame_length)
        .unwrap_or(DEFAULT_MAX_FRAME_LENGTH);
    let cbor = encode_cbor(
        &CborValue::from_json(&validated),
        Some(&CborOptions {
            max_byte_length: Some(max_frame_length),
            ..CborOptions::default()
        }),
    )
    .map_err(|e| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&e.0)
        ))
    })?;
    let frame = encode_frame(&cbor).map_err(|e| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&e.0)
        ))
    })?;
    assert_complete_frame(
        &frame,
        Some(FrameDecoderOptions {
            max_frame_length: Some(max_frame_length),
        }),
    )
    .map_err(|e| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&e.0)
        ))
    })?;
    Ok(frame)
}

pub fn encode_client_message(
    message: &Value,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_protocol_message(message, parse_client_message, "client", options)
}

pub fn encode_server_message(
    message: &Value,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_protocol_message(message, parse_server_message, "server", options)
}

struct ValidatedMessageDecoder {
    failed: bool,
    frames: FrameDecoder,
    kind: &'static str,
    max_frame_length: u64,
    parse: fn(&Value) -> Result<Value, ProtocolValidationError>,
}

impl ValidatedMessageDecoder {
    fn new(
        kind: &'static str,
        parse: fn(&Value) -> Result<Value, ProtocolValidationError>,
        options: Option<FrameDecoderOptions>,
    ) -> Result<Self, crate::framing::RangeError> {
        Ok(Self {
            failed: false,
            frames: FrameDecoder::new(options)?,
            kind,
            max_frame_length: options
                .and_then(|o| o.max_frame_length)
                .unwrap_or(DEFAULT_MAX_FRAME_LENGTH),
            parse,
        })
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, ProtocolValidationError> {
        if self.failed {
            return Err(ProtocolValidationError::new(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        match self.frames.push(chunk) {
            Ok(frames) => {
                let mut messages = Vec::new();
                for frame in frames {
                    let decoded = decode_cbor(
                        &frame,
                        Some(&CborOptions {
                            max_byte_length: Some(self.max_frame_length),
                            ..CborOptions::default()
                        }),
                    )
                    .map_err(|e| {
                        self.failed = true;
                        ProtocolValidationError::new(format!(
                            "Invalid {} protocol frame: {}",
                            self.kind,
                            bounded_error_message(&e.0)
                        ))
                    })?;
                    messages.push((self.parse)(&decoded.to_json()).inspect_err(|_| {
                        self.failed = true;
                    })?);
                }
                Ok(messages)
            }
            Err(error) => {
                self.failed = true;
                Err(ProtocolValidationError::new(format!(
                    "Invalid {} protocol frame: {}",
                    self.kind,
                    bounded_error_message(&error.0)
                )))
            }
        }
    }

    fn end(&mut self) -> Result<(), ProtocolValidationError> {
        if self.failed {
            return Err(ProtocolValidationError::new(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        self.frames.end().map_err(|error| {
            self.failed = true;
            ProtocolValidationError::new(format!(
                "Invalid {} protocol framing: {}",
                self.kind,
                bounded_error_message(&error.0)
            ))
        })
    }
}

pub struct ClientMessageDecoder {
    decoder: ValidatedMessageDecoder,
}

impl ClientMessageDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, crate::framing::RangeError> {
        Ok(Self {
            decoder: ValidatedMessageDecoder::new("client", parse_client_message, options)?,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, ProtocolValidationError> {
        self.decoder.push(chunk)
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.decoder.end()
    }
}

pub struct ServerMessageDecoder {
    decoder: ValidatedMessageDecoder,
}

impl ServerMessageDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, crate::framing::RangeError> {
        Ok(Self {
            decoder: ValidatedMessageDecoder::new("server", parse_server_message, options)?,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Value>, ProtocolValidationError> {
        self.decoder.push(chunk)
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.decoder.end()
    }
}

pub fn is_supported_protocol_version(version: f64) -> bool {
    version.fract() == 0.0 && version as i64 == crate::PROTOCOL_VERSION
}
