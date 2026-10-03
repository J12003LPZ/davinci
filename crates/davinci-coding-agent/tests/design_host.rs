use davinci_coding_agent::design::host::{FrameDecoder, MAX_FRAME};
use serde_json::json;

fn frame(value: serde_json::Value) -> Vec<u8> {
    let bytes = serde_json::to_vec(&value).unwrap();
    let mut result = (bytes.len() as u32).to_be_bytes().to_vec();
    result.extend(bytes);
    result
}
#[test]
fn framing_handles_every_split_and_rejects_invalid_bounds_or_identity() {
    let bytes = frame(json!({"version":1,"id":"test-123","operation":"list","payload":{}}));
    for split in 0..=bytes.len() {
        let mut decoder = FrameDecoder::default();
        let mut messages = decoder.receive(&bytes[..split]).unwrap();
        messages.extend(decoder.receive(&bytes[split..]).unwrap());
        assert_eq!(messages.len(), 1);
    }
    assert!(FrameDecoder::default()
        .receive(&((MAX_FRAME + 1) as u32).to_be_bytes())
        .is_err());
    assert!(FrameDecoder::default().receive(&[0, 0, 0, 0]).is_err());
    for value in [
        json!({"version":2,"id":"a"}),
        json!({"version":1,"id":"../../x"}),
        json!({"version":1,"id":1}),
    ] {
        assert!(FrameDecoder::default().receive(&frame(value)).is_err());
    }
    let batch = frame(json!({"version":1,"id":"a"})).repeat(9);
    assert!(FrameDecoder::default().receive(&batch).is_err());
}
