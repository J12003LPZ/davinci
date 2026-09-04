//! Length-prefixed CBOR protocol matching `@earendil-works/pi-protocol`.

mod cbor;
mod codec;
mod framing;
mod schemas;

pub use cbor::{
    decode_cbor, encode_cbor, CborError, CborOptions, CborValue, DEFAULT_MAX_CBOR_BYTE_LENGTH,
    DEFAULT_MAX_CBOR_CONTAINER_LENGTH, DEFAULT_MAX_CBOR_DEPTH,
};
pub use codec::{
    create_client_message_decoder, create_server_message_decoder, encode_client_message,
    encode_server_message, is_supported_protocol_version, parse_client_message,
    parse_server_message, ClientMessageDecoder, ProtocolValidationError, ServerMessageDecoder,
};
pub use framing::{
    assert_complete_frame, encode_frame, FrameDecoder, FrameDecoderOptions, FrameError,
    DEFAULT_MAX_FRAME_LENGTH,
};
pub use schemas::*;
