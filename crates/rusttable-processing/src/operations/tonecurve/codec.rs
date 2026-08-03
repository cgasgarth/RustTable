//! Tone Curve history codec ported from `legacy_params()` and the v5
//! `dt_iop_tonecurve_params_t` ABI in `src/iop/tonecurve.c`.
//!
//! Native structure layout is represented explicitly as little-endian bytes;
//! Rust enum layout and alignment never participate in history serialization.

#![forbid(unsafe_code)]

use std::fmt;

use super::parameters::{
    CHANNELS, LEGACY_V1_BYTES, LEGACY_V3_BYTES, LEGACY_V4_BYTES, MAX_NODES, PARAMETER_BYTES,
    PARAMETER_VERSION, ParameterError, ToneCurveNode, ToneCurveParametersV5,
};

const NODE_BYTES: usize = 8;
const NODES_OFFSET: usize = 0;
const NODE_COUNTS_OFFSET: usize = NODES_OFFSET + CHANNELS * MAX_NODES * NODE_BYTES;
const CURVE_TYPES_OFFSET: usize = NODE_COUNTS_OFFSET + CHANNELS * 4;
const AUTOSCALE_OFFSET: usize = CURVE_TYPES_OFFSET + CHANNELS * 4;
const PRESET_OFFSET: usize = AUTOSCALE_OFFSET + 4;
const UNBOUND_OFFSET: usize = PRESET_OFFSET + 4;
const PRESERVE_OFFSET: usize = UNBOUND_OFFSET + 4;

const _: () = assert!(PRESERVE_OFFSET + 4 == PARAMETER_BYTES);

impl ToneCurveParametersV5 {
    /// Serializes all 520 bytes, including inactive node tails.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; PARAMETER_BYTES] {
        let mut bytes = [0_u8; PARAMETER_BYTES];
        for channel in 0..CHANNELS {
            for node in 0..MAX_NODES {
                let offset = NODES_OFFSET + (channel * MAX_NODES + node) * NODE_BYTES;
                bytes[offset..offset + 4]
                    .copy_from_slice(&self.tonecurve[channel][node].x.to_le_bytes());
                bytes[offset + 4..offset + 8]
                    .copy_from_slice(&self.tonecurve[channel][node].y.to_le_bytes());
            }
            let count_offset = NODE_COUNTS_OFFSET + channel * 4;
            bytes[count_offset..count_offset + 4]
                .copy_from_slice(&self.tonecurve_nodes[channel].to_le_bytes());
            let type_offset = CURVE_TYPES_OFFSET + channel * 4;
            bytes[type_offset..type_offset + 4]
                .copy_from_slice(&(self.tonecurve_type[channel] as i32).to_le_bytes());
        }
        bytes[AUTOSCALE_OFFSET..AUTOSCALE_OFFSET + 4]
            .copy_from_slice(&(self.tonecurve_autoscale_ab as i32).to_le_bytes());
        bytes[PRESET_OFFSET..PRESET_OFFSET + 4]
            .copy_from_slice(&self.tonecurve_preset.to_le_bytes());
        bytes[UNBOUND_OFFSET..UNBOUND_OFFSET + 4]
            .copy_from_slice(&(i32::from(self.tonecurve_unbound_ab)).to_le_bytes());
        bytes[PRESERVE_OFFSET..PRESERVE_OFFSET + 4]
            .copy_from_slice(&(self.preserve_colors as i32).to_le_bytes());
        bytes
    }

    /// Decodes and validates a native v5 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ToneCurveCodecError> {
        if bytes.len() != PARAMETER_BYTES {
            return Err(ToneCurveCodecError::InvalidLength {
                expected: PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        decode_v5(bytes)
    }
}

/// A decoded history item. Every supported legacy path is materialized as v5,
/// matching native `legacy_params()`; unsupported versions fail closed.
#[derive(Debug, Clone, PartialEq)]
pub enum ToneCurveHistory {
    V5(ToneCurveParametersV5),
}

impl ToneCurveHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ToneCurveCodecError> {
        let parameters = match version {
            1 => migrate_v1(bytes)?,
            2 => return Err(ToneCurveCodecError::UnsupportedVersion(2)),
            3 => migrate_v3(bytes)?,
            4 => migrate_v4(bytes)?,
            PARAMETER_VERSION => ToneCurveParametersV5::from_bytes(bytes)?,
            other => return Err(ToneCurveCodecError::UnsupportedVersion(other)),
        };
        Ok(Self::V5(parameters))
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        PARAMETER_VERSION
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V5(parameters) => parameters.to_bytes().to_vec(),
        }
    }

    pub const fn current(&self) -> &ToneCurveParametersV5 {
        match self {
            Self::V5(parameters) => parameters,
        }
    }

    /// Exact native migration graph. Version 2 intentionally has no edge.
    #[must_use]
    pub const fn migration_edges() -> &'static [(u16, u16)] {
        &[(1, 5), (3, 5), (4, 5)]
    }
}

fn decode_v5(bytes: &[u8]) -> Result<ToneCurveParametersV5, ToneCurveCodecError> {
    let (tonecurve, tonecurve_nodes, tonecurve_type) = decode_curve_prefix(bytes);
    ToneCurveParametersV5::from_raw(
        tonecurve,
        tonecurve_nodes,
        tonecurve_type,
        read_i32(bytes, AUTOSCALE_OFFSET),
        read_i32(bytes, PRESET_OFFSET),
        read_i32(bytes, UNBOUND_OFFSET),
        read_i32(bytes, PRESERVE_OFFSET),
    )
    .map_err(ToneCurveCodecError::InvalidParameters)
}

fn migrate_v1(bytes: &[u8]) -> Result<ToneCurveParametersV5, ToneCurveCodecError> {
    ensure_length(bytes, LEGACY_V1_BYTES)?;
    let mut parameters = ToneCurveParametersV5::default();
    for index in 0..6 {
        parameters.tonecurve[0][index].x = read_f32(bytes, index * 4);
        parameters.tonecurve[0][index].y = read_f32(bytes, 24 + index * 4);
    }
    parameters.tonecurve_nodes[0] = 6;
    parameters.tonecurve_type[0] = super::parameters::ToneCurveType::CubicSpline;
    parameters.tonecurve_autoscale_ab = super::parameters::ToneCurveAutoscale::AutomaticLab;
    parameters.tonecurve_preset = read_i32(bytes, 48);
    parameters.tonecurve_unbound_ab = false;
    parameters.preserve_colors = super::parameters::PreserveColors::None;
    parameters
        .validate()
        .map_err(ToneCurveCodecError::InvalidParameters)?;
    Ok(parameters)
}

fn migrate_v3(bytes: &[u8]) -> Result<ToneCurveParametersV5, ToneCurveCodecError> {
    ensure_length(bytes, LEGACY_V3_BYTES)?;
    let (tonecurve, tonecurve_nodes, tonecurve_type) = decode_curve_prefix(bytes);
    ToneCurveParametersV5::from_raw(
        tonecurve,
        tonecurve_nodes,
        tonecurve_type,
        read_i32(bytes, AUTOSCALE_OFFSET),
        read_i32(bytes, PRESET_OFFSET),
        0,
        0,
    )
    .map_err(ToneCurveCodecError::InvalidParameters)
}

fn migrate_v4(bytes: &[u8]) -> Result<ToneCurveParametersV5, ToneCurveCodecError> {
    ensure_length(bytes, LEGACY_V4_BYTES)?;
    let (tonecurve, tonecurve_nodes, tonecurve_type) = decode_curve_prefix(bytes);
    ToneCurveParametersV5::from_raw(
        tonecurve,
        tonecurve_nodes,
        tonecurve_type,
        read_i32(bytes, AUTOSCALE_OFFSET),
        read_i32(bytes, PRESET_OFFSET),
        read_i32(bytes, UNBOUND_OFFSET),
        0,
    )
    .map_err(ToneCurveCodecError::InvalidParameters)
}

fn decode_curve_prefix(
    bytes: &[u8],
) -> (
    [[ToneCurveNode; MAX_NODES]; CHANNELS],
    [i32; CHANNELS],
    [i32; CHANNELS],
) {
    let mut tonecurve = [[ToneCurveNode::ZERO; MAX_NODES]; CHANNELS];
    let mut tonecurve_nodes = [0_i32; CHANNELS];
    let mut tonecurve_type = [0_i32; CHANNELS];
    for channel in 0..CHANNELS {
        for node in 0..MAX_NODES {
            let offset = NODES_OFFSET + (channel * MAX_NODES + node) * NODE_BYTES;
            tonecurve[channel][node] =
                ToneCurveNode::new(read_f32(bytes, offset), read_f32(bytes, offset + 4));
        }
        tonecurve_nodes[channel] = read_i32(bytes, NODE_COUNTS_OFFSET + channel * 4);
        tonecurve_type[channel] = read_i32(bytes, CURVE_TYPES_OFFSET + channel * 4);
    }
    (tonecurve, tonecurve_nodes, tonecurve_type)
}

const fn ensure_length(bytes: &[u8], expected: usize) -> Result<(), ToneCurveCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ToneCurveCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("codec length was checked"),
    )
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("codec length was checked"),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToneCurveCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
    InvalidParameters(ParameterError),
}

impl fmt::Display for ToneCurveCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Tone Curve payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "Tone Curve version {version} is unsupported")
            }
            Self::InvalidParameters(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ToneCurveCodecError {}

/// Native ABI offsets, retained as a source map for contract tests.
#[must_use]
pub const fn abi_offsets() -> [usize; 7] {
    [
        NODES_OFFSET,
        NODE_COUNTS_OFFSET,
        CURVE_TYPES_OFFSET,
        AUTOSCALE_OFFSET,
        PRESET_OFFSET,
        UNBOUND_OFFSET,
        PRESERVE_OFFSET,
    ]
}
