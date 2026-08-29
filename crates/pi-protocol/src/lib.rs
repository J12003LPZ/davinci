//! Framed CBOR protocol used by `pi-client` and `pi-server`.

mod cbor;
mod framing;
mod messages;

pub use cbor::{
    decode_cbor, decode_cbor_with, encode_cbor, encode_cbor_with, CborError, CborOptions, CborValue,
    DEFAULT_MAX_CBOR_BYTE_LENGTH, DEFAULT_MAX_CBOR_CONTAINER_LENGTH, DEFAULT_MAX_CBOR_DEPTH,
};
pub use framing::{assert_complete_frame, encode_frame, FrameDecoder, FrameError, DEFAULT_MAX_FRAME_LENGTH};
pub use messages::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn from_hex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn rfc8949_known_vectors() {
        let vectors: Vec<(CborValue, &str)> = vec![
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
            (CborValue::Integer(9_007_199_254_740_991), "1b001fffffffffffff"),
            (CborValue::Integer(-1), "20"),
            (CborValue::Integer(-10), "29"),
            (CborValue::Integer(-24), "37"),
            (CborValue::Integer(-25), "3818"),
            (CborValue::Integer(-100), "3863"),
            (CborValue::Integer(-1000), "3903e7"),
            (CborValue::Integer(-1_000_000), "3a000f423f"),
            (CborValue::Integer(-9_007_199_254_740_991), "3b001ffffffffffffe"),
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
            (
                CborValue::Array(vec![
                    CborValue::Integer(1),
                    CborValue::Array(vec![CborValue::Integer(2), CborValue::Integer(3)]),
                    CborValue::Array(vec![CborValue::Integer(4), CborValue::Integer(5)]),
                ]),
                "8301820203820405",
            ),
            (
                CborValue::Map(vec![
                    ("a".into(), CborValue::Integer(1)),
                    (
                        "b".into(),
                        CborValue::Array(vec![CborValue::Integer(2), CborValue::Integer(3)]),
                    ),
                ]),
                "a26161016162820203",
            ),
        ];
        for (value, wire) in vectors {
            assert_eq!(to_hex(&encode_cbor(&value).unwrap()), wire, "encode {wire}");
            let decoded = decode_cbor(&from_hex(wire)).unwrap();
            if matches!(value, CborValue::Float(float) if float == 0.0 && float.is_sign_negative()) {
                match decoded {
                    CborValue::Float(float) => assert!(float == 0.0 && float.is_sign_negative()),
                    other => panic!("expected -0.0, got {other:?}"),
                }
            } else {
                assert_eq!(decoded, value, "decode {wire}");
            }
        }
    }

    #[test]
    fn rejects_invalid_decoder_input() {
        for wire in [
            "",
            "18",
            "1c",
            "5f",
            "7f",
            "9f",
            "bf",
            "c000",
            "f7",
            "e0",
            "ff",
            "f93c00",
            "fa3f800000",
            "fb7ff0000000000000",
            "fb7ff8000000000000",
            "fb3ff00000",
            "44010203",
            "636162",
            "8201",
            "a16161",
            "0000",
            "a10102",
            "a2616101616102",
            "61ff",
            "62c080",
            "63eda080",
            "1b0020000000000000",
            "3b001fffffffffffff",
            "fb4340000000000000",
        ] {
            assert!(
                decode_cbor(&from_hex(wire)).is_err(),
                "expected rejection for {wire}"
            );
        }
    }

    #[test]
    fn frames_round_trip_and_fragment() {
        let payload = encode_cbor(&CborValue::Text("hello".into())).unwrap();
        let frame = encode_frame(&payload).unwrap();
        assert_complete_frame(&frame, DEFAULT_MAX_FRAME_LENGTH).unwrap();
        let mut decoder = FrameDecoder::new(None).unwrap();
        let mut assembled = Vec::new();
        for byte in &frame {
            assembled.extend(decoder.push(&[*byte]).unwrap());
        }
        decoder.end().unwrap();
        assert_eq!(assembled, vec![payload]);
    }

    #[test]
    fn hello_message_round_trip() {
        let message = ClientMessage::Hello {
            version: PROTOCOL_VERSION,
        };
        let framed = encode_client_message(&message).unwrap();
        let mut decoder = ClientMessageDecoder::new();
        let decoded = decoder.push(&framed).unwrap();
        assert_eq!(decoded, vec![message]);
    }
}
