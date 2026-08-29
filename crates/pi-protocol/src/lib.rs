//! Transport-neutral CBOR protocol for remote pi sessions.
//! Mirrors `vendor/pi/packages/protocol`.

pub mod cbor;
pub mod codec;
pub mod framing;
pub mod validate;

pub const PROTOCOL_VERSION: i64 = 1;

pub use cbor::{
    decode_cbor, encode_cbor, from_hex, to_hex, CborError, CborOptions, CborValue,
    DEFAULT_MAX_CBOR_BYTE_LENGTH, DEFAULT_MAX_CBOR_CONTAINER_LENGTH, DEFAULT_MAX_CBOR_DEPTH,
};
pub use codec::{
    encode_client_message, encode_server_message, is_supported_protocol_version,
    ClientMessageDecoder, ServerMessageDecoder,
};
pub use framing::{
    assert_complete_frame, encode_frame, FrameDecoder, FrameDecoderOptions, FrameError, RangeError,
    DEFAULT_MAX_FRAME_LENGTH,
};
pub use validate::{parse_client_message, parse_server_message, ProtocolValidationError};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
