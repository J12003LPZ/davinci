//! Strict definite-length RFC 8949 subset matching `packages/protocol/src/cbor`.

use indexmap::IndexMap;
use thiserror::Error;

pub const UINT32_BASE: u64 = 0x1_0000_0000;
pub const MAX_UINT32: u64 = 0xffff_ffff;
const MAX_CONFIGURED_DEPTH: u32 = 512;
pub const DEFAULT_MAX_CBOR_BYTE_LENGTH: u64 = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CBOR_CONTAINER_LENGTH: u64 = 1_000_000;
pub const DEFAULT_MAX_CBOR_DEPTH: u32 = 64;
const JS_MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
#[error("{0}")]
pub struct CborError(pub String);

impl CborError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

#[derive(Debug, Clone, Default)]
pub struct CborOptions {
    pub max_byte_length: Option<u64>,
    pub max_container_length: Option<u64>,
    pub max_depth: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct ResolvedCborOptions {
    pub max_byte_length: u64,
    pub max_container_length: u64,
    pub max_depth: u32,
}

fn resolve_limit(name: &str, value: u64, maximum: u64) -> Result<u64, CborError> {
    if value > maximum {
        return Err(CborError::new(format!(
            "{name} must be an integer between 0 and {maximum}"
        )));
    }
    Ok(value)
}

pub fn resolve_options(options: Option<&CborOptions>) -> Result<ResolvedCborOptions, CborError> {
    let defaults = CborOptions::default();
    let options = options.unwrap_or(&defaults);
    Ok(ResolvedCborOptions {
        max_byte_length: resolve_limit(
            "maxByteLength",
            options
                .max_byte_length
                .unwrap_or(DEFAULT_MAX_CBOR_BYTE_LENGTH),
            MAX_UINT32,
        )?,
        max_container_length: resolve_limit(
            "maxContainerLength",
            options
                .max_container_length
                .unwrap_or(DEFAULT_MAX_CBOR_CONTAINER_LENGTH),
            MAX_UINT32,
        )?,
        max_depth: resolve_limit(
            "maxDepth",
            u64::from(options.max_depth.unwrap_or(DEFAULT_MAX_CBOR_DEPTH)),
            u64::from(MAX_CONFIGURED_DEPTH),
        )? as u32,
    })
}

/// Protocol JSON/CBOR value. Maps preserve insertion order and `__proto__` keys.
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
    pub fn from_json(value: &serde_json::Value) -> Self {
        match value {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(v) => Self::Bool(*v),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Self::Integer(i)
                } else if let Some(u) = n.as_u64() {
                    if u <= JS_MAX_SAFE_INTEGER as u64 {
                        Self::Integer(u as i64)
                    } else {
                        Self::Float(n.as_f64().unwrap_or(f64::NAN))
                    }
                } else {
                    Self::Float(n.as_f64().unwrap_or(f64::NAN))
                }
            }
            serde_json::Value::String(s) => Self::Text(s.clone()),
            serde_json::Value::Array(items) => {
                Self::Array(items.iter().map(Self::from_json).collect())
            }
            serde_json::Value::Object(map) => {
                let mut out = IndexMap::new();
                for (k, v) in map {
                    out.insert(k.clone(), Self::from_json(v));
                }
                Self::Map(out)
            }
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(v) => serde_json::Value::Bool(*v),
            Self::Integer(v) => serde_json::Value::Number((*v).into()),
            Self::Float(v) => serde_json::Number::from_f64(*v)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Self::Bytes(bytes) => serde_json::Value::Array(
                bytes.iter().copied().map(serde_json::Value::from).collect(),
            ),
            Self::Text(s) => serde_json::Value::String(s.clone()),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Map(map) => {
                let mut obj = serde_json::Map::new();
                for (k, v) in map {
                    obj.insert(k.clone(), v.to_json());
                }
                serde_json::Value::Object(obj)
            }
        }
    }
}

struct CborWriter {
    buffer: Vec<u8>,
    max_byte_length: u64,
}

impl CborWriter {
    fn new(max_byte_length: u64) -> Self {
        Self {
            buffer: Vec::with_capacity(256.min(max_byte_length as usize)),
            max_byte_length,
        }
    }

    fn ensure_capacity(&mut self, additional: usize) -> Result<(), CborError> {
        let required = self.buffer.len() + additional;
        if required as u64 > self.max_byte_length {
            return Err(CborError::new(format!(
                "CBOR byte length exceeds configured limit of {}",
                self.max_byte_length
            )));
        }
        Ok(())
    }

    fn write_byte(&mut self, value: u8) -> Result<(), CborError> {
        self.ensure_capacity(1)?;
        self.buffer.push(value);
        Ok(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), CborError> {
        self.ensure_capacity(bytes.len())?;
        self.buffer.extend_from_slice(bytes);
        Ok(())
    }

    fn write_uint16(&mut self, value: u16) -> Result<(), CborError> {
        self.ensure_capacity(2)?;
        self.buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn write_uint32(&mut self, value: u32) -> Result<(), CborError> {
        self.ensure_capacity(4)?;
        self.buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn write_uint64(&mut self, value: u64) -> Result<(), CborError> {
        self.ensure_capacity(8)?;
        self.buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    fn write_float64(&mut self, value: f64) -> Result<(), CborError> {
        self.ensure_capacity(9)?;
        self.buffer.push(0xfb);
        self.buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }
}

fn write_argument(writer: &mut CborWriter, major_type: u8, value: u64) -> Result<(), CborError> {
    let prefix = major_type << 5;
    if value < 24 {
        writer.write_byte(prefix | value as u8)
    } else if value <= 0xff {
        writer.write_byte(prefix | 24)?;
        writer.write_byte(value as u8)
    } else if value <= 0xffff {
        writer.write_byte(prefix | 25)?;
        writer.write_uint16(value as u16)
    } else if value <= MAX_UINT32 {
        writer.write_byte(prefix | 26)?;
        writer.write_uint32(value as u32)
    } else {
        writer.write_byte(prefix | 27)?;
        writer.write_uint64(value)
    }
}

fn encode_text(
    writer: &mut CborWriter,
    value: &str,
    options: &ResolvedCborOptions,
) -> Result<(), CborError> {
    let bytes = value.as_bytes();
    if bytes.len() as u64 > options.max_byte_length {
        return Err(CborError::new(format!(
            "CBOR text string length exceeds configured limit of {}",
            options.max_byte_length
        )));
    }
    if !value.is_char_boundary(value.len()) {
        return Err(CborError::new(
            "CBOR text strings must contain valid Unicode scalar values",
        ));
    }
    write_argument(writer, 3, bytes.len() as u64)?;
    writer.write_bytes(bytes)
}

fn encode_value(
    writer: &mut CborWriter,
    value: &CborValue,
    options: &ResolvedCborOptions,
    depth: u32,
) -> Result<(), CborError> {
    if depth > options.max_depth {
        return Err(CborError::new(format!(
            "CBOR nesting depth exceeds configured limit of {}",
            options.max_depth
        )));
    }
    match value {
        CborValue::Null => writer.write_byte(0xf6),
        CborValue::Bool(true) => writer.write_byte(0xf5),
        CborValue::Bool(false) => writer.write_byte(0xf4),
        CborValue::Integer(n) => {
            if *n > JS_MAX_SAFE_INTEGER || *n < -JS_MAX_SAFE_INTEGER {
                return Err(CborError::new(
                    "CBOR integers must be safe JavaScript integers",
                ));
            }
            if *n >= 0 {
                write_argument(writer, 0, *n as u64)
            } else {
                write_argument(writer, 1, (-1 - *n) as u64)
            }
        }
        CborValue::Float(n) => {
            if !n.is_finite() {
                return Err(CborError::new("CBOR numbers must be finite"));
            }
            writer.write_float64(*n)
        }
        CborValue::Text(s) => encode_text(writer, s, options),
        CborValue::Bytes(bytes) => {
            if bytes.len() as u64 > options.max_byte_length {
                return Err(CborError::new(format!(
                    "CBOR byte string length exceeds configured limit of {}",
                    options.max_byte_length
                )));
            }
            write_argument(writer, 2, bytes.len() as u64)?;
            writer.write_bytes(bytes)
        }
        CborValue::Array(items) => {
            if items.len() as u64 > options.max_container_length {
                return Err(CborError::new(format!(
                    "CBOR array length exceeds configured limit of {}",
                    options.max_container_length
                )));
            }
            write_argument(writer, 4, items.len() as u64)?;
            for item in items {
                encode_value(writer, item, options, depth + 1)?;
            }
            Ok(())
        }
        CborValue::Map(map) => {
            if map.len() as u64 > options.max_container_length {
                return Err(CborError::new(format!(
                    "CBOR map length exceeds configured limit of {}",
                    options.max_container_length
                )));
            }
            write_argument(writer, 5, map.len() as u64)?;
            for (key, entry) in map {
                encode_text(writer, key, options)?;
                encode_value(writer, entry, options, depth + 1)?;
            }
            Ok(())
        }
    }
}

pub fn encode_cbor(value: &CborValue, options: Option<&CborOptions>) -> Result<Vec<u8>, CborError> {
    let resolved = resolve_options(options)?;
    let mut writer = CborWriter::new(resolved.max_byte_length);
    encode_value(&mut writer, value, &resolved, 0)?;
    Ok(writer.buffer)
}

struct CborReader<'a> {
    bytes: &'a [u8],
    offset: usize,
    options: ResolvedCborOptions,
}

impl<'a> CborReader<'a> {
    fn decode(&mut self) -> Result<CborValue, CborError> {
        let value = self.read_item(0)?;
        if self.offset != self.bytes.len() {
            return Err(CborError::new("CBOR payload contains trailing data"));
        }
        Ok(value)
    }

    fn read_item(&mut self, depth: u32) -> Result<CborValue, CborError> {
        if depth > self.options.max_depth {
            return Err(CborError::new(format!(
                "CBOR nesting depth exceeds configured limit of {}",
                self.options.max_depth
            )));
        }
        let initial = self.read_byte()?;
        let major_type = initial >> 5;
        let additional = initial & 0x1f;
        match major_type {
            0 => Ok(CborValue::Integer(self.read_argument(additional)? as i64)),
            1 => {
                let n = -1 - (self.read_argument(additional)? as i64);
                if n < -JS_MAX_SAFE_INTEGER {
                    return Err(CborError::new(
                        "Decoded CBOR integer is outside the safe range",
                    ));
                }
                Ok(CborValue::Integer(n))
            }
            2 => {
                let length =
                    self.read_length(additional, "byte string", self.options.max_byte_length)?;
                Ok(CborValue::Bytes(self.read_bytes(length)?.to_vec()))
            }
            3 => {
                let length =
                    self.read_length(additional, "text string", self.options.max_byte_length)?;
                let bytes = self.read_bytes(length)?;
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| CborError::new("CBOR text string contains invalid UTF-8"))?;
                Ok(CborValue::Text(text.to_string()))
            }
            4 => {
                let length =
                    self.read_length(additional, "array", self.options.max_container_length)?;
                let mut result = Vec::with_capacity(length);
                for _ in 0..length {
                    result.push(self.read_item(depth + 1)?);
                }
                Ok(CborValue::Array(result))
            }
            5 => {
                let length =
                    self.read_length(additional, "map", self.options.max_container_length)?;
                let mut result = IndexMap::new();
                for _ in 0..length {
                    match self.read_item(depth + 1)? {
                        CborValue::Text(key) => {
                            if result.contains_key(&key) {
                                return Err(CborError::new("CBOR map contains a duplicate key"));
                            }
                            let value = self.read_item(depth + 1)?;
                            result.insert(key, value);
                        }
                        _ => return Err(CborError::new("CBOR map keys must be strings")),
                    }
                }
                Ok(CborValue::Map(result))
            }
            6 => Err(CborError::new("CBOR tags are not supported")),
            7 => self.read_simple(additional),
            _ => Err(CborError::new("Malformed CBOR major type")),
        }
    }

    fn read_simple(&mut self, additional: u8) -> Result<CborValue, CborError> {
        match additional {
            20 => Ok(CborValue::Bool(false)),
            21 => Ok(CborValue::Bool(true)),
            22 => Ok(CborValue::Null),
            27 => {
                let bytes = self.read_bytes(8)?;
                let mut raw = [0u8; 8];
                raw.copy_from_slice(bytes);
                let value = f64::from_be_bytes(raw);
                if !value.is_finite() {
                    return Err(CborError::new("Decoded CBOR number must be finite"));
                }
                if value.fract() == 0.0
                    && value.is_sign_positive()
                    && value.abs() as i64 > JS_MAX_SAFE_INTEGER
                {
                    return Err(CborError::new(
                        "Decoded CBOR integer is outside the safe range",
                    ));
                }
                if value.fract() == 0.0
                    && !is_neg_zero(value)
                    && value.abs() <= JS_MAX_SAFE_INTEGER as f64
                {
                    Ok(CborValue::Integer(value as i64))
                } else {
                    Ok(CborValue::Float(value))
                }
            }
            31 => Err(CborError::new("CBOR break marker is not supported")),
            _ => Err(CborError::new(
                "Unsupported CBOR simple value or floating-point width",
            )),
        }
    }

    fn read_length(&mut self, additional: u8, kind: &str, limit: u64) -> Result<usize, CborError> {
        if additional == 31 {
            return Err(CborError::new(format!(
                "Indefinite-length CBOR {kind}s are not supported"
            )));
        }
        let length = self.read_argument(additional)?;
        if length > limit {
            return Err(CborError::new(format!(
                "CBOR {kind} length exceeds configured limit of {limit}"
            )));
        }
        Ok(length as usize)
    }

    fn read_argument(&mut self, additional: u8) -> Result<u64, CborError> {
        if additional < 24 {
            return Ok(u64::from(additional));
        }
        match additional {
            24 => Ok(u64::from(self.read_byte()?)),
            25 => {
                let bytes = self.read_bytes(2)?;
                Ok(u64::from(bytes[0]) * 0x100 + u64::from(bytes[1]))
            }
            26 => {
                let bytes = self.read_bytes(4)?;
                Ok(u64::from(bytes[0]) * 0x1_000_000
                    + u64::from(bytes[1]) * 0x1_0000
                    + u64::from(bytes[2]) * 0x100
                    + u64::from(bytes[3]))
            }
            27 => {
                let high = self.read_argument(26)?;
                let low = self.read_argument(26)?;
                if high > 0x1f_ffff {
                    return Err(CborError::new(
                        "Decoded CBOR integer or length is outside the safe range",
                    ));
                }
                Ok(high * UINT32_BASE + low)
            }
            31 => Err(CborError::new(
                "Indefinite-length CBOR items are not supported",
            )),
            _ => Err(CborError::new("Malformed CBOR additional information")),
        }
    }

    fn read_byte(&mut self) -> Result<u8, CborError> {
        if self.offset >= self.bytes.len() {
            return Err(CborError::new("Truncated CBOR payload"));
        }
        let value = self.bytes[self.offset];
        self.offset += 1;
        Ok(value)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], CborError> {
        if length > self.bytes.len() - self.offset {
            return Err(CborError::new("Truncated CBOR payload"));
        }
        let value = &self.bytes[self.offset..self.offset + length];
        self.offset += length;
        Ok(value)
    }
}

fn is_neg_zero(value: f64) -> bool {
    value == 0.0 && value.is_sign_negative()
}

pub fn decode_cbor(bytes: &[u8], options: Option<&CborOptions>) -> Result<CborValue, CborError> {
    let resolved = resolve_options(options)?;
    if bytes.len() as u64 > resolved.max_byte_length {
        return Err(CborError::new(format!(
            "CBOR byte length exceeds configured limit of {}",
            resolved.max_byte_length
        )));
    }
    CborReader {
        bytes,
        offset: 0,
        options: resolved,
    }
    .decode()
}

pub fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub fn from_hex(hex: &str) -> Result<Vec<u8>, CborError> {
    if hex.len() % 2 != 0 {
        return Err(CborError::new("Hex fixture must contain whole bytes"));
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16)
                .map_err(|_| CborError::new("Hex fixture must contain whole bytes"))
        })
        .collect()
}
