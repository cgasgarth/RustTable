use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

use crate::operation::model::{CodecField, ParameterCodec};

#[derive(Debug, Clone, PartialEq)]
pub enum ParameterValue {
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float(f64),
    Bytes(Vec<u8>),
    Array(Vec<ParameterValue>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedParameter {
    pub fields: BTreeMap<String, ParameterValue>,
    pub raw: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    InvalidCodec(String),
    InvalidPayload { expected: usize, actual: usize },
    MissingField(String),
    InvalidValue(String),
    UnknownEnumValue { field: String, value: i64 },
}

impl Display for CodecError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCodec(message) => write!(formatter, "invalid parameter codec: {message}"),
            Self::InvalidPayload { expected, actual } => {
                write!(
                    formatter,
                    "invalid parameter payload length: expected {expected}, got {actual}"
                )
            }
            Self::MissingField(field) => write!(formatter, "parameter field is missing: {field}"),
            Self::InvalidValue(message) => write!(formatter, "invalid parameter value: {message}"),
            Self::UnknownEnumValue { field, value } => {
                write!(formatter, "unknown enum value {value} for {field}")
            }
        }
    }
}

impl std::error::Error for CodecError {}

/// Decodes a current persisted parameter payload without executing reference code.
///
/// The original bytes are retained so implicit and explicit padding survives a
/// decode/encode round trip exactly.
///
/// # Errors
///
/// Returns an error when the codec is incomplete, the payload size is wrong,
/// or a field cannot be decoded according to its declared ABI type.
pub fn decode_parameter(
    codec: &ParameterCodec,
    payload: &[u8],
) -> Result<DecodedParameter, CodecError> {
    validate_codec(codec)?;
    if payload.len() != codec.byte_size {
        return Err(CodecError::InvalidPayload {
            expected: codec.byte_size,
            actual: payload.len(),
        });
    }
    let mut fields = BTreeMap::new();
    for field in &codec.fields {
        let bytes = &payload[field.offset..field.offset + field.size];
        let value = decode_field(field, bytes, &codec.byte_order)?;
        fields.insert(field.name.clone(), value);
    }
    Ok(DecodedParameter {
        fields,
        raw: payload.to_owned(),
    })
}

/// Encodes a typed parameter payload using the exact ABI byte layout.
///
/// # Errors
///
/// Returns an error when the codec is incomplete, raw padding is the wrong
/// size, a field is missing, or a value cannot be represented by its ABI type.
pub fn encode_parameter(
    codec: &ParameterCodec,
    parameter: &DecodedParameter,
) -> Result<Vec<u8>, CodecError> {
    validate_codec(codec)?;
    if parameter.raw.len() != codec.byte_size {
        return Err(CodecError::InvalidPayload {
            expected: codec.byte_size,
            actual: parameter.raw.len(),
        });
    }
    let mut payload = parameter.raw.clone();
    for field in &codec.fields {
        let value = parameter
            .fields
            .get(&field.name)
            .ok_or_else(|| CodecError::MissingField(field.name.clone()))?;
        encode_field(
            field,
            value,
            &codec.byte_order,
            &mut payload[field.offset..field.offset + field.size],
        )?;
    }
    Ok(payload)
}

fn validate_codec(codec: &ParameterCodec) -> Result<(), CodecError> {
    if codec.byte_size == 0
        || !matches!(codec.byte_order.as_str(), "little" | "big")
        || codec.decoder.trim().is_empty()
        || codec.encoder.trim().is_empty()
        || codec.fields.is_empty()
    {
        return Err(CodecError::InvalidCodec(
            "current codec needs a byte size, explicit byte order, decoder, encoder, and fields"
                .to_owned(),
        ));
    }
    let mut ranges = Vec::new();
    for field in &codec.fields {
        validate_field(field, codec.byte_size)?;
        let end = field.offset + field.size;
        if ranges
            .iter()
            .any(|(start, stop): &(usize, usize)| field.offset < *stop && end > *start)
        {
            return Err(CodecError::InvalidCodec(format!(
                "overlapping field {}",
                field.name
            )));
        }
        ranges.push((field.offset, end));
    }
    Ok(())
}

fn validate_field(field: &CodecField, byte_size: usize) -> Result<(), CodecError> {
    if field.name.trim().is_empty() || field.size == 0 || field.offset + field.size > byte_size {
        return Err(CodecError::InvalidCodec(format!(
            "field {} is outside the payload",
            field.name
        )));
    }
    let (width, array) = scalar_width(&field.kind).ok_or_else(|| {
        CodecError::InvalidCodec(format!("unsupported field kind {}", field.kind))
    })?;
    let extent = field.array_extent.unwrap_or(1);
    if extent == 0 || field.size != width * extent {
        return Err(CodecError::InvalidCodec(format!(
            "field {} size does not match its kind and extent",
            field.name
        )));
    }
    if array && field.array_extent.is_none() {
        return Err(CodecError::InvalidCodec(format!(
            "array field {} is missing its extent",
            field.name
        )));
    }
    Ok(())
}

fn scalar_width(kind: &str) -> Option<(usize, bool)> {
    Some(match kind {
        "bool" | "i8" | "u8" => (1, false),
        "glib_bool" | "i32" | "u32" | "enum_i32" | "f32" => (4, false),
        "i16" | "u16" => (2, false),
        "i64" | "u64" | "f64" | "enum_i64" => (8, false),
        "bytes" => (1, true),
        _ => return None,
    })
}

fn decode_field(
    field: &CodecField,
    bytes: &[u8],
    byte_order: &str,
) -> Result<ParameterValue, CodecError> {
    let extent = field.array_extent.unwrap_or(1);
    if extent > 1 {
        let width = bytes.len() / extent;
        let mut values = Vec::with_capacity(extent);
        for chunk in bytes.chunks_exact(width) {
            values.push(decode_scalar(field, chunk, byte_order)?);
        }
        return Ok(ParameterValue::Array(values));
    }
    decode_scalar(field, bytes, byte_order)
}

fn decode_scalar(
    field: &CodecField,
    bytes: &[u8],
    byte_order: &str,
) -> Result<ParameterValue, CodecError> {
    let value = match field.kind.as_str() {
        "bool" => ParameterValue::Boolean(bytes[0] != 0),
        "glib_bool" => ParameterValue::Boolean(read_unsigned(bytes, byte_order) != 0),
        "i8" => ParameterValue::Signed(i64::from(i8::from_ne_bytes([bytes[0]]))),
        "u8" => ParameterValue::Unsigned(u64::from(bytes[0])),
        "i16" | "i32" | "enum_i32" | "i64" | "enum_i64" => {
            ParameterValue::Signed(read_signed(bytes, byte_order))
        }
        "u16" | "u32" | "u64" => ParameterValue::Unsigned(read_unsigned(bytes, byte_order)),
        "f32" => {
            let bits = u32::try_from(read_unsigned(bytes, byte_order))
                .map_err(|_| CodecError::InvalidValue(field.name.clone()))?;
            ParameterValue::Float(f64::from(f32::from_bits(bits)))
        }
        "f64" => ParameterValue::Float(f64::from_bits(read_unsigned(bytes, byte_order))),
        "bytes" => ParameterValue::Bytes(bytes.to_owned()),
        kind => {
            return Err(CodecError::InvalidCodec(format!(
                "unsupported field kind {kind}"
            )));
        }
    };
    if matches!(field.kind.as_str(), "enum_i32" | "enum_i64") {
        let ParameterValue::Signed(signed) = value else {
            unreachable!()
        };
        if !field.enum_values.is_empty()
            && !field.enum_values.iter().any(|item| item.value == signed)
        {
            return Err(CodecError::UnknownEnumValue {
                field: field.name.clone(),
                value: signed,
            });
        }
    }
    Ok(value)
}

fn encode_field(
    field: &CodecField,
    value: &ParameterValue,
    byte_order: &str,
    destination: &mut [u8],
) -> Result<(), CodecError> {
    let values = match (field.array_extent.unwrap_or(1), value) {
        (1, value) => vec![value],
        (extent, ParameterValue::Array(values)) if extent == values.len() => {
            values.iter().collect()
        }
        (extent, _) => {
            return Err(CodecError::InvalidValue(format!(
                "field {} requires an array of {extent}",
                field.name
            )));
        }
    };
    let width = destination.len() / values.len();
    for (index, value) in values.into_iter().enumerate() {
        encode_scalar(
            field,
            value,
            byte_order,
            &mut destination[index * width..(index + 1) * width],
        )?;
    }
    Ok(())
}

fn encode_scalar(
    field: &CodecField,
    value: &ParameterValue,
    byte_order: &str,
    destination: &mut [u8],
) -> Result<(), CodecError> {
    match field.kind.as_str() {
        "bool" | "glib_bool" => {
            let value = match value {
                ParameterValue::Boolean(value) => u64::from(*value),
                _ => return Err(CodecError::InvalidValue(field.name.clone())),
            };
            write_unsigned(destination, value, byte_order)
        }
        "i8" | "i16" | "i32" | "i64" | "enum_i32" | "enum_i64" => {
            let value = match value {
                ParameterValue::Signed(value) => *value,
                ParameterValue::Unsigned(value) => i64::try_from(*value)
                    .map_err(|_| CodecError::InvalidValue(field.name.clone()))?,
                _ => return Err(CodecError::InvalidValue(field.name.clone())),
            };
            if matches!(field.kind.as_str(), "enum_i32" | "enum_i64")
                && !field.enum_values.is_empty()
                && !field.enum_values.iter().any(|item| item.value == value)
            {
                return Err(CodecError::UnknownEnumValue {
                    field: field.name.clone(),
                    value,
                });
            }
            write_signed(destination, value, byte_order)
        }
        "u8" | "u16" | "u32" | "u64" => {
            let value = match value {
                ParameterValue::Unsigned(value) => *value,
                ParameterValue::Signed(value) if *value >= 0 => u64::try_from(*value)
                    .map_err(|_| CodecError::InvalidValue(field.name.clone()))?,
                _ => return Err(CodecError::InvalidValue(field.name.clone())),
            };
            write_unsigned(destination, value, byte_order)
        }
        "f32" | "f64" => {
            let value = match value {
                ParameterValue::Float(value) => *value,
                _ => return Err(CodecError::InvalidValue(field.name.clone())),
            };
            write_unsigned(
                destination,
                if field.kind == "f32" {
                    #[allow(clippy::cast_possible_truncation)]
                    let narrowed = value as f32;
                    u64::from(narrowed.to_bits())
                } else {
                    value.to_bits()
                },
                byte_order,
            )
        }
        "bytes" => match value {
            ParameterValue::Bytes(value) if value.len() == destination.len() => {
                destination.copy_from_slice(value);
                Ok(())
            }
            _ => Err(CodecError::InvalidValue(field.name.clone())),
        },
        kind => Err(CodecError::InvalidCodec(format!(
            "unsupported field kind {kind}"
        ))),
    }
}

fn read_unsigned(bytes: &[u8], byte_order: &str) -> u64 {
    let mut value = 0_u64;
    if byte_order == "little" {
        for (index, byte) in bytes.iter().enumerate() {
            value |= u64::from(*byte) << (index * 8);
        }
    } else {
        for byte in bytes {
            value = (value << 8) | u64::from(*byte);
        }
    }
    value
}

fn read_signed(bytes: &[u8], byte_order: &str) -> i64 {
    let unsigned = read_unsigned(bytes, byte_order);
    let bits = bytes.len() * 8;
    if bits < 64 && (unsigned & (1_u64 << (bits - 1))) != 0 {
        i64::from_ne_bytes((unsigned | (!0_u64 << bits)).to_ne_bytes())
    } else {
        i64::from_ne_bytes(unsigned.to_ne_bytes())
    }
}

fn write_unsigned(destination: &mut [u8], value: u64, byte_order: &str) -> Result<(), CodecError> {
    if destination.len() > 8
        || (destination.len() < 8 && value >= (1_u64 << (destination.len() * 8)))
    {
        return Err(CodecError::InvalidValue(format!(
            "value does not fit in {} bytes",
            destination.len()
        )));
    }
    if byte_order == "little" {
        for (index, byte) in destination.iter_mut().enumerate() {
            *byte = ((value >> (index * 8)) & 0xff) as u8;
        }
    } else {
        let length = destination.len();
        for (index, byte) in destination.iter_mut().enumerate() {
            *byte = ((value >> ((length - index - 1) * 8)) & 0xff) as u8;
        }
    }
    Ok(())
}

fn write_signed(destination: &mut [u8], value: i64, byte_order: &str) -> Result<(), CodecError> {
    let bits = destination.len() * 8;
    if bits < 64 {
        let minimum = -(1_i64 << (bits - 1));
        let maximum = (1_i64 << (bits - 1)) - 1;
        if !(minimum..=maximum).contains(&value) {
            return Err(CodecError::InvalidValue(format!(
                "value does not fit in {} signed bytes",
                destination.len()
            )));
        }
    }
    let raw = u64::from_ne_bytes(value.to_ne_bytes());
    let raw = if bits < 64 {
        raw & ((1_u64 << bits) - 1)
    } else {
        raw
    };
    write_unsigned(destination, raw, byte_order)
}
