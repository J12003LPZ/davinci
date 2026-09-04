use serde_json::json;

use crate::*;

#[test]
fn prefixes_payloads_with_four_byte_big_endian_length() {
    let frame = encode_frame(&[0xaa, 0xbb, 0xcc]).unwrap();
    assert_eq!(frame, vec![0x00, 0x00, 0x00, 0x03, 0xaa, 0xbb, 0xcc]);
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
        .0
        .contains("complete"));
    assert!(assert_complete_frame(&[0, 0, 0, 1, 1, 2], None)
        .unwrap_err()
        .0
        .contains("exactly"));
    assert!(assert_complete_frame(
        &[0, 0, 0, 3, 1, 2, 3],
        Some(FrameDecoderOptions {
            max_frame_length: Some(2)
        })
    )
    .unwrap_err()
    .0
    .contains("limit"));
}

#[test]
fn decodes_fragmented_coalesced_and_empty_frames() {
    let mut wire = encode_frame(&[1, 2, 3]).unwrap();
    wire.extend(encode_frame(&[]).unwrap());
    wire.extend(encode_frame(&[4]).unwrap());
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
fn assembles_payloads_spanning_multiple_internal_blocks() {
    let payload: Vec<u8> = (0..70_000).map(|i| (i % 251) as u8).collect();
    let wire = encode_frame(&payload).unwrap();
    let mut decoder = FrameDecoder::new(None).unwrap();
    let mut frames = decoder.push(&wire[..101]).unwrap();
    frames.extend(decoder.push(&wire[101..65_541]).unwrap());
    frames.extend(decoder.push(&wire[65_541..]).unwrap());
    decoder.end().unwrap();
    assert_eq!(frames, vec![payload]);
}

#[test]
fn copies_payload_bytes() {
    let mut chunk = encode_frame(&[1, 2, 3]).unwrap();
    let mut decoder = FrameDecoder::new(None).unwrap();
    let frames = decoder.push(&chunk).unwrap();
    chunk.fill(9);
    assert_eq!(frames, vec![vec![1, 2, 3]]);
}

#[test]
fn rejects_truncated_stream() {
    let mut decoder = FrameDecoder::new(None).unwrap();
    assert!(decoder.push(&[0, 0, 0]).unwrap().is_empty());
    assert!(decoder.end().unwrap_err().0.contains("Truncated"));
}

#[test]
fn rejects_oversized_declared_length() {
    let mut decoder = FrameDecoder::new(Some(FrameDecoderOptions {
        max_frame_length: Some(3),
    }))
    .unwrap();
    assert!(decoder.push(&[0, 0, 0, 4]).unwrap_err().0.contains("limit"));
    assert!(decoder.push(&[1]).unwrap_err().0.contains("failed"));
}

#[test]
fn cannot_push_after_end() {
    let mut decoder = FrameDecoder::new(None).unwrap();
    decoder.end().unwrap();
    assert!(decoder.push(&[]).unwrap_err().0.contains("ended"));
    assert!(decoder.end().unwrap_err().0.contains("ended"));
}

#[test]
fn rejects_invalid_max_frame_length() {
    assert!(FrameDecoder::new(Some(FrameDecoderOptions {
        max_frame_length: Some(DEFAULT_MAX_FRAME_LENGTH * 1_000)
    }))
    .is_err());
}

fn known_vectors() -> Vec<(CborValue, &'static str)> {
    vec![
        (CborValue::Null, "f6"),
        (CborValue::Bool(false), "f4"),
        (CborValue::Bool(true), "f5"),
        (CborValue::Integer(0), "00"),
        (CborValue::Integer(1), "01"),
        (CborValue::Integer(10), "0a"),
        (CborValue::Integer(23), "17"),
        (CborValue::Integer(24), "1818"),
        (CborValue::Integer(25), "1819"),
        (CborValue::Integer(100), "1864"),
        (CborValue::Integer(1000), "1903e8"),
        (CborValue::Integer(1_000_000), "1a000f4240"),
        (CborValue::Integer(1_000_000_000_000), "1b000000e8d4a51000"),
        (
            CborValue::Integer(9_007_199_254_740_991),
            "1b001fffffffffffff",
        ),
        (CborValue::Integer(-1), "20"),
        (CborValue::Integer(-10), "29"),
        (CborValue::Integer(-24), "37"),
        (CborValue::Integer(-25), "3818"),
        (CborValue::Integer(-100), "3863"),
        (CborValue::Integer(-1000), "3903e7"),
        (CborValue::Integer(-1_000_000), "3a000f423f"),
        (
            CborValue::Integer(-9_007_199_254_740_991),
            "3b001ffffffffffffe",
        ),
        (CborValue::Float(1.1), "fb3ff199999999999a"),
        (CborValue::Float(-0.0), "fb8000000000000000"),
        (CborValue::Bytes(vec![1, 2, 3, 4]), "4401020304"),
        (CborValue::Text(String::new()), "60"),
        (CborValue::Text("IETF".into()), "6449455446"),
        (CborValue::Text("ü".into()), "62c3bc"),
        (CborValue::Text("水".into()), "63e6b0b4"),
        (CborValue::Text("𐅑".into()), "64f0908591"),
        (CborValue::Array(vec![]), "80"),
        (
            CborValue::Array(vec![
                CborValue::Integer(1),
                CborValue::Integer(2),
                CborValue::Integer(3),
            ]),
            "83010203",
        ),
    ]
}

#[test]
fn encodes_and_decodes_rfc8949_vectors() {
    for (value, wire) in known_vectors() {
        let encoded = encode_cbor(&value, None).unwrap();
        assert_eq!(to_hex(&encoded), wire, "encode {value:?}");
        let decoded = decode_cbor(&from_hex(wire).unwrap(), None).unwrap();
        if matches!(value, CborValue::Float(v) if v == 0.0 && v.is_sign_negative()) {
            match decoded {
                CborValue::Float(v) => assert!(v == 0.0 && v.is_sign_negative()),
                other => panic!("expected -0.0, got {other:?}"),
            }
        } else {
            assert_eq!(decoded, value);
        }
    }
}

#[test]
fn protocol_version_is_one() {
    assert_eq!(PROTOCOL_VERSION, 1);
    assert!(is_supported_protocol_version(1.0));
    assert!(!is_supported_protocol_version(2.0));
    assert!(!is_supported_protocol_version(2.5));
}

#[test]
fn accepts_integer_client_hello_versions() {
    for version in [0, 1, 2] {
        let message = json!({ "type": "hello", "version": version });
        assert_eq!(parse_client_message(&message).unwrap(), message);
    }
}

#[test]
fn rejects_invalid_client_hello() {
    assert!(parse_client_message(&json!({ "type": "hello", "version": "1" })).is_err());
    assert!(parse_client_message(&json!({ "type": "hello", "version": 1.5 })).is_err());
    assert!(
        parse_client_message(&json!({ "type": "hello", "version": 1, "token": "secret" })).is_err()
    );
    assert!(
        parse_client_message(&json!({ "type": "hello", "version": 1, "extra": true })).is_err()
    );
}

#[test]
fn does_not_parse_json_strings() {
    assert!(parse_client_message(&json!(r#"{"type":"hello","version":1}"#)).is_err());
}

#[test]
fn rejects_image_input_on_prompt() {
    assert!(parse_client_message(&json!({
        "type": "request",
        "id": "request-1",
        "request": {
            "command": "prompt",
            "sessionId": "session-1",
            "text": "inspect",
            "images": [{ "type": "image", "data": "abc", "mimeType": "image/png" }]
        }
    }))
    .is_err());
}

#[test]
fn parses_server_handshake() {
    let server_hello = json!({
        "type": "hello",
        "version": 1,
        "connectionId": "connection-1",
        "snapshot": {
            "serverId": "server-1",
            "protocolVersion": 1,
            "revision": 0,
            "sessions": [],
            "models": []
        }
    });
    assert_eq!(parse_server_message(&server_hello).unwrap(), server_hello);
}

#[test]
fn encodes_client_hello_frame() {
    let hello = json!({ "type": "hello", "version": 1 });
    let frame = encode_client_message(&hello, None).unwrap();
    let mut decoder = ClientMessageDecoder::new(None).unwrap();
    let messages = decoder.push(&frame).unwrap();
    decoder.end().unwrap();
    assert_eq!(messages, vec![hello]);
}
