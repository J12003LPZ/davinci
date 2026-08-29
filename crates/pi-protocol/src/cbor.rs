use std::collections::{BTreeMap, HashSet};
use std::f64;

use thiserror::Error;

pub const DEFAULT_MAX_CBOR_BYTE_LENGTH: usize = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CBOR_CONTAINER_LENGTH: usize = 1_000_000;
pub const DEFAULT_MAX_CBOR_DEPTH: usize = 64;
const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

#[derive(Debug, Clone, PartialEq, Error)]
#[error("{0}")]
pub struct CborError(pub String);

#[derive(Debug, Clone, Copy)]
pub struct CborOptions {
    pub max_byte_length: usize,
    pub max_container_length: usize,
    pub max_depth: usize,
}

impl Default for CborOptions {
    fn default() -> Self {
        Self {
            max_byte_length: DEFAULT_MAX_CBOR_BYTE_LENGTH,
            max_container_length: DEFAULT_MAX_CBOR_CONTAINER_LENGTH,
            max_depth: DEFAULT_MAX_CBOR_DEPTH,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CborValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Bytes(Vec<u8>),
    Text(String),
    Array(Vec<CborValue>),
    Map(Vec<(String, CborValue)>),
}

impl CborValue {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, CborError> {
        match value {
            serde_json::Value::Null => Ok(Self::Null),
            serde_json::Value::Bool(flag) => Ok(Self::Bool(*flag)),
            serde_json::Value::Number(number) => {
                if let Some(int) = number.as_i64() {
                    if !is_safe_integer(int) {
                        return Err(CborError("unsafe integer".into()));
                    }
                    Ok(Self::Integer(int))
                } else if let Some(float) = number.as_f64() {
                    if !float.is_finite() {
                        return Err(CborError("non-finite number".into()));
                    }
                    Ok(Self::Float(float))
                } else {
                    Err(CborError("unsupported number".into()))
                }
            }
            serde_json::Value::String(text) => Ok(Self::Text(text.clone())),
            serde_json::Value::Array(items) => Ok(Self::Array(
                items
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            serde_json::Value::Object(map) => Ok(Self::Map(
                map.iter()
                    .map(|(key, value)| Ok((key.clone(), Self::from_json(value)?)))
                    .collect::<Result<Vec<_>, CborError>>()?,
            )),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(flag) => serde_json::Value::Bool(*flag),
            Self::Integer(int) => serde_json::json!(*int),
            Self::Float(float) => serde_json::Number::from_f64(*float)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::Bytes(bytes) => serde_json::json!(bytes),
            Self::Text(text) => serde_json::Value::String(text.clone()),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Map(entries) => {
                let mut map = serde_json::Map::new();
                for (key, value) in entries {
                    map.insert(key.clone(), value.to_json());
                }
                serde_json::Value::Object(map)
            }
        }
    }
}

fn is_safe_integer(value: i64) -> bool {
    (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&value)
}

pub fn encode_cbor(value: &CborValue) -> Result<Vec<u8>, CborError> {
    encode_cbor_with(value, CborOptions::default())
}

pub fn encode_cbor_with(value: &CborValue, options: CborOptions) -> Result<Vec<u8>, CborError> {
    let mut out = Vec::new();
    encode_into(value, &mut out, 0, &options, &mut Vec::new())?;
    Ok(out)
}

fn encode_into(
    value: &CborValue,
    out: &mut Vec<u8>,
    depth: usize,
    options: &CborOptions,
    stack: &mut Vec<*const CborValue>,
) -> Result<(), CborError> {
    if depth > options.max_depth {
        return Err(CborError("CBOR depth limit exceeded".into()));
    }
    let ptr = value as *const CborValue;
    if stack.contains(&ptr) {
        return Err(CborError("CBOR cycles are not allowed".into()));
    }
    stack.push(ptr);
    let result = encode_value(value, out, depth, options, stack);
    stack.pop();
    result
}

fn encode_value(
    value: &CborValue,
    out: &mut Vec<u8>,
    depth: usize,
    options: &CborOptions,
    stack: &mut Vec<*const CborValue>,
) -> Result<(), CborError> {
    match value {
        CborValue::Null => out.push(0xf6),
        CborValue::Bool(false) => out.push(0xf4),
        CborValue::Bool(true) => out.push(0xf5),
        CborValue::Integer(int) => encode_integer(*int, out)?,
        CborValue::Float(float) => encode_float(*float, out)?,
        CborValue::Bytes(bytes) => {
            if bytes.len() > options.max_byte_length {
                return Err(CborError("CBOR byte length limit exceeded".into()));
            }
            encode_header(2, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        CborValue::Text(text) => {
            if text.contains('\u{FFFD}')
                && text
                    .chars()
                    .any(|ch| (0xD800..=0xDFFF).contains(&(ch as u32)))
            {
                return Err(CborError("lone Unicode surrogate".into()));
            }
            if !text.is_char_boundary(text.len()) {
                return Err(CborError("invalid Unicode".into()));
            }
            let bytes = text.as_bytes();
            if bytes.len() > options.max_byte_length {
                return Err(CborError("CBOR byte length limit exceeded".into()));
            }
            encode_header(3, bytes.len() as u64, out);
            out.extend_from_slice(bytes);
        }
        CborValue::Array(items) => {
            if items.len() > options.max_container_length {
                return Err(CborError("CBOR container length limit exceeded".into()));
            }
            encode_header(4, items.len() as u64, out);
            for item in items {
                encode_into(item, out, depth + 1, options, stack)?;
            }
        }
        CborValue::Map(entries) => {
            if entries.len() > options.max_container_length {
                return Err(CborError("CBOR container length limit exceeded".into()));
            }
            let mut seen = HashSet::new();
            encode_header(5, entries.len() as u64, out);
            for (key, value) in entries {
                if !seen.insert(key) {
                    return Err(CborError("duplicate map key".into()));
                }
                encode_into(
                    &CborValue::Text(key.clone()),
                    out,
                    depth + 1,
                    options,
                    stack,
                )?;
                encode_into(value, out, depth + 1, options, stack)?;
            }
        }
    }
    Ok(())
}

fn encode_integer(value: i64, out: &mut Vec<u8>) -> Result<(), CborError> {
    if !is_safe_integer(value) {
        return Err(CborError("unsafe integer".into()));
    }
    if value >= 0 {
        encode_header(0, value as u64, out);
    } else {
        encode_header(1, ((-1) - value) as u64, out);
    }
    Ok(())
}

fn encode_float(value: f64, out: &mut Vec<u8>) -> Result<(), CborError> {
    if !value.is_finite() && !value.is_sign_negative() && value == 0.0 {
        return Err(CborError("non-finite number".into()));
    }
    if !value.is_finite() {
        return Err(CborError("non-finite number".into()));
    }
    out.push(0xfb);
    out.extend_from_slice(&value.to_bits().to_be_bytes());
    Ok(())
}

fn encode_header(major: u8, argument: u64, out: &mut Vec<u8>) {
    if argument <= 23 {
        out.push((major << 5) | argument as u8);
    } else if argument <= 0xff {
        out.push((major << 5) | 24);
        out.push(argument as u8);
    } else if argument <= 0xffff {
        out.push((major << 5) | 25);
        out.extend_from_slice(&(argument as u16).to_be_bytes());
    } else if argument <= 0xffff_ffff {
        out.push((major << 5) | 26);
        out.extend_from_slice(&(argument as u32).to_be_bytes());
    } else {
        out.push((major << 5) | 27);
        out.extend_from_slice(&argument.to_be_bytes());
    }
}

pub fn decode_cbor(bytes: &[u8]) -> Result<CborValue, CborError> {
    decode_cbor_with(bytes, CborOptions::default())
}

pub fn decode_cbor_with(bytes: &[u8], options: CborOptions) -> Result<CborValue, CborError> {
    if bytes.is_empty() {
        return Err(CborError("empty input".into()));
    }
    let (value, rest) = decode_item(bytes, 0, &options)?;
    if !rest.is_empty() {
        return Err(CborError("trailing data".into()));
    }
    Ok(value)
}

fn decode_item<'a>(
    bytes: &'a [u8],
    depth: usize,
    options: &CborOptions,
) -> Result<(CborValue, &'a [u8]), CborError> {
    if depth > options.max_depth {
        return Err(CborError("CBOR depth limit exceeded".into()));
    }
    if bytes.is_empty() {
        return Err(CborError("truncated CBOR value".into()));
    }
    let initial = bytes[0];
    let major = initial >> 5;
    let additional = initial & 0x1f;
    if major == 7 {
        return decode_simple(additional, 0, &bytes[1..]);
    }
    let (argument, rest) = read_argument(additional, &bytes[1..])?;
    match major {
        0 => {
            if argument > MAX_SAFE_INTEGER as u64 {
                return Err(CborError("unsafe integer".into()));
            }
            Ok((CborValue::Integer(argument as i64), rest))
        }
        1 => {
            if argument > MAX_SAFE_INTEGER as u64 {
                return Err(CborError("unsafe integer".into()));
            }
            let value = -1 - argument as i64;
            if !is_safe_integer(value) {
                return Err(CborError("unsafe integer".into()));
            }
            Ok((CborValue::Integer(value), rest))
        }
        2 => {
            if argument as usize > options.max_byte_length {
                return Err(CborError("CBOR byte length limit exceeded".into()));
            }
            if rest.len() < argument as usize {
                return Err(CborError("truncated byte string".into()));
            }
            let (head, tail) = rest.split_at(argument as usize);
            Ok((CborValue::Bytes(head.to_vec()), tail))
        }
        3 => {
            if argument as usize > options.max_byte_length {
                return Err(CborError("CBOR byte length limit exceeded".into()));
            }
            if rest.len() < argument as usize {
                return Err(CborError("truncated text string".into()));
            }
            let (head, tail) = rest.split_at(argument as usize);
            let text = std::str::from_utf8(head).map_err(|_| CborError("invalid UTF-8".into()))?;
            if !text
                .chars()
                .all(|ch| !(0xD800..=0xDFFF).contains(&(ch as u32)))
                && text.as_bytes().iter().any(|b| *b >= 0x80)
            {
                validate_utf8_strict(head)?;
            }
            validate_utf8_strict(head)?;
            Ok((CborValue::Text(text.to_string()), tail))
        }
        4 => {
            if argument as usize > options.max_container_length {
                return Err(CborError("CBOR container length limit exceeded".into()));
            }
            let mut items = Vec::with_capacity(argument as usize);
            let mut cursor = rest;
            for _ in 0..argument {
                let (item, next) = decode_item(cursor, depth + 1, options)?;
                items.push(item);
                cursor = next;
            }
            Ok((CborValue::Array(items), cursor))
        }
        5 => {
            if argument as usize > options.max_container_length {
                return Err(CborError("CBOR container length limit exceeded".into()));
            }
            let mut entries = Vec::with_capacity(argument as usize);
            let mut seen = BTreeMap::new();
            let mut cursor = rest;
            for _ in 0..argument {
                let (key, next) = decode_item(cursor, depth + 1, options)?;
                let CborValue::Text(key) = key else {
                    return Err(CborError("non-string map key".into()));
                };
                if seen.insert(key.clone(), ()).is_some() {
                    return Err(CborError("duplicate map key".into()));
                }
                let (value, next) = decode_item(next, depth + 1, options)?;
                entries.push((key, value));
                cursor = next;
            }
            Ok((CborValue::Map(entries), cursor))
        }
        6 => Err(CborError("CBOR tags are not supported".into())),
        _ => Err(CborError("unsupported major type".into())),
    }
}

fn read_argument(additional: u8, rest: &[u8]) -> Result<(u64, &[u8]), CborError> {
    match additional {
        0..=23 => Ok((u64::from(additional), rest)),
        24 => {
            if rest.is_empty() {
                return Err(CborError("truncated integer".into()));
            }
            Ok((u64::from(rest[0]), &rest[1..]))
        }
        25 => take_int(rest, 2),
        26 => take_int(rest, 4),
        27 => take_int(rest, 8),
        28..=30 => Err(CborError("reserved additional information".into())),
        31 => Err(CborError(
            "indefinite-length items are not supported".into(),
        )),
        _ => Err(CborError("reserved additional information".into())),
    }
}

fn take_int(rest: &[u8], width: usize) -> Result<(u64, &[u8]), CborError> {
    if rest.len() < width {
        return Err(CborError("truncated integer".into()));
    }
    let mut buf = [0u8; 8];
    buf[8 - width..].copy_from_slice(&rest[..width]);
    Ok((u64::from_be_bytes(buf), &rest[width..]))
}

fn decode_simple(
    additional: u8,
    argument: u64,
    rest: &[u8],
) -> Result<(CborValue, &[u8]), CborError> {
    match additional {
        20 => Ok((CborValue::Bool(false), rest)),
        21 => Ok((CborValue::Bool(true), rest)),
        22 => Ok((CborValue::Null, rest)),
        23 => Err(CborError("undefined".into())),
        25 => Err(CborError("float16 is not supported".into())),
        26 => Err(CborError("float32 is not supported".into())),
        27 => {
            if rest.len() < 8 {
                return Err(CborError("truncated float64".into()));
            }
            let bits = u64::from_be_bytes(rest[..8].try_into().unwrap());
            let float = f64::from_bits(bits);
            if float.is_nan() || float.is_infinite() {
                return Err(CborError("non-finite number".into()));
            }
            if float.fract() == 0.0 && float.abs() > MAX_SAFE_INTEGER as f64 {
                return Err(CborError("unsafe integer encoded as float64".into()));
            }
            if float == 0.0 || float.fract() != 0.0 {
                Ok((CborValue::Float(float), &rest[8..]))
            } else if is_safe_integer(float as i64) && !float.is_sign_negative() {
                // Integers must use integer major types; whole floats that are
                // not -0 stay floats only when they are not integers in JSON.
                Ok((CborValue::Float(float), &rest[8..]))
            } else {
                Ok((CborValue::Float(float), &rest[8..]))
            }
        }
        0..=19 => Err(CborError("unsupported simple value".into())),
        24 => {
            let _ = argument;
            Err(CborError("unsupported simple value".into()))
        }
        31 => Err(CborError("break outside an indefinite item".into())),
        _ => Err(CborError("unsupported simple value".into())),
    }
}

fn validate_utf8_strict(bytes: &[u8]) -> Result<(), CborError> {
    let mut index = 0;
    while index < bytes.len() {
        let first = bytes[index];
        let width = match first {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => return Err(CborError("invalid UTF-8 byte".into())),
        };
        if index + width > bytes.len() {
            return Err(CborError("truncated text string".into()));
        }
        let slice = &bytes[index..index + width];
        match std::str::from_utf8(slice) {
            Ok(text) => {
                if text
                    .chars()
                    .any(|ch| (0xD800..=0xDFFF).contains(&(ch as u32)))
                {
                    return Err(CborError("UTF-8 surrogate".into()));
                }
            }
            Err(_) => {
                if first == 0xC0 || first == 0xC1 || (first == 0xE0 && bytes[index + 1] < 0xA0) {
                    return Err(CborError("overlong UTF-8".into()));
                }
                return Err(CborError("invalid UTF-8 byte".into()));
            }
        }
        index += width;
    }
    Ok(())
}
