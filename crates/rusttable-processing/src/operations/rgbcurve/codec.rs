//! History codec ported from the native `dt_iop_rgbcurve_params_t` ABI in
//! `src/iop/rgbcurve.c`.
//!
//! Darktable's supported ABI is encoded explicitly as little-endian bytes;
//! Rust layout, alignment, and enum representation are never serialized.

#![forbid(unsafe_code)]

use std::fmt;

use super::parameters::{
    CHANNELS, MAX_NODES, PARAMETER_BYTES, PARAMETER_VERSION, ParameterError, RgbCurveNode,
    RgbCurveParametersV1,
};

const NODE_BYTES: usize = 8;
const NODES_OFFSET: usize = 0;
const NODE_COUNTS_OFFSET: usize = NODES_OFFSET + CHANNELS * MAX_NODES * NODE_BYTES;
const CURVE_TYPES_OFFSET: usize = NODE_COUNTS_OFFSET + CHANNELS * 4;
const AUTOSCALE_OFFSET: usize = CURVE_TYPES_OFFSET + CHANNELS * 4;
const COMPENSATE_OFFSET: usize = AUTOSCALE_OFFSET + 4;
const PRESERVE_OFFSET: usize = COMPENSATE_OFFSET + 4;

const _: () = assert!(PRESERVE_OFFSET + 4 == PARAMETER_BYTES);

impl RgbCurveParametersV1 {
    /// Serializes all 516 bytes, including inactive node tails.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PARAMETER_BYTES] {
        let mut bytes = [0_u8; PARAMETER_BYTES];
        for channel in 0..CHANNELS {
            for node in 0..MAX_NODES {
                let offset = NODES_OFFSET + (channel * MAX_NODES + node) * NODE_BYTES;
                bytes[offset..offset + 4]
                    .copy_from_slice(&self.curve_nodes[channel][node].x.to_le_bytes());
                bytes[offset + 4..offset + 8]
                    .copy_from_slice(&self.curve_nodes[channel][node].y.to_le_bytes());
            }
            let count_offset = NODE_COUNTS_OFFSET + channel * 4;
            bytes[count_offset..count_offset + 4]
                .copy_from_slice(&self.curve_num_nodes[channel].to_le_bytes());
            let type_offset = CURVE_TYPES_OFFSET + channel * 4;
            bytes[type_offset..type_offset + 4]
                .copy_from_slice(&(self.curve_type[channel] as i32).to_le_bytes());
        }
        bytes[AUTOSCALE_OFFSET..AUTOSCALE_OFFSET + 4]
            .copy_from_slice(&(self.curve_autoscale as i32).to_le_bytes());
        bytes[COMPENSATE_OFFSET..COMPENSATE_OFFSET + 4]
            .copy_from_slice(&(i32::from(self.compensate_middle_grey)).to_le_bytes());
        bytes[PRESERVE_OFFSET..PRESERVE_OFFSET + 4]
            .copy_from_slice(&(self.preserve_colors as i32).to_le_bytes());
        bytes
    }

    /// Decodes and validates the supported native version-1 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RgbCurveCodecError> {
        if bytes.len() != PARAMETER_BYTES {
            return Err(RgbCurveCodecError::InvalidLength {
                expected: PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut curve_nodes = [[RgbCurveNode::ZERO; MAX_NODES]; CHANNELS];
        for channel in 0..CHANNELS {
            for node in 0..MAX_NODES {
                let offset = NODES_OFFSET + (channel * MAX_NODES + node) * NODE_BYTES;
                curve_nodes[channel][node] = RgbCurveNode::new(
                    f32::from_le_bytes(
                        bytes[offset..offset + 4]
                            .try_into()
                            .expect("checked length"),
                    ),
                    f32::from_le_bytes(
                        bytes[offset + 4..offset + 8]
                            .try_into()
                            .expect("checked length"),
                    ),
                );
            }
        }
        let mut curve_num_nodes = [0_i32; CHANNELS];
        let mut curve_type = [0_i32; CHANNELS];
        for channel in 0..CHANNELS {
            let count_offset = NODE_COUNTS_OFFSET + channel * 4;
            curve_num_nodes[channel] = i32::from_le_bytes(
                bytes[count_offset..count_offset + 4]
                    .try_into()
                    .expect("checked length"),
            );
            let type_offset = CURVE_TYPES_OFFSET + channel * 4;
            curve_type[channel] = i32::from_le_bytes(
                bytes[type_offset..type_offset + 4]
                    .try_into()
                    .expect("checked length"),
            );
        }
        let parameters = RgbCurveParametersV1::from_raw(
            curve_nodes,
            curve_num_nodes,
            curve_type,
            read_i32(bytes, AUTOSCALE_OFFSET),
            read_i32(bytes, COMPENSATE_OFFSET),
            read_i32(bytes, PRESERVE_OFFSET),
        )?;
        Ok(parameters)
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("checked length"),
    )
}

/// Versioned RGB Curve history, retaining unsupported versions byte-for-byte.
#[derive(Debug, Clone, PartialEq)]
pub enum RgbCurveHistory {
    V1(RgbCurveParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl RgbCurveHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, RgbCurveCodecError> {
        if version == PARAMETER_VERSION {
            Ok(Self::V1(RgbCurveParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => PARAMETER_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<&RgbCurveParametersV1, RgbCurveCodecError> {
        match self {
            Self::V1(parameters) => Ok(parameters),
            Self::Opaque { version, .. } => Err(RgbCurveCodecError::UnsupportedVersion(*version)),
        }
    }

    /// The native module has no `legacy_params()` and therefore no migrations.
    #[must_use]
    pub const fn migration_edges() -> &'static [(u16, u16)] {
        &[]
    }
}

/// Errors at the explicit version-1 history boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RgbCurveCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
    InvalidParameters(ParameterError),
}

impl From<ParameterError> for RgbCurveCodecError {
    fn from(error: ParameterError) -> Self {
        Self::InvalidParameters(error)
    }
}

impl fmt::Display for RgbCurveCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "RGB Curve payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "RGB Curve version {version} is opaque and unsupported"
                )
            }
            Self::InvalidParameters(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for RgbCurveCodecError {}

/// Native ABI offsets retained as a testable source map.
#[must_use]
pub const fn abi_offsets() -> [usize; 6] {
    [
        NODES_OFFSET,
        NODE_COUNTS_OFFSET,
        CURVE_TYPES_OFFSET,
        AUTOSCALE_OFFSET,
        COMPENSATE_OFFSET,
        PRESERVE_OFFSET,
    ]
}
