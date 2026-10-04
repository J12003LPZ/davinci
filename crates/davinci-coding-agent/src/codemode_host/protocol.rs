//! Bounded framing shared by the supervisor and its private host transport.
use serde_json::Value;
use std::io::{self, Read, Write};

pub const MAX_FRAME_BYTES: usize = 2_097_152;

pub fn read_frame(reader: &mut impl Read) -> io::Result<Value> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Codemode frame limit",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    let value: Value = serde_json::from_slice(&body)?;
    if !value.is_object()
        || value.get("version").and_then(Value::as_u64) != Some(1)
        || value.get("type").and_then(Value::as_str).is_none()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid Codemode envelope",
        ));
    }
    Ok(value)
}

pub fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(value)?;
    if body.is_empty() || body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Codemode frame limit",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn round_trip_and_malformed_frames() {
        let value = serde_json::json!({"version":1,"type":"hello","text":"界"});
        let mut wire = Vec::new();
        write_frame(&mut wire, &value).unwrap();
        assert_eq!(read_frame(&mut &wire[..]).unwrap(), value);
        assert!(read_frame(&mut &wire[..wire.len() - 1]).is_err());
        let header = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        assert!(read_frame(&mut &header[..]).is_err());
    }
}
