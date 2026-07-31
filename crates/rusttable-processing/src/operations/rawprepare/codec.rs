//! Exact parameter bytes and the one retained `rawprepare.c` migration.
//!
//! Direct source lineage: `src/iop/rawprepare.c:38-55` and
//! `src/iop/rawprepare.c:121-163`.  The C declarations have a 28-byte v1
//! native struct (including the white point at offset 24 and natural padding)
//! and a 32-byte v2 native struct on the supported ABIs.  The repository's
//! operation manifest carries those payloads in 296-byte and 40-byte history
//! slots respectively; the slot tails are retained verbatim and are never
//! interpreted as camera data.

#![forbid(unsafe_code)]
#![allow(
    clippy::large_enum_variant,
    clippy::missing_errors_doc,
    clippy::similar_names
)]

use std::fmt;

pub const RAWPREPARE_COMPATIBILITY_ID: &str = "rawprepare";
pub const RAWPREPARE_INTROSPECTION_VERSION: u16 = 2;
pub const RAWPREPARE_NATIVE_V1_PARAMETER_BYTES: usize = 28;
pub const RAWPREPARE_NATIVE_V2_PARAMETER_BYTES: usize = 32;
pub const RAWPREPARE_HISTORY_V1_PARAMETER_BYTES: usize = 296;
pub const RAWPREPARE_HISTORY_V2_PARAMETER_BYTES: usize = 40;
/// Current Darktable history slot size, including its opaque tail.
pub const RAWPREPARE_PARAMETER_BYTES: usize = RAWPREPARE_HISTORY_V2_PARAMETER_BYTES;
/// Compatibility alias for operation-manifest consumers.
pub const RAWPREPARE_V1_PARAMETER_BYTES: usize = RAWPREPARE_HISTORY_V1_PARAMETER_BYTES;
/// Compatibility alias for operation-manifest consumers.
pub const RAWPREPARE_V2_PARAMETER_BYTES: usize = RAWPREPARE_HISTORY_V2_PARAMETER_BYTES;

const BLACK_LEVEL_OFFSET: usize = 16;
const WHITE_POINT_OFFSET: usize = 24;
const FLAT_FIELD_OFFSET: usize = 28;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RawPrepareFlatField {
    Off = 0,
    Embedded = 1,
}

impl RawPrepareFlatField {
    fn from_i32(value: i32) -> Result<Self, RawPrepareCodecError> {
        match value {
            0 => Ok(Self::Off),
            1 => Ok(Self::Embedded),
            _ => Err(RawPrepareCodecError::UnknownFlatField(value)),
        }
    }
}

/// The source's v1 declaration, transported in the manifest's opaque slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPrepareParametersV1 {
    bytes: [u8; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES],
}

impl RawPrepareParametersV1 {
    /// Creates a v1 payload with the native declaration at its front.
    #[must_use]
    pub fn new(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        raw_black_level_separate: [u16; 4],
        raw_white_point: u16,
    ) -> Self {
        let mut bytes = [0; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES];
        write_i32(&mut bytes, 0, left);
        write_i32(&mut bytes, 4, top);
        write_i32(&mut bytes, 8, right);
        write_i32(&mut bytes, 12, bottom);
        write_u16_array(&mut bytes, BLACK_LEVEL_OFFSET, raw_black_level_separate);
        write_u16(&mut bytes, WHITE_POINT_OFFSET, raw_white_point);
        Self { bytes }
    }

    /// Decodes either the native declaration or its manifest history slot.
    /// Native-size input is widened with zeroed opaque tail bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RawPrepareCodecError> {
        if bytes.len() != RAWPREPARE_NATIVE_V1_PARAMETER_BYTES
            && bytes.len() != RAWPREPARE_HISTORY_V1_PARAMETER_BYTES
        {
            return Err(RawPrepareCodecError::InvalidLength {
                version: 1,
                expected: RAWPREPARE_HISTORY_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut payload = [0; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES];
        payload[..bytes.len()].copy_from_slice(bytes);
        Ok(Self { bytes: payload })
    }

    /// Decodes exactly the native v1 declaration.
    pub fn from_native_bytes(
        bytes: &[u8; RAWPREPARE_NATIVE_V1_PARAMETER_BYTES],
    ) -> Result<Self, RawPrepareCodecError> {
        Self::from_bytes(bytes)
    }

    #[must_use]
    pub fn to_bytes(&self) -> &[u8; RAWPREPARE_HISTORY_V1_PARAMETER_BYTES] {
        &self.bytes
    }

    #[must_use]
    pub fn to_native_bytes(&self) -> [u8; RAWPREPARE_NATIVE_V1_PARAMETER_BYTES] {
        self.bytes[..RAWPREPARE_NATIVE_V1_PARAMETER_BYTES]
            .try_into()
            .expect("native v1 range")
    }

    #[must_use]
    pub fn left(&self) -> i32 {
        read_i32(&self.bytes, 0)
    }

    #[must_use]
    pub fn top(&self) -> i32 {
        read_i32(&self.bytes, 4)
    }

    #[must_use]
    pub fn right(&self) -> i32 {
        read_i32(&self.bytes, 8)
    }

    #[must_use]
    pub fn bottom(&self) -> i32 {
        read_i32(&self.bytes, 12)
    }

    #[must_use]
    pub fn raw_black_level_separate(&self) -> [u16; 4] {
        read_u16_array(&self.bytes, BLACK_LEVEL_OFFSET)
    }

    #[must_use]
    pub fn raw_white_point(&self) -> u16 {
        read_u16(&self.bytes, WHITE_POINT_OFFSET)
    }
}

/// The current v2 declaration plus the manifest-preserved opaque tail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPrepareParametersV2 {
    bytes: [u8; RAWPREPARE_HISTORY_V2_PARAMETER_BYTES],
}

impl RawPrepareParametersV2 {
    #[must_use]
    pub fn new(
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
        raw_black_level_separate: [u16; 4],
        raw_white_point: u16,
        flat_field: RawPrepareFlatField,
    ) -> Self {
        let mut bytes = [0; RAWPREPARE_HISTORY_V2_PARAMETER_BYTES];
        write_i32(&mut bytes, 0, left);
        write_i32(&mut bytes, 4, top);
        write_i32(&mut bytes, 8, right);
        write_i32(&mut bytes, 12, bottom);
        write_u16_array(&mut bytes, BLACK_LEVEL_OFFSET, raw_black_level_separate);
        write_u16(&mut bytes, WHITE_POINT_OFFSET, raw_white_point);
        write_i32(&mut bytes, FLAT_FIELD_OFFSET, flat_field as i32);
        Self { bytes }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RawPrepareCodecError> {
        if bytes.len() != RAWPREPARE_NATIVE_V2_PARAMETER_BYTES
            && bytes.len() != RAWPREPARE_HISTORY_V2_PARAMETER_BYTES
        {
            return Err(RawPrepareCodecError::InvalidLength {
                version: 2,
                expected: RAWPREPARE_HISTORY_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut payload = [0; RAWPREPARE_HISTORY_V2_PARAMETER_BYTES];
        payload[..bytes.len()].copy_from_slice(bytes);
        let flat_field = RawPrepareFlatField::from_i32(read_i32(&payload, FLAT_FIELD_OFFSET))?;
        let _ = flat_field;
        Ok(Self { bytes: payload })
    }

    /// Decodes exactly the native v2 declaration.
    pub fn from_native_bytes(
        bytes: &[u8; RAWPREPARE_NATIVE_V2_PARAMETER_BYTES],
    ) -> Result<Self, RawPrepareCodecError> {
        Self::from_bytes(bytes)
    }

    #[must_use]
    pub fn to_bytes(&self) -> &[u8; RAWPREPARE_HISTORY_V2_PARAMETER_BYTES] {
        &self.bytes
    }

    #[must_use]
    pub fn to_native_bytes(&self) -> [u8; RAWPREPARE_NATIVE_V2_PARAMETER_BYTES] {
        self.bytes[..RAWPREPARE_NATIVE_V2_PARAMETER_BYTES]
            .try_into()
            .expect("native v2 range")
    }

    #[must_use]
    pub fn left(&self) -> i32 {
        read_i32(&self.bytes, 0)
    }

    #[must_use]
    pub fn top(&self) -> i32 {
        read_i32(&self.bytes, 4)
    }

    #[must_use]
    pub fn right(&self) -> i32 {
        read_i32(&self.bytes, 8)
    }

    #[must_use]
    pub fn bottom(&self) -> i32 {
        read_i32(&self.bytes, 12)
    }

    #[must_use]
    pub fn raw_black_level_separate(&self) -> [u16; 4] {
        read_u16_array(&self.bytes, BLACK_LEVEL_OFFSET)
    }

    #[must_use]
    pub fn raw_white_point(&self) -> u16 {
        read_u16(&self.bytes, WHITE_POINT_OFFSET)
    }

    #[must_use]
    pub fn flat_field(&self) -> RawPrepareFlatField {
        RawPrepareFlatField::from_i32(read_i32(&self.bytes, FLAT_FIELD_OFFSET))
            .expect("v2 payload validated at construction")
    }

    /// Returns the bytes after the native v2 declaration.  They are retained
    /// for round trips but intentionally have no semantic meaning here.
    #[must_use]
    pub fn opaque_tail(&self) -> &[u8] {
        &self.bytes[RAWPREPARE_NATIVE_V2_PARAMETER_BYTES..]
    }
}

/// History payloads are decoded only at the operation-local boundary. Unknown
/// versions stay opaque and cannot be executed by a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPrepareHistory {
    V1(RawPrepareParametersV1),
    V2(RawPrepareParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl RawPrepareHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, RawPrepareCodecError> {
        match version {
            1 => Ok(Self::V1(RawPrepareParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(RawPrepareParametersV2::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::Opaque { version, .. } => *version,
        }
    }

    /// Applies the native `legacy_params` branch: copy only the v1 native
    /// declaration and append `FLAT_FIELD_OFF` to the v2 declaration.
    pub fn migrate_to_v2(&self) -> Result<RawPrepareParametersV2, RawPrepareCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(parameters)),
            Self::V2(parameters) => Ok(parameters.clone()),
            Self::Opaque { version, .. } => Err(RawPrepareCodecError::OpaqueVersion(*version)),
        }
    }
}

/// Performs the native-size migration without a history-slot wrapper.
pub fn migrate_native_v1_to_v2(
    v1: &[u8; RAWPREPARE_NATIVE_V1_PARAMETER_BYTES],
) -> Result<[u8; RAWPREPARE_NATIVE_V2_PARAMETER_BYTES], RawPrepareCodecError> {
    let v1 = RawPrepareParametersV1::from_native_bytes(v1)?;
    Ok(migrate_v1_to_v2(&v1).to_native_bytes())
}

#[must_use]
pub fn migrate_v1_to_v2(v1: &RawPrepareParametersV1) -> RawPrepareParametersV2 {
    // `legacy_params` uses `memcpy(sizeof *o)`, so the native v1 padding at
    // offsets 26..27 is part of the migrated byte contract even though it has
    // no named Rust field. Copy the complete native prefix before appending
    // the v2 flat-field field rather than reconstructing named values.
    let mut bytes = [0; RAWPREPARE_HISTORY_V2_PARAMETER_BYTES];
    bytes[..RAWPREPARE_NATIVE_V1_PARAMETER_BYTES]
        .copy_from_slice(&v1.bytes[..RAWPREPARE_NATIVE_V1_PARAMETER_BYTES]);
    write_i32(
        &mut bytes,
        FLAT_FIELD_OFFSET,
        RawPrepareFlatField::Off as i32,
    );
    debug_assert_eq!(
        &bytes[..RAWPREPARE_NATIVE_V1_PARAMETER_BYTES],
        &v1.bytes[..RAWPREPARE_NATIVE_V1_PARAMETER_BYTES]
    );
    RawPrepareParametersV2 { bytes }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPrepareCodecError {
    InvalidLength {
        version: u16,
        expected: usize,
        actual: usize,
    },
    UnknownFlatField(i32),
    OpaqueVersion(u16),
}

impl fmt::Display for RawPrepareCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength {
                version,
                expected,
                actual,
            } => write!(
                formatter,
                "rawprepare v{version} payload has {actual} bytes; expected {expected}"
            ),
            Self::UnknownFlatField(value) => {
                write!(formatter, "rawprepare flat field value {value} is unknown")
            }
            Self::OpaqueVersion(version) => {
                write!(formatter, "rawprepare history version {version} is opaque")
            }
        }
    }
}

impl std::error::Error for RawPrepareCodecError {}

fn write_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u16_array(bytes: &mut [u8], offset: usize, values: [u16; 4]) {
    for (index, value) in values.into_iter().enumerate() {
        write_u16(bytes, offset + index * 2, value);
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("fixed v2 field range"),
    )
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(
        bytes[offset..offset + 2]
            .try_into()
            .expect("fixed field range"),
    )
}

fn read_u16_array(bytes: &[u8], offset: usize) -> [u16; 4] {
    std::array::from_fn(|index| read_u16(bytes, offset + index * 2))
}
