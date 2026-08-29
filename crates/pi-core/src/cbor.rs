use crate::error::CborError;
use indexmap::IndexMap;
use serde_json::{Map, Number, Value};

pub const DEFAULT_MAX_CBOR_BYTE_LENGTH: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CBOR_CONTAINER_LENGTH: usize = 1_000_000;
pub const DEFAULT_MAX_CBOR_DEPTH: usize = 64;

const UINT32_BASE: u64 = 0x1_0000_0000;

#[derive(Debug, Clone, PartialEq)]
pub enum CborValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(IndexMap<String, CborValue>),
}

impl CborValue {
    pub fn from_json(value: &Value) -> Result<Self, CborError> {
        json_to_cbor(value, 0)
    }

    pub fn to_json(&self) -> Result<Value, CborError> {
        cbor_to_json(self)
    }
}

fn json_to_cbor(value: &Value, depth: usize) -> Result<CborValue, CborError> {
    if depth > DEFAULT_MAX_CBOR_DEPTH {
        return Err(CborError::new("CBOR depth exceeds configured limit"));
    }
    match value {
        Value::Null => Ok(CborValue::Null),
        Value::Bool(b) => Ok(CborValue::Bool(*b)),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if !is_safe_integer(i as f64) && i.abs() > (1i64 << 53) {
                    return Err(CborError::new("unsafe integer"));
                }
                Ok(CborValue::Integer(i))
            } else if let Some(u) = n.as_u64() {
                if u > 9_007_199_254_740_991 {
                    return Err(CborError::new("unsafe positive integer"));
                }
                Ok(CborValue::Integer(u as i64))
            } else if let Some(f) = n.as_f64() {
                if !f.is_finite() {
                    return Err(CborError::new("non-finite number"));
                }
                Ok(CborValue::Float(f))
            } else {
                Err(CborError::new("unsupported number"))
            }
        }
        Value::String(s) => {
            validate_unicode(s)?;
            Ok(CborValue::Text(s.clone()))
        }
        Value::Array(items) => {
            if items.len() > DEFAULT_MAX_CBOR_CONTAINER_LENGTH {
                return Err(CborError::new("container too large"));
            }
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(json_to_cbor(item, depth + 1)?);
            }
            Ok(CborValue::Array(out))
        }
        Value::Object(map) => {
            if map.len() > DEFAULT_MAX_CBOR_CONTAINER_LENGTH {
                return Err(CborError::new("container too large"));
            }
            let mut out = IndexMap::new();
            for (k, v) in map {
                validate_unicode(k)?;
                out.insert(k.clone(), json_to_cbor(v, depth + 1)?);
            }
            Ok(CborValue::Map(out))
        }
    }
}

fn cbor_to_json(value: &CborValue) -> Result<Value, CborError> {
    match value {
        CborValue::Null => Ok(Value::Null),
        CborValue::Bool(b) => Ok(Value::Bool(*b)),
        CborValue::Integer(i) => Ok(Value::Number(Number::from(*i))),
        CborValue::Float(f) => Number::from_f64(*f)
            .map(Value::Number)
            .ok_or_else(|| CborError::new("non-finite float")),
        CborValue::Bytes(_) => Err(CborError::new(
            "CBOR byte strings are not allowed inside JsonValue",
        )),
        CborValue::Text(s) => Ok(Value::String(s.clone())),
        CborValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(cbor_to_json(item)?);
            }
            Ok(Value::Array(out))
        }
        CborValue::Map(map) => {
            let mut out = Map::new();
            for (k, v) in map {
                out.insert(k.clone(), cbor_to_json(v)?);
            }
            Ok(Value::Object(out))
        }
    }
}

fn is_safe_integer(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value.abs() <= 9_007_199_254_740_991.0
}

fn validate_unicode(s: &str) -> Result<(), CborError> {
    // Rust `str` already excludes unpaired surrogates; reject the replacement
    // used when a host language would have produced a lossy string.
    if s.chars().any(|c| (c as u32) > 0x10FFFF) {
        return Err(CborError::new("string contains unpaired Unicode surrogate"));
    }
    Ok(())
}

struct Writer {
    buf: Vec<u8>,
    max: usize,
}

impl Writer {
    fn new(max: usize) -> Self {
        Self {
            buf: Vec::with_capacity(256.min(max)),
            max,
        }
    }

    fn ensure(&mut self, extra: usize) -> Result<(), CborError> {
        if self.buf.len() + extra > self.max {
            return Err(CborError::new(format!(
                "CBOR byte length exceeds configured limit of {}",
                self.max
            )));
        }
        Ok(())
    }

    fn write_byte(&mut self, b: u8) -> Result<(), CborError> {
        self.ensure(1)?;
        self.buf.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CborError> {
        self.ensure(bytes.len())?;
        self.buf.extend_from_slice(bytes);
        Ok(())
    }

    fn write_uint(&mut self, major: u8, value: u64) -> Result<(), CborError> {
        if value <= 23 {
            self.write_byte((major << 5) | value as u8)
        } else if value <= 0xff {
            self.write_byte((major << 5) | 24)?;
            self.write_byte(value as u8)
        } else if value <= 0xffff {
            self.write_byte((major << 5) | 25)?;
            self.write_bytes(&[(value >> 8) as u8, value as u8])
        } else if value <= 0xffff_ffff {
            self.write_byte((major << 5) | 26)?;
            self.write_bytes(&[
                (value >> 24) as u8,
                (value >> 16) as u8,
                (value >> 8) as u8,
                value as u8,
            ])
        } else {
            self.write_byte((major << 5) | 27)?;
            let high = value / UINT32_BASE;
            let low = value - high * UINT32_BASE;
            self.write_bytes(&[
                (high >> 24) as u8,
                (high >> 16) as u8,
                (high >> 8) as u8,
                high as u8,
                (low >> 24) as u8,
                (low >> 16) as u8,
                (low >> 8) as u8,
                low as u8,
            ])
        }
    }
}

pub fn encode_cbor(value: &CborValue) -> Result<Vec<u8>, CborError> {
    let mut w = Writer::new(DEFAULT_MAX_CBOR_BYTE_LENGTH);
    encode_into(&mut w, value, 0)?;
    Ok(w.buf)
}

pub fn encode_json(value: &Value) -> Result<Vec<u8>, CborError> {
    encode_cbor(&CborValue::from_json(value)?)
}

fn encode_into(w: &mut Writer, value: &CborValue, depth: usize) -> Result<(), CborError> {
    if depth > DEFAULT_MAX_CBOR_DEPTH {
        return Err(CborError::new("CBOR depth exceeds configured limit"));
    }
    match value {
        CborValue::Null => w.write_byte(0xf6),
        CborValue::Bool(false) => w.write_byte(0xf4),
        CborValue::Bool(true) => w.write_byte(0xf5),
        CborValue::Integer(n) => {
            if *n >= 0 {
                w.write_uint(0, *n as u64)
            } else {
                w.write_uint(1, (-1 - *n) as u64)
            }
        }
        CborValue::Float(f) => {
            if !f.is_finite() {
                return Err(CborError::new("non-finite number"));
            }
            w.ensure(9)?;
            w.buf.push(0xfb);
            w.buf.extend_from_slice(&f.to_be_bytes());
            Ok(())
        }
        CborValue::Bytes(bytes) => {
            w.write_uint(2, bytes.len() as u64)?;
            w.write_bytes(bytes)
        }
        CborValue::Text(s) => {
            validate_unicode(s)?;
            w.write_uint(3, s.len() as u64)?;
            w.write_bytes(s.as_bytes())
        }
        CborValue::Array(items) => {
            if items.len() > DEFAULT_MAX_CBOR_CONTAINER_LENGTH {
                return Err(CborError::new("container too large"));
            }
            w.write_uint(4, items.len() as u64)?;
            for item in items {
                encode_into(w, item, depth + 1)?;
            }
            Ok(())
        }
        CborValue::Map(map) => {
            if map.len() > DEFAULT_MAX_CBOR_CONTAINER_LENGTH {
                return Err(CborError::new("container too large"));
            }
            w.write_uint(5, map.len() as u64)?;
            for (k, v) in map {
                encode_into(w, &CborValue::Text(k.clone()), depth + 1)?;
                encode_into(w, v, depth + 1)?;
            }
            Ok(())
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
    max_depth: usize,
    max_container: usize,
}

impl<'a> Reader<'a> {
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    fn read_byte(&mut self) -> Result<u8, CborError> {
        if self.offset >= self.data.len() {
            return Err(CborError::new("truncated CBOR"));
        }
        let b = self.data[self.offset];
        self.offset += 1;
        Ok(b)
    }

    fn read_exact(&mut self, n: usize) -> Result<&'a [u8], CborError> {
        if self.remaining() < n {
            return Err(CborError::new("truncated CBOR"));
        }
        let slice = &self.data[self.offset..self.offset + n];
        self.offset += n;
        Ok(slice)
    }

    fn read_uint(&mut self, additional: u8) -> Result<u64, CborError> {
        match additional {
            0..=23 => Ok(additional as u64),
            24 => Ok(self.read_byte()? as u64),
            25 => {
                let b = self.read_exact(2)?;
                Ok(((b[0] as u64) << 8) | b[1] as u64)
            }
            26 => {
                let b = self.read_exact(4)?;
                Ok(((b[0] as u64) << 24)
                    | ((b[1] as u64) << 16)
                    | ((b[2] as u64) << 8)
                    | b[3] as u64)
            }
            27 => {
                let b = self.read_exact(8)?;
                let mut v = 0u64;
                for byte in b {
                    v = (v << 8) | *byte as u64;
                }
                Ok(v)
            }
            31 => Err(CborError::new("indefinite-length items are not supported")),
            _ => Err(CborError::new("unsupported additional information")),
        }
    }
}

pub fn decode_cbor(bytes: &[u8]) -> Result<CborValue, CborError> {
    if bytes.len() > DEFAULT_MAX_CBOR_BYTE_LENGTH {
        return Err(CborError::new("CBOR byte length exceeds configured limit"));
    }
    let mut r = Reader {
        data: bytes,
        offset: 0,
        max_depth: DEFAULT_MAX_CBOR_DEPTH,
        max_container: DEFAULT_MAX_CBOR_CONTAINER_LENGTH,
    };
    let value = decode_value(&mut r, 0)?;
    if r.offset != bytes.len() {
        return Err(CborError::new("trailing CBOR data"));
    }
    Ok(value)
}

pub fn decode_json(bytes: &[u8]) -> Result<Value, CborError> {
    decode_cbor(bytes)?.to_json()
}

fn decode_value(r: &mut Reader<'_>, depth: usize) -> Result<CborValue, CborError> {
    if depth > r.max_depth {
        return Err(CborError::new("CBOR depth exceeds configured limit"));
    }
    let initial = r.read_byte()?;
    let major = initial >> 5;
    let additional = initial & 0x1f;
    match major {
        0 => {
            let n = r.read_uint(additional)?;
            if n > 9_007_199_254_740_991 {
                return Err(CborError::new("unsafe positive integer"));
            }
            Ok(CborValue::Integer(n as i64))
        }
        1 => {
            let n = r.read_uint(additional)?;
            if n > 9_007_199_254_740_991 {
                return Err(CborError::new("unsafe negative integer"));
            }
            Ok(CborValue::Integer(-1 - n as i64))
        }
        2 => {
            let len = r.read_uint(additional)? as usize;
            let bytes = r.read_exact(len)?;
            Ok(CborValue::Bytes(bytes.to_vec()))
        }
        3 => {
            let len = r.read_uint(additional)? as usize;
            let bytes = r.read_exact(len)?;
            let s = std::str::from_utf8(bytes)
                .map_err(|_| CborError::new("invalid UTF-8"))?
                .to_string();
            Ok(CborValue::Text(s))
        }
        4 => {
            let len = r.read_uint(additional)? as usize;
            if len > r.max_container {
                return Err(CborError::new("container too large"));
            }
            let mut items = Vec::with_capacity(len);
            for _ in 0..len {
                items.push(decode_value(r, depth + 1)?);
            }
            Ok(CborValue::Array(items))
        }
        5 => {
            let len = r.read_uint(additional)? as usize;
            if len > r.max_container {
                return Err(CborError::new("container too large"));
            }
            let mut map = IndexMap::new();
            for _ in 0..len {
                let key = match decode_value(r, depth + 1)? {
                    CborValue::Text(s) => s,
                    _ => return Err(CborError::new("map keys must be strings")),
                };
                if map.contains_key(&key) {
                    return Err(CborError::new("duplicate map key"));
                }
                let value = decode_value(r, depth + 1)?;
                map.insert(key, value);
            }
            Ok(CborValue::Map(map))
        }
        6 => Err(CborError::new("CBOR tags are not supported")),
        7 => match additional {
            20 => Ok(CborValue::Bool(false)),
            21 => Ok(CborValue::Bool(true)),
            22 => Ok(CborValue::Null),
            26 => Err(CborError::new("float32 is not supported")),
            27 => {
                let b = r.read_exact(8)?;
                let bits = u64::from_be_bytes(b.try_into().expect("8 bytes"));
                let f = f64::from_bits(bits);
                if !f.is_finite() {
                    return Err(CborError::new("non-finite number"));
                }
                Ok(CborValue::Float(f))
            }
            31 => Err(CborError::new("break markers are not supported")),
            _ => Err(CborError::new("unsupported simple value")),
        },
        _ => Err(CborError::new("unsupported major type")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn from_hex(hex: &str) -> Vec<u8> {
        hex::decode(hex).expect("hex")
    }

    fn to_hex(bytes: &[u8]) -> String {
        hex::encode(bytes)
    }

    #[test]
    fn rfc8949_vectors() {
        let cases: &[(&str, CborValue)] = &[
            ("f6", CborValue::Null),
            ("f4", CborValue::Bool(false)),
            ("f5", CborValue::Bool(true)),
            ("00", CborValue::Integer(0)),
            ("01", CborValue::Integer(1)),
            ("0a", CborValue::Integer(10)),
            ("17", CborValue::Integer(23)),
            ("1818", CborValue::Integer(24)),
            ("1819", CborValue::Integer(25)),
            ("1864", CborValue::Integer(100)),
            ("1903e8", CborValue::Integer(1000)),
            ("1a000f4240", CborValue::Integer(1_000_000)),
            ("1b000000e8d4a51000", CborValue::Integer(1_000_000_000_000)),
            (
                "1b001fffffffffffff",
                CborValue::Integer(9_007_199_254_740_991),
            ),
            ("20", CborValue::Integer(-1)),
            ("29", CborValue::Integer(-10)),
            ("37", CborValue::Integer(-24)),
            ("3818", CborValue::Integer(-25)),
            ("3863", CborValue::Integer(-100)),
            ("3903e7", CborValue::Integer(-1000)),
            ("3a000f423f", CborValue::Integer(-1_000_000)),
            ("60", CborValue::Text(String::new())),
            ("6449455446", CborValue::Text("IETF".into())),
            ("62c3bc", CborValue::Text("ü".into())),
            ("63e6b0b4", CborValue::Text("水".into())),
            ("64f0908591", CborValue::Text("𐅑".into())),
            ("80", CborValue::Array(vec![])),
            (
                "83010203",
                CborValue::Array(vec![
                    CborValue::Integer(1),
                    CborValue::Integer(2),
                    CborValue::Integer(3),
                ]),
            ),
        ];

        for (wire, value) in cases {
            assert_eq!(to_hex(&encode_cbor(value).unwrap()), *wire, "encode {wire}");
            assert_eq!(
                decode_cbor(&from_hex(wire)).unwrap(),
                *value,
                "decode {wire}"
            );
        }

        let nested = CborValue::Array(vec![
            CborValue::Integer(1),
            CborValue::Array(vec![CborValue::Integer(2), CborValue::Integer(3)]),
            CborValue::Array(vec![CborValue::Integer(4), CborValue::Integer(5)]),
        ]);
        assert_eq!(to_hex(&encode_cbor(&nested).unwrap()), "8301820203820405");

        let mut map = IndexMap::new();
        map.insert("a".into(), CborValue::Integer(1));
        map.insert(
            "b".into(),
            CborValue::Array(vec![CborValue::Integer(2), CborValue::Integer(3)]),
        );
        assert_eq!(
            to_hex(&encode_cbor(&CborValue::Map(map)).unwrap()),
            "a26161016162820203"
        );

        let bytes = CborValue::Bytes(vec![1, 2, 3, 4]);
        assert_eq!(to_hex(&encode_cbor(&bytes).unwrap()), "4401020304");

        let f = encode_cbor(&CborValue::Float(1.1)).unwrap();
        assert_eq!(to_hex(&f), "fb3ff199999999999a");

        let neg_zero = encode_cbor(&CborValue::Float(-0.0)).unwrap();
        assert_eq!(to_hex(&neg_zero), "fb8000000000000000");
    }

    #[test]
    fn rejects_trailing_data() {
        assert!(decode_cbor(&[0x00, 0x00]).is_err());
    }
}
