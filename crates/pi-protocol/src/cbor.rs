use thiserror::Error;

pub const MAX_UINT32: u64 = 0xffff_ffff;
const MAX_CONFIGURED_DEPTH: u32 = 512;

pub const DEFAULT_MAX_CBOR_BYTE_LENGTH: u64 = 16 * 1024 * 1024;
pub const DEFAULT_MAX_CBOR_CONTAINER_LENGTH: u64 = 1_000_000;
pub const DEFAULT_MAX_CBOR_DEPTH: u32 = 64;

#[derive(Debug, Clone, Copy, Default)]
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

#[derive(Debug, Error)]
pub enum CborError {
    #[error("{0}")]
    Message(String),
}

impl CborError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum CborValue {
    Null,
    Bool(bool),
    Integer(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Array(Vec<CborValue>),
    Map(Vec<(String, CborValue)>),
}

impl CborValue {
    pub fn from_json(value: &serde_json::Value) -> Result<Self, CborError> {
        match value {
            serde_json::Value::Null => Ok(Self::Null),
            serde_json::Value::Bool(v) => Ok(Self::Bool(*v)),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    Ok(Self::Integer(i))
                } else if let Some(u) = n.as_u64() {
                    if u > i64::MAX as u64 {
                        return Err(CborError::new(
                            "CBOR integers must be safe JavaScript integers",
                        ));
                    }
                    Ok(Self::Integer(u as i64))
                } else if let Some(f) = n.as_f64() {
                    if !f.is_finite() {
                        return Err(CborError::new("CBOR numbers must be finite"));
                    }
                    Ok(Self::Float(f))
                } else {
                    Err(CborError::new("Unsupported CBOR value type: number"))
                }
            }
            serde_json::Value::String(s) => Ok(Self::Text(s.clone())),
            serde_json::Value::Array(items) => Ok(Self::Array(
                items
                    .iter()
                    .map(Self::from_json)
                    .collect::<Result<Vec<_>, _>>()?,
            )),
            serde_json::Value::Object(map) => {
                let mut out = Vec::with_capacity(map.len());
                for (key, value) in map {
                    out.push((key.clone(), Self::from_json(value)?));
                }
                Ok(Self::Map(out))
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
            Self::Text(v) => serde_json::Value::String(v.clone()),
            Self::Bytes(v) => serde_json::Value::Array(
                v.iter()
                    .map(|b| serde_json::Value::Number((*b).into()))
                    .collect(),
            ),
            Self::Array(items) => {
                serde_json::Value::Array(items.iter().map(Self::to_json).collect())
            }
            Self::Map(map) => {
                let mut object = serde_json::Map::new();
                for (key, value) in map {
                    object.insert(key.clone(), value.to_json());
                }
                serde_json::Value::Object(object)
            }
        }
    }
}

fn resolve_limit(name: &str, value: u64, maximum: u64) -> Result<u64, CborError> {
    if value > maximum {
        return Err(CborError::new(format!(
            "{name} must be an integer between 0 and {maximum}"
        )));
    }
    Ok(value)
}

pub fn resolve_options(options: Option<CborOptions>) -> Result<ResolvedCborOptions, CborError> {
    let options = options.unwrap_or_default();
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
        self.write_bytes(&value.to_be_bytes())
    }

    fn write_uint32(&mut self, value: u32) -> Result<(), CborError> {
        self.write_bytes(&value.to_be_bytes())
    }

    fn write_uint64(&mut self, value: u64) -> Result<(), CborError> {
        self.write_bytes(&value.to_be_bytes())
    }

    fn write_float64(&mut self, value: f64) -> Result<(), CborError> {
        self.write_byte(0xfb)?;
        self.write_bytes(&value.to_be_bytes())
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
            if !(-9007199254740991..=9007199254740991).contains(n) {
                return Err(CborError::new(
                    "CBOR integers must be safe JavaScript integers",
                ));
            }
            if *n >= 0 {
                write_argument(writer, 0, *n as u64)
            } else {
                write_argument(writer, 1, (-1 - n) as u64)
            }
        }
        CborValue::Float(n) => {
            if !n.is_finite() {
                return Err(CborError::new("CBOR numbers must be finite"));
            }
            writer.write_float64(*n)
        }
        CborValue::Text(text) => encode_text(writer, text, options),
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
            for (key, value) in map {
                encode_text(writer, key, options)?;
                encode_value(writer, value, options, depth + 1)?;
            }
            Ok(())
        }
    }
}

pub fn encode_cbor(value: &CborValue, options: Option<CborOptions>) -> Result<Vec<u8>, CborError> {
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
    fn read_byte(&mut self) -> Result<u8, CborError> {
        let byte = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| CborError::new("Unexpected end of CBOR payload"))?;
        self.offset += 1;
        Ok(byte)
    }

    fn read_bytes(&mut self, length: usize) -> Result<&'a [u8], CborError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| CborError::new("CBOR length overflow"))?;
        let slice = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| CborError::new("Unexpected end of CBOR payload"))?;
        self.offset = end;
        Ok(slice)
    }

    fn read_argument(&mut self, additional: u8) -> Result<u64, CborError> {
        match additional {
            n if n < 24 => Ok(u64::from(n)),
            24 => Ok(u64::from(self.read_byte()?)),
            25 => {
                let bytes = self.read_bytes(2)?;
                Ok(u64::from(u16::from_be_bytes([bytes[0], bytes[1]])))
            }
            26 => {
                let bytes = self.read_bytes(4)?;
                Ok(u64::from(u32::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3],
                ])))
            }
            27 => {
                let bytes = self.read_bytes(8)?;
                Ok(u64::from_be_bytes([
                    bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
                ]))
            }
            _ => Err(CborError::new("Unsupported CBOR additional information")),
        }
    }

    fn read_length(
        &mut self,
        additional: u8,
        kind: &str,
        maximum: u64,
    ) -> Result<usize, CborError> {
        let length = self.read_argument(additional)?;
        if length > maximum {
            return Err(CborError::new(format!(
                "CBOR {kind} length exceeds configured limit of {maximum}"
            )));
        }
        usize::try_from(length).map_err(|_| CborError::new("CBOR length overflow"))
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
            0 => {
                let value = self.read_argument(additional)?;
                if value > 9007199254740991 {
                    return Err(CborError::new(
                        "Decoded CBOR integer is outside the safe range",
                    ));
                }
                Ok(CborValue::Integer(value as i64))
            }
            1 => {
                let argument = self.read_argument(additional)?;
                let value = -1 - (argument as i64);
                if !(-9007199254740991..=9007199254740991).contains(&value) {
                    return Err(CborError::new(
                        "Decoded CBOR integer is outside the safe range",
                    ));
                }
                Ok(CborValue::Integer(value))
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
                let text = std::str::from_utf8(bytes).map_err(|_| {
                    CborError::new("CBOR text strings must contain valid Unicode scalar values")
                })?;
                Ok(CborValue::Text(text.to_string()))
            }
            4 => {
                let length =
                    self.read_length(additional, "array", self.options.max_container_length)?;
                let mut items = Vec::with_capacity(length);
                for _ in 0..length {
                    items.push(self.read_item(depth + 1)?);
                }
                Ok(CborValue::Array(items))
            }
            5 => {
                let length =
                    self.read_length(additional, "map", self.options.max_container_length)?;
                let mut map = Vec::with_capacity(length);
                for _ in 0..length {
                    let key = match self.read_item(depth + 1)? {
                        CborValue::Text(key) => key,
                        _ => return Err(CborError::new("CBOR map keys must be strings")),
                    };
                    map.push((key, self.read_item(depth + 1)?));
                }
                Ok(CborValue::Map(map))
            }
            7 => match additional {
                20 => Ok(CborValue::Bool(false)),
                21 => Ok(CborValue::Bool(true)),
                22 => Ok(CborValue::Null),
                27 => {
                    let bytes = self.read_bytes(8)?;
                    let value = f64::from_be_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                        bytes[7],
                    ]);
                    if !value.is_finite() {
                        return Err(CborError::new("CBOR numbers must be finite"));
                    }
                    Ok(CborValue::Float(value))
                }
                _ => Err(CborError::new("Unsupported CBOR simple value")),
            },
            _ => Err(CborError::new("Unsupported CBOR major type")),
        }
    }
}

pub fn decode_cbor(bytes: &[u8], options: Option<CborOptions>) -> Result<CborValue, CborError> {
    let resolved = resolve_options(options)?;
    if bytes.len() as u64 > resolved.max_byte_length {
        return Err(CborError::new(format!(
            "CBOR byte length exceeds configured limit of {}",
            resolved.max_byte_length
        )));
    }
    let mut reader = CborReader {
        bytes,
        offset: 0,
        options: resolved,
    };
    let value = reader.read_item(0)?;
    if reader.offset != bytes.len() {
        return Err(CborError::new("CBOR payload contains trailing data"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_hello_map() {
        let map = vec![
            ("type".to_string(), CborValue::Text("hello".into())),
            ("version".to_string(), CborValue::Integer(1)),
        ];
        let encoded = encode_cbor(&CborValue::Map(map.clone()), None).unwrap();
        assert_eq!(decode_cbor(&encoded, None).unwrap(), CborValue::Map(map));
    }

    #[test]
    fn rejects_trailing_data() {
        let encoded = encode_cbor(&CborValue::Null, None).unwrap();
        let mut trailing = encoded;
        trailing.push(0xf6);
        let error = decode_cbor(&trailing, None).unwrap_err();
        assert_eq!(error.to_string(), "CBOR payload contains trailing data");
    }
}
