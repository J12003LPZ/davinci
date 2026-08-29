use pi_parity::{assert_cbor_roundtrip, assert_session_parity};

#[test]
fn test_cbor_wire_format_roundtrip() {
    assert_cbor_roundtrip();
}

#[test]
fn test_session_jsonl_parity() {
    assert_session_parity();
}
