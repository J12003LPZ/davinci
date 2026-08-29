use serde::Serialize;
use thiserror::Error;

use crate::cbor::{decode_cbor, encode_cbor, CborValue};
use crate::framing::{
    assert_complete_frame, encode_frame, FrameDecoder, FrameDecoderOptions,
    DEFAULT_MAX_FRAME_LENGTH,
};
use crate::schemas::{ClientMessage, ServerMessage, PROTOCOL_VERSION};

#[derive(Debug, Error)]
#[error("{0}")]
pub struct ProtocolValidationError(pub String);

impl ProtocolValidationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

fn bounded_error_message(error: &dyn std::error::Error) -> String {
    let message = error.to_string();
    if message.len() <= 500 {
        message
    } else {
        format!("{}...", &message[..497])
    }
}

fn is_protocol_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => true,
        serde_json::Value::Array(items) => items.iter().all(is_protocol_value),
        serde_json::Value::Object(map) => map.values().all(is_protocol_value),
    }
}

pub fn parse_client_message(
    value: &serde_json::Value,
) -> Result<ClientMessage, ProtocolValidationError> {
    if !is_protocol_value(value) {
        return Err(ProtocolValidationError::new(
            "Invalid client protocol message",
        ));
    }
    serde_json::from_value(value.clone())
        .map_err(|_| ProtocolValidationError::new("Invalid client protocol message"))
}

pub fn parse_server_message(
    value: &serde_json::Value,
) -> Result<ServerMessage, ProtocolValidationError> {
    if !is_protocol_value(value) {
        return Err(ProtocolValidationError::new(
            "Invalid server protocol message",
        ));
    }
    serde_json::from_value(value.clone())
        .map_err(|_| ProtocolValidationError::new("Invalid server protocol message"))
}

fn encode_protocol_message<T: Serialize>(
    value: &T,
    kind: &str,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    let json = serde_json::to_value(value).map_err(|err| {
        ProtocolValidationError::new(format!("Unable to encode {kind} protocol message: {err}"))
    })?;
    let cbor = CborValue::from_json(&json).map_err(|err| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&err)
        ))
    })?;
    let max_frame_length = options
        .and_then(|o| o.max_frame_length)
        .unwrap_or(DEFAULT_MAX_FRAME_LENGTH);
    let payload = encode_cbor(
        &cbor,
        Some(crate::cbor::CborOptions {
            max_byte_length: Some(u64::from(max_frame_length)),
            ..Default::default()
        }),
    )
    .map_err(|err| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&err)
        ))
    })?;
    let frame = encode_frame(&payload).map_err(|err| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&err)
        ))
    })?;
    assert_complete_frame(
        &frame,
        Some(FrameDecoderOptions {
            max_frame_length: Some(max_frame_length),
        }),
    )
    .map_err(|err| {
        ProtocolValidationError::new(format!(
            "Unable to encode {kind} protocol message: {}",
            bounded_error_message(&err)
        ))
    })?;
    Ok(frame)
}

pub fn encode_client_message(
    message: &ClientMessage,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_protocol_message(message, "client", options)
}

pub fn encode_server_message(
    message: &ServerMessage,
    options: Option<FrameDecoderOptions>,
) -> Result<Vec<u8>, ProtocolValidationError> {
    encode_protocol_message(message, "server", options)
}

struct ValidatedMessageDecoder<T> {
    failed: bool,
    frames: FrameDecoder,
    kind: &'static str,
    max_frame_length: u32,
    parse: fn(&serde_json::Value) -> Result<T, ProtocolValidationError>,
}

impl<T> ValidatedMessageDecoder<T> {
    fn new(
        kind: &'static str,
        parse: fn(&serde_json::Value) -> Result<T, ProtocolValidationError>,
        options: Option<FrameDecoderOptions>,
    ) -> Result<Self, ProtocolValidationError> {
        Ok(Self {
            failed: false,
            frames: FrameDecoder::new(options)
                .map_err(|err| ProtocolValidationError::new(err.to_string()))?,
            kind,
            max_frame_length: options
                .and_then(|o| o.max_frame_length)
                .unwrap_or(DEFAULT_MAX_FRAME_LENGTH),
            parse,
        })
    }

    fn push(&mut self, chunk: &[u8]) -> Result<Vec<T>, ProtocolValidationError> {
        if self.failed {
            return Err(ProtocolValidationError::new(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        match self.decode_chunk(chunk) {
            Ok(messages) => Ok(messages),
            Err(err) => {
                self.failed = true;
                Err(err)
            }
        }
    }

    fn decode_chunk(&mut self, chunk: &[u8]) -> Result<Vec<T>, ProtocolValidationError> {
        let frames = self.frames.push(chunk).map_err(|err| {
            ProtocolValidationError::new(format!(
                "Invalid {} protocol frame: {}",
                self.kind,
                bounded_error_message(&err)
            ))
        })?;
        let mut messages = Vec::new();
        for frame in frames {
            let value = decode_cbor(
                &frame,
                Some(crate::cbor::CborOptions {
                    max_byte_length: Some(u64::from(self.max_frame_length)),
                    ..Default::default()
                }),
            )
            .map_err(|err| {
                ProtocolValidationError::new(format!(
                    "Invalid {} protocol frame: {}",
                    self.kind,
                    bounded_error_message(&err)
                ))
            })?;
            messages.push((self.parse)(&value.to_json())?);
        }
        Ok(messages)
    }

    fn end(&mut self) -> Result<(), ProtocolValidationError> {
        if self.failed {
            return Err(ProtocolValidationError::new(format!(
                "{} message decoder has failed",
                self.kind
            )));
        }
        self.frames.end().map_err(|err| {
            self.failed = true;
            ProtocolValidationError::new(format!(
                "Invalid {} protocol framing: {}",
                self.kind,
                bounded_error_message(&err)
            ))
        })
    }
}

pub struct ClientMessageDecoder {
    decoder: ValidatedMessageDecoder<ClientMessage>,
}

impl ClientMessageDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, ProtocolValidationError> {
        Ok(Self {
            decoder: ValidatedMessageDecoder::new("client", parse_client_message, options)?,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ClientMessage>, ProtocolValidationError> {
        self.decoder.push(chunk)
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.decoder.end()
    }
}

pub struct ServerMessageDecoder {
    decoder: ValidatedMessageDecoder<ServerMessage>,
}

impl ServerMessageDecoder {
    pub fn new(options: Option<FrameDecoderOptions>) -> Result<Self, ProtocolValidationError> {
        Ok(Self {
            decoder: ValidatedMessageDecoder::new("server", parse_server_message, options)?,
        })
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<ServerMessage>, ProtocolValidationError> {
        self.decoder.push(chunk)
    }

    pub fn end(&mut self) -> Result<(), ProtocolValidationError> {
        self.decoder.end()
    }
}

pub fn create_client_message_decoder(
    options: Option<FrameDecoderOptions>,
) -> Result<ClientMessageDecoder, ProtocolValidationError> {
    ClientMessageDecoder::new(options)
}

pub fn create_server_message_decoder(
    options: Option<FrameDecoderOptions>,
) -> Result<ServerMessageDecoder, ProtocolValidationError> {
    ServerMessageDecoder::new(options)
}

pub fn is_supported_protocol_version(version: u32) -> bool {
    version == PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schemas::{ClientMessage, PROTOCOL_VERSION};

    #[test]
    fn hello_roundtrip() {
        let hello = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let frame = encode_client_message(&hello, None).unwrap();
        let mut decoder = ClientMessageDecoder::new(None).unwrap();
        let messages = decoder.push(&frame).unwrap();
        decoder.end().unwrap();
        assert_eq!(messages, vec![hello]);
    }

    #[test]
    fn rejects_unsupported_version_helper() {
        assert!(is_supported_protocol_version(1));
        assert!(!is_supported_protocol_version(2));
    }

    #[test]
    fn invalid_client_message_uses_ts_error_string() {
        let error = parse_client_message(&serde_json::json!({"type": "nope"})).unwrap_err();
        assert_eq!(error.to_string(), "Invalid client protocol message");
    }
}
