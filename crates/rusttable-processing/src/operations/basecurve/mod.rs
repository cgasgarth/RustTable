//! Bounded Basecurve CPU leaf ported from `src/iop/basecurve.c`.
//!
//! This module deliberately is not exposed through the processing registry yet. Its
//! public surface is the source-faithful history/default/curve/CPU contract that an
//! integration owner can route later. Exposure fusion, GPU, GTK, masks, blending,
//! and production routing remain explicitly unavailable.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::struct_field_names,
    clippy::large_types_passed_by_value,
    clippy::match_same_arms,
    clippy::unreadable_literal,
    clippy::unused_self,
    dead_code,
    unused_imports,
    reason = "this leaf preserves the native fixed-layout and f32 arithmetic boundaries"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Basecurve equations preserve source evaluation order and IEEE-754 parity."
)]

use std::fmt;
use std::mem::size_of;
use std::sync::Arc;

use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, sample_curve_v1,
};

mod presets;
pub mod source_map;

pub use presets::{
    BasecurveBlendColorspace, BasecurveCameraMetadata, BasecurvePreset,
    BasecurvePresetRegistration, basecurve_camera_presets, basecurve_presets, check_camera,
    init_presets, match_pattern, reload_defaults,
};

pub const BASECURVE_COMPATIBILITY_ID: &str = "basecurve";
pub const BASECURVE_RUST_ID: &str = "rusttable.basecurve";
pub const BASECURVE_SCHEMA_VERSION: u16 = 6;

pub const MAX_CURVES: usize = 3;
pub const MAX_NODES: usize = 20;
const MAX_NODES_I32: i32 = 20;
pub const LUT_RESOLUTION: usize = 0x1_0000;
pub const LUT_RESOLUTION_U32: u32 = LUT_RESOLUTION as u32;

pub const BASECURVE_V1_PARAMETER_BYTES: usize = 52;
pub const BASECURVE_V2_PARAMETER_BYTES: usize = 504;
pub const BASECURVE_V3_PARAMETER_BYTES: usize = 512;
pub const BASECURVE_V4_PARAMETER_BYTES: usize = 512;
pub const BASECURVE_V5_PARAMETER_BYTES: usize = 516;
pub const BASECURVE_V6_PARAMETER_BYTES: usize = 520;

pub const CUBIC_SPLINE: i32 = 0;
pub const CATMULL_ROM: i32 = 1;
pub const MONOTONE_HERMITE: i32 = 2;

pub const DT_RGB_NORM_NONE: i32 = 0;
pub const DT_RGB_NORM_LUMINANCE: i32 = 1;
pub const DT_RGB_NORM_MAX: i32 = 2;
pub const DT_RGB_NORM_AVERAGE: i32 = 3;
pub const DT_RGB_NORM_SUM: i32 = 4;
pub const DT_RGB_NORM_NORM: i32 = 5;
pub const DT_RGB_NORM_POWER: i32 = 6;

/// Native v6 curve node (`float x, float y`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct BasecurveNode {
    pub x: f32,
    pub y: f32,
}

impl BasecurveNode {
    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Native v6 `dt_iop_basecurve_params_t`, in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct BasecurveParameters {
    pub basecurve: [[BasecurveNode; MAX_NODES]; MAX_CURVES],
    pub basecurve_nodes: [i32; MAX_CURVES],
    pub basecurve_type: [i32; MAX_CURVES],
    pub exposure_fusion: i32,
    pub exposure_stops: f32,
    pub exposure_bias: f32,
    pub preserve_colors: i32,
}

const _: () = assert!(size_of::<BasecurveNode>() == 8);
const _: () = assert!(size_of::<BasecurveParameters>() == BASECURVE_V6_PARAMETER_BYTES);

impl BasecurveParameters {
    /// Native `init()` plus `dt_iop_default_init()` state.
    #[must_use]
    pub const fn defaults() -> Self {
        let mut basecurve = [[BasecurveNode::new(0.0, 0.0); MAX_NODES]; MAX_CURVES];
        basecurve[0][1] = BasecurveNode::new(1.0, 1.0);
        Self {
            basecurve,
            basecurve_nodes: [2, 0, 0],
            basecurve_type: [MONOTONE_HERMITE; MAX_CURVES],
            exposure_fusion: 0,
            exposure_stops: 1.0,
            exposure_bias: 1.0,
            preserve_colors: DT_RGB_NORM_LUMINANCE,
        }
    }

    /// Serializes the complete v6 payload in the native field order.
    #[must_use]
    pub fn to_bytes(self) -> [u8; BASECURVE_V6_PARAMETER_BYTES] {
        let mut bytes = [0_u8; BASECURVE_V6_PARAMETER_BYTES];
        let mut offset = 0;
        for curve in self.basecurve {
            for node in curve {
                put_f32(&mut bytes, &mut offset, node.x);
                put_f32(&mut bytes, &mut offset, node.y);
            }
        }
        for value in self.basecurve_nodes {
            put_i32(&mut bytes, &mut offset, value);
        }
        for value in self.basecurve_type {
            put_i32(&mut bytes, &mut offset, value);
        }
        put_i32(&mut bytes, &mut offset, self.exposure_fusion);
        put_f32(&mut bytes, &mut offset, self.exposure_stops);
        put_f32(&mut bytes, &mut offset, self.exposure_bias);
        put_i32(&mut bytes, &mut offset, self.preserve_colors);
        debug_assert_eq!(offset, bytes.len());
        bytes
    }

    /// Decodes exactly one native v6 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BasecurveCodecError> {
        if bytes.len() != BASECURVE_V6_PARAMETER_BYTES {
            return Err(BasecurveCodecError::InvalidLength {
                version: BASECURVE_SCHEMA_VERSION,
                expected: BASECURVE_V6_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut reader = Reader::new(bytes);
        let mut basecurve = [[BasecurveNode::default(); MAX_NODES]; MAX_CURVES];
        for curve in &mut basecurve {
            for node in curve {
                node.x = reader.f32();
                node.y = reader.f32();
            }
        }
        let mut basecurve_nodes = [0_i32; MAX_CURVES];
        for value in &mut basecurve_nodes {
            *value = reader.i32();
        }
        let mut basecurve_type = [0_i32; MAX_CURVES];
        for value in &mut basecurve_type {
            *value = reader.i32();
        }
        Ok(Self {
            basecurve,
            basecurve_nodes,
            basecurve_type,
            exposure_fusion: reader.i32(),
            exposure_stops: reader.f32(),
            exposure_bias: reader.f32(),
            preserve_colors: reader.i32(),
        })
    }
}

impl Default for BasecurveParameters {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Native module initialization state, including the initially disabled flag.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveDefaultState {
    pub parameters: BasecurveParameters,
    pub enabled: bool,
}

#[must_use]
pub const fn default_state() -> BasecurveDefaultState {
    BasecurveDefaultState {
        parameters: BasecurveParameters::defaults(),
        enabled: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveParametersV1 {
    pub tonecurve_x: [f32; 6],
    pub tonecurve_y: [f32; 6],
    pub tonecurve_preset: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveCurveState {
    pub basecurve: [[BasecurveNode; MAX_NODES]; MAX_CURVES],
    pub basecurve_nodes: [i32; MAX_CURVES],
    pub basecurve_type: [i32; MAX_CURVES],
}

impl BasecurveCurveState {
    fn from_reader(reader: &mut Reader<'_>) -> Self {
        let mut basecurve = [[BasecurveNode::default(); MAX_NODES]; MAX_CURVES];
        for curve in &mut basecurve {
            for node in curve {
                node.x = reader.f32();
                node.y = reader.f32();
            }
        }
        let mut basecurve_nodes = [0_i32; MAX_CURVES];
        for value in &mut basecurve_nodes {
            *value = reader.i32();
        }
        let mut basecurve_type = [0_i32; MAX_CURVES];
        for value in &mut basecurve_type {
            *value = reader.i32();
        }
        Self {
            basecurve,
            basecurve_nodes,
            basecurve_type,
        }
    }

    fn write_bytes(self, bytes: &mut [u8], offset: &mut usize) {
        for curve in self.basecurve {
            for node in curve {
                put_f32(bytes, offset, node.x);
                put_f32(bytes, offset, node.y);
            }
        }
        for value in self.basecurve_nodes {
            put_i32(bytes, offset, value);
        }
        for value in self.basecurve_type {
            put_i32(bytes, offset, value);
        }
    }

    const fn into_current(
        self,
        exposure_fusion: i32,
        exposure_stops: f32,
        exposure_bias: f32,
        preserve_colors: i32,
    ) -> BasecurveParameters {
        BasecurveParameters {
            basecurve: self.basecurve,
            basecurve_nodes: self.basecurve_nodes,
            basecurve_type: self.basecurve_type,
            exposure_fusion,
            exposure_stops,
            exposure_bias,
            preserve_colors,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveParametersV2 {
    pub state: BasecurveCurveState,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveParametersV3 {
    pub state: BasecurveCurveState,
    pub exposure_fusion: i32,
    pub exposure_stops: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveParametersV4 {
    pub state: BasecurveCurveState,
    pub exposure_fusion: i32,
    pub exposure_stops: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurveParametersV5 {
    pub state: BasecurveCurveState,
    pub exposure_fusion: i32,
    pub exposure_stops: f32,
    pub exposure_bias: f32,
}

/// Decoded native history. Unknown versions fail closed like `legacy_params`.
#[derive(Debug, Clone, PartialEq)]
pub enum BasecurveHistory {
    V1(BasecurveParametersV1),
    V2(BasecurveParametersV2),
    V3(BasecurveParametersV3),
    V4(BasecurveParametersV4),
    V5(BasecurveParametersV5),
    V6(BasecurveParameters),
}

impl BasecurveHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, BasecurveCodecError> {
        match version {
            1 => Ok(Self::V1(decode_v1(bytes)?)),
            2 => Ok(Self::V2(decode_v2(bytes)?)),
            3 => Ok(Self::V3(decode_v3(bytes)?)),
            4 => Ok(Self::V4(decode_v4(bytes)?)),
            5 => Ok(Self::V5(decode_v5(bytes)?)),
            BASECURVE_SCHEMA_VERSION => Ok(Self::V6(BasecurveParameters::from_bytes(bytes)?)),
            _ => Err(BasecurveCodecError::UnsupportedVersion(version)),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => 3,
            Self::V4(_) => 4,
            Self::V5(_) => 5,
            Self::V6(_) => BASECURVE_SCHEMA_VERSION,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => {
                let mut bytes = [0_u8; BASECURVE_V1_PARAMETER_BYTES];
                let mut offset = 0;
                for point in value.tonecurve_x {
                    put_f32(&mut bytes, &mut offset, point);
                }
                for point in value.tonecurve_y {
                    put_f32(&mut bytes, &mut offset, point);
                }
                put_i32(&mut bytes, &mut offset, value.tonecurve_preset);
                bytes.to_vec()
            }
            Self::V2(value) => {
                let mut bytes = [0_u8; BASECURVE_V2_PARAMETER_BYTES];
                let mut offset = 0;
                value.state.write_bytes(&mut bytes, &mut offset);
                bytes.to_vec()
            }
            Self::V3(value) => {
                let mut bytes = [0_u8; BASECURVE_V3_PARAMETER_BYTES];
                let mut offset = 0;
                value.state.write_bytes(&mut bytes, &mut offset);
                put_i32(&mut bytes, &mut offset, value.exposure_fusion);
                put_f32(&mut bytes, &mut offset, value.exposure_stops);
                bytes.to_vec()
            }
            Self::V4(value) => {
                let mut bytes = [0_u8; BASECURVE_V4_PARAMETER_BYTES];
                let mut offset = 0;
                value.state.write_bytes(&mut bytes, &mut offset);
                put_i32(&mut bytes, &mut offset, value.exposure_fusion);
                put_f32(&mut bytes, &mut offset, value.exposure_stops);
                bytes.to_vec()
            }
            Self::V5(value) => {
                let mut bytes = [0_u8; BASECURVE_V5_PARAMETER_BYTES];
                let mut offset = 0;
                value.state.write_bytes(&mut bytes, &mut offset);
                put_i32(&mut bytes, &mut offset, value.exposure_fusion);
                put_f32(&mut bytes, &mut offset, value.exposure_stops);
                put_f32(&mut bytes, &mut offset, value.exposure_bias);
                bytes.to_vec()
            }
            Self::V6(value) => value.to_bytes().to_vec(),
        }
    }

    pub fn current(&self) -> BasecurveParameters {
        match self {
            Self::V1(value) => migrate_v1_to_v6(*value),
            Self::V2(value) => migrate_v2_to_v6(*value),
            Self::V3(value) => migrate_v3_to_v6(*value),
            Self::V4(value) => migrate_v4_to_v6(*value),
            Self::V5(value) => migrate_v5_to_v6(*value),
            Self::V6(value) => *value,
        }
    }
}

pub fn decode_history(version: u16, bytes: &[u8]) -> Result<BasecurveHistory, BasecurveCodecError> {
    BasecurveHistory::decode(version, bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasecurveCodecError {
    InvalidLength {
        version: u16,
        expected: usize,
        actual: usize,
    },
    UnsupportedVersion(u16),
}

impl fmt::Display for BasecurveCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength {
                version,
                expected,
                actual,
            } => write!(
                formatter,
                "Basecurve version {version} payload has {actual} bytes; expected {expected}"
            ),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "Basecurve version {version} cannot be migrated")
            }
        }
    }
}

impl std::error::Error for BasecurveCodecError {}

pub fn migrate_v1_to_v6(value: BasecurveParametersV1) -> BasecurveParameters {
    let mut current = BasecurveParameters::defaults();
    for index in 0..6 {
        current.basecurve[0][index] =
            BasecurveNode::new(value.tonecurve_x[index], value.tonecurve_y[index]);
    }
    current.basecurve_nodes = [6, 3, 3];
    current.basecurve_type = [CUBIC_SPLINE, MONOTONE_HERMITE, MONOTONE_HERMITE];
    current.exposure_fusion = 0;
    current.exposure_stops = 1.0;
    current.exposure_bias = 1.0;
    current.preserve_colors = DT_RGB_NORM_NONE;
    current
}

pub const fn migrate_v2_to_v6(value: BasecurveParametersV2) -> BasecurveParameters {
    value.state.into_current(0, 1.0, 1.0, DT_RGB_NORM_NONE)
}

pub fn migrate_v3_to_v6(value: BasecurveParametersV3) -> BasecurveParameters {
    let stops = if value.exposure_fusion == 0 && value.exposure_stops == 0.0 {
        1.0
    } else {
        value.exposure_stops
    };
    value
        .state
        .into_current(value.exposure_fusion, stops, 1.0, DT_RGB_NORM_NONE)
}

pub const fn migrate_v4_to_v6(value: BasecurveParametersV4) -> BasecurveParameters {
    value.state.into_current(
        value.exposure_fusion,
        value.exposure_stops,
        1.0,
        DT_RGB_NORM_NONE,
    )
}

pub const fn migrate_v5_to_v6(value: BasecurveParametersV5) -> BasecurveParameters {
    value.state.into_current(
        value.exposure_fusion,
        value.exposure_stops,
        value.exposure_bias,
        DT_RGB_NORM_NONE,
    )
}

fn decode_v1(bytes: &[u8]) -> Result<BasecurveParametersV1, BasecurveCodecError> {
    check_length(1, bytes, BASECURVE_V1_PARAMETER_BYTES)?;
    let mut reader = Reader::new(bytes);
    let mut tonecurve_x = [0.0; 6];
    let mut tonecurve_y = [0.0; 6];
    for value in &mut tonecurve_x {
        *value = reader.f32();
    }
    for value in &mut tonecurve_y {
        *value = reader.f32();
    }
    Ok(BasecurveParametersV1 {
        tonecurve_x,
        tonecurve_y,
        tonecurve_preset: reader.i32(),
    })
}

fn decode_v2(bytes: &[u8]) -> Result<BasecurveParametersV2, BasecurveCodecError> {
    check_length(2, bytes, BASECURVE_V2_PARAMETER_BYTES)?;
    let mut reader = Reader::new(bytes);
    Ok(BasecurveParametersV2 {
        state: BasecurveCurveState::from_reader(&mut reader),
    })
}

fn decode_v3(bytes: &[u8]) -> Result<BasecurveParametersV3, BasecurveCodecError> {
    check_length(3, bytes, BASECURVE_V3_PARAMETER_BYTES)?;
    let mut reader = Reader::new(bytes);
    Ok(BasecurveParametersV3 {
        state: BasecurveCurveState::from_reader(&mut reader),
        exposure_fusion: reader.i32(),
        exposure_stops: reader.f32(),
    })
}

fn decode_v4(bytes: &[u8]) -> Result<BasecurveParametersV4, BasecurveCodecError> {
    check_length(4, bytes, BASECURVE_V4_PARAMETER_BYTES)?;
    let mut reader = Reader::new(bytes);
    Ok(BasecurveParametersV4 {
        state: BasecurveCurveState::from_reader(&mut reader),
        exposure_fusion: reader.i32(),
        exposure_stops: reader.f32(),
    })
}

fn decode_v5(bytes: &[u8]) -> Result<BasecurveParametersV5, BasecurveCodecError> {
    check_length(5, bytes, BASECURVE_V5_PARAMETER_BYTES)?;
    let mut reader = Reader::new(bytes);
    Ok(BasecurveParametersV5 {
        state: BasecurveCurveState::from_reader(&mut reader),
        exposure_fusion: reader.i32(),
        exposure_stops: reader.f32(),
        exposure_bias: reader.f32(),
    })
}

const fn check_length(
    version: u16,
    bytes: &[u8],
    expected: usize,
) -> Result<(), BasecurveCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(BasecurveCodecError::InvalidLength {
            version,
            expected,
            actual: bytes.len(),
        })
    }
}

fn put_f32(bytes: &mut [u8], offset: &mut usize, value: f32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

fn put_i32(bytes: &mut [u8], offset: &mut usize, value: i32) {
    bytes[*offset..*offset + 4].copy_from_slice(&value.to_le_bytes());
    *offset += 4;
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }
    fn f32(&mut self) -> f32 {
        let value = f32::from_le_bytes(
            self.bytes[self.offset..self.offset + 4]
                .try_into()
                .expect("length checked"),
        );
        self.offset += 4;
        value
    }
    fn i32(&mut self) -> i32 {
        let value = i32::from_le_bytes(
            self.bytes[self.offset..self.offset + 4]
                .try_into()
                .expect("length checked"),
        );
        self.offset += 4;
        value
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasecurveCompileError {
    UnsupportedExposureFusion { steps: i32 },
    InvalidNodeCount { count: i32 },
    InvalidCurveType { curve_type: i32 },
    Curve(CurveError),
    UnexpectedSampleCount { expected: usize, actual: usize },
    AllocationFailed { required_bytes: usize },
}

impl fmt::Display for BasecurveCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedExposureFusion { steps } => write!(
                formatter,
                "Basecurve exposure fusion {steps} is unsupported by the CPU LUT leaf"
            ),
            Self::InvalidNodeCount { count } => write!(
                formatter,
                "Basecurve active node count {count} is outside 2..={MAX_NODES}"
            ),
            Self::InvalidCurveType { curve_type } => write!(
                formatter,
                "Basecurve interpolation type {curve_type} is unsupported"
            ),
            Self::Curve(source) => {
                write!(formatter, "Basecurve curve compilation failed: {source}")
            }
            Self::UnexpectedSampleCount { expected, actual } => write!(
                formatter,
                "Basecurve sampler returned {actual} samples; expected {expected}"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Basecurve LUT allocation of {required_bytes} bytes failed"
            ),
        }
    }
}

impl std::error::Error for BasecurveCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Curve(source) => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasecurveTiling {
    pub factor_milli: u32,
    pub overlap_pixels: u32,
    pub alignment_pixels: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferredCapability {
    Supported,
    Unsupported,
    Deferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BasecurveCapabilities {
    pub cpu_lut: bool,
    pub gpu: bool,
    pub gtk: bool,
    pub consumes_masks: bool,
    pub outer_blending: DeferredCapability,
    pub production_routing: DeferredCapability,
    pub tiling: BasecurveTiling,
}

impl BasecurveCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_lut: true,
            gpu: false,
            gtk: false,
            consumes_masks: false,
            outer_blending: DeferredCapability::Deferred,
            production_routing: DeferredCapability::Deferred,
            tiling: BasecurveTiling {
                factor_milli: 2_000,
                overlap_pixels: 0,
                alignment_pixels: 1,
            },
        }
    }

    pub const fn require_gpu(self) -> Result<(), BasecurveExecutionError> {
        Err(BasecurveExecutionError::UnsupportedCapability(
            "GPU Basecurve execution is not ported",
        ))
    }

    pub const fn require_gtk(self) -> Result<(), BasecurveExecutionError> {
        Err(BasecurveExecutionError::UnsupportedCapability(
            "GTK Basecurve controls are not ported",
        ))
    }

    pub const fn require_masks(self) -> Result<(), BasecurveExecutionError> {
        Err(BasecurveExecutionError::UnsupportedCapability(
            "Basecurve mask consumption is not ported",
        ))
    }

    pub const fn require_production_routing(self) -> Result<(), BasecurveExecutionError> {
        Err(BasecurveExecutionError::UnsupportedCapability(
            "Basecurve production routing is deferred",
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BasecurvePlan {
    parameters: BasecurveParameters,
    table: Arc<[f32]>,
    unbounded_coefficients: [f32; 3],
}

impl BasecurvePlan {
    /// Compiles the active channel into the native 65536-entry LUT.
    pub fn compile(parameters: BasecurveParameters) -> Result<Self, BasecurveCompileError> {
        if parameters.exposure_fusion != 0 {
            return Err(BasecurveCompileError::UnsupportedExposureFusion {
                steps: parameters.exposure_fusion,
            });
        }
        let count = parameters.basecurve_nodes[0];
        if !(2..=MAX_NODES_I32).contains(&count) {
            return Err(BasecurveCompileError::InvalidNodeCount { count });
        }
        let curve_type = native_curve_type(parameters.basecurve_type[0])?;
        let anchors: Vec<_> = parameters.basecurve[0][..count as usize]
            .iter()
            .copied()
            .map(|node| CurveAnchor::new(node.x, node.y))
            .collect();
        let curve = Curve::new(curve_type, CurveBounds::unit(), &anchors)
            .map_err(BasecurveCompileError::Curve)?;
        let samples = sample_curve_v1(&curve, LUT_RESOLUTION_U32, LUT_RESOLUTION_U32)
            .map_err(BasecurveCompileError::Curve)?;
        if samples.len() != LUT_RESOLUTION {
            return Err(BasecurveCompileError::UnexpectedSampleCount {
                expected: LUT_RESOLUTION,
                actual: samples.len(),
            });
        }
        let required_bytes = LUT_RESOLUTION * size_of::<f32>();
        let mut table = Vec::new();
        table
            .try_reserve_exact(LUT_RESOLUTION)
            .map_err(|_| BasecurveCompileError::AllocationFailed { required_bytes })?;
        table.extend(
            samples
                .into_iter()
                .map(|sample| f32::from(sample) / LUT_RESOLUTION as f32),
        );
        let table: Arc<[f32]> = Arc::from(table);
        let last_x = anchors[count as usize - 1].x();
        let x = [0.7 * last_x, 0.8 * last_x, 0.9 * last_x, last_x];
        let y = x.map(|value| table[lookup_index(value)]);
        let coefficients = estimate_exp(x, y);
        Ok(Self {
            parameters,
            table,
            unbounded_coefficients: coefficients,
        })
    }

    #[must_use]
    pub const fn parameters(&self) -> BasecurveParameters {
        self.parameters
    }
    #[must_use]
    pub fn table(&self) -> &[f32] {
        &self.table
    }
    #[must_use]
    pub const fn unbounded_coefficients(&self) -> [f32; 3] {
        self.unbounded_coefficients
    }
    #[must_use]
    pub const fn capabilities() -> BasecurveCapabilities {
        BasecurveCapabilities::bounded_cpu_leaf()
    }

    pub fn execute_rgba(
        &self,
        input: &[BasecurvePixel],
    ) -> Result<Vec<BasecurvePixel>, BasecurveExecutionError> {
        self.execute_rgba_with_profile(input, None, || false)
    }

    pub fn execute_rgba_with_profile<F: Fn() -> bool>(
        &self,
        input: &[BasecurvePixel],
        profile: Option<&BasecurveProfileEvidence>,
        cancelled: F,
    ) -> Result<Vec<BasecurvePixel>, BasecurveExecutionError> {
        if cancelled() {
            return Err(BasecurveExecutionError::Cancelled);
        }
        let mut output = Vec::new();
        output.try_reserve_exact(input.len()).map_err(|_| {
            BasecurveExecutionError::AllocationFailed {
                required_bytes: size_of_val(input),
            }
        })?;
        for (index, pixel) in input.iter().copied().enumerate() {
            if index % 256 == 0 && cancelled() {
                return Err(BasecurveExecutionError::Cancelled);
            }
            let channels = if self.parameters.preserve_colors == DT_RGB_NORM_NONE {
                apply_legacy(
                    pixel.channels,
                    1.0,
                    &self.table,
                    self.unbounded_coefficients,
                )
            } else {
                apply_curve(
                    pixel.channels,
                    self.parameters.preserve_colors,
                    1.0,
                    &self.table,
                    self.unbounded_coefficients,
                    profile,
                )
            };
            output.push(BasecurvePixel::from_channels(channels));
        }
        Ok(output)
    }
}

const fn native_curve_type(value: i32) -> Result<CurveType, BasecurveCompileError> {
    match value {
        CUBIC_SPLINE => Ok(CurveType::CubicSpline),
        CATMULL_ROM => Ok(CurveType::CatmullRom),
        MONOTONE_HERMITE => Ok(CurveType::MonotoneHermite),
        curve_type => Err(BasecurveCompileError::InvalidCurveType { curve_type }),
    }
}

fn estimate_exp(x: [f32; 4], y: [f32; 4]) -> [f32; 3] {
    let x0 = x[3];
    let y0 = y[3];
    let mut g = 0.0;
    let mut count = 0;
    for index in 0..3 {
        let yy = y[index] / y0;
        let xx = x[index] / x0;
        if yy > 0.0 && xx > 0.0 {
            g += (y[index] / y0).ln() / (x[index] / x0).ln();
            count += 1;
        }
    }
    if count != 0 {
        g *= 1.0 / count as f32;
    } else {
        g = 1.0;
    }
    [1.0 / x0, y0, g]
}

fn eval_exp(coefficients: [f32; 3], value: f32) -> f32 {
    coefficients[1] * (value * coefficients[0]).powf(coefficients[2])
}

fn lookup_index(value: f32) -> usize {
    let index = (value * LUT_RESOLUTION as f32) as i64;
    index.clamp(0, (LUT_RESOLUTION - 1) as i64) as usize
}

fn lookup_unbounded(table: &[f32], coefficients: [f32; 3], value: f32) -> f32 {
    if value < 1.0 {
        table[lookup_index(value)]
    } else {
        eval_exp(coefficients, value)
    }
}

fn apply_legacy(
    input: [f32; 4],
    multiplier: f32,
    table: &[f32],
    coefficients: [f32; 3],
) -> [f32; 4] {
    [
        lookup_unbounded(table, coefficients, multiplier * input[0]).max(0.0),
        lookup_unbounded(table, coefficients, multiplier * input[1]).max(0.0),
        lookup_unbounded(table, coefficients, multiplier * input[2]).max(0.0),
        input[3],
    ]
}

fn apply_curve(
    input: [f32; 4],
    preserve_colors: i32,
    multiplier: f32,
    table: &[f32],
    coefficients: [f32; 3],
    profile: Option<&BasecurveProfileEvidence>,
) -> [f32; 4] {
    let luminance = multiplier * rgb_norm(input, preserve_colors, profile);
    let ratio = if luminance > 0.0 {
        multiplier * lookup_unbounded(table, coefficients, luminance) / luminance
    } else {
        1.0
    };
    [
        (ratio * input[0]).max(0.0),
        (ratio * input[1]).max(0.0),
        (ratio * input[2]).max(0.0),
        input[3],
    ]
}

fn rgb_norm(input: [f32; 4], norm: i32, profile: Option<&BasecurveProfileEvidence>) -> f32 {
    match norm {
        DT_RGB_NORM_LUMINANCE => profile.map_or_else(
            || input[0] * 0.2225045 + input[1] * 0.7168786 + input[2] * 0.0606169,
            |value| value.working_luminance(input),
        ),
        DT_RGB_NORM_MAX => input[0].max(input[1].max(input[2])),
        DT_RGB_NORM_AVERAGE => (input[0] + input[1] + input[2]) / 3.0,
        DT_RGB_NORM_SUM => input[0] + input[1] + input[2],
        DT_RGB_NORM_NORM => {
            (input[0] * input[0] + input[1] * input[1] + input[2] * input[2]).sqrt()
        }
        DT_RGB_NORM_POWER => {
            let red = input[0] * input[0];
            let green = input[1] * input[1];
            let blue = input[2] * input[2];
            (input[0] * red + input[1] * green + input[2] * blue) / (red + green + blue)
        }
        _ => (input[0] + input[1] + input[2]) / 3.0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasecurvePixel {
    channels: [f32; 4],
}

impl BasecurvePixel {
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            channels: [red, green, blue, alpha],
        }
    }
    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }
    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
        self.channels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasecurveExecutionError {
    Cancelled,
    AllocationFailed { required_bytes: usize },
    UnsupportedCapability(&'static str),
}

impl fmt::Display for BasecurveExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("Basecurve execution was cancelled"),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Basecurve output allocation of {required_bytes} bytes failed"
            ),
            Self::UnsupportedCapability(reason) => {
                write!(formatter, "unsupported Basecurve capability: {reason}")
            }
        }
    }
}

impl std::error::Error for BasecurveExecutionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasecurveProfileError {
    ZeroLutSize,
    LutSizeTooSmall {
        actual: usize,
    },
    MatrixNonFinite,
    UnsupportedCapability(&'static str),
    CoefficientNonFinite {
        channel: usize,
        coefficient: usize,
    },
    LutTooShort {
        channel: usize,
        expected: usize,
        actual: usize,
    },
    LutNonFinite {
        channel: usize,
        index: usize,
    },
}

impl fmt::Display for BasecurveProfileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLutSize => formatter.write_str("working profile LUT size must be nonzero"),
            Self::LutSizeTooSmall { actual } => {
                write!(
                    formatter,
                    "working profile LUT size {actual} is smaller than two"
                )
            }
            Self::MatrixNonFinite => {
                formatter.write_str("working profile matrix contains a non-finite value")
            }
            Self::UnsupportedCapability(reason) => {
                write!(
                    formatter,
                    "unsupported Basecurve profile capability: {reason}"
                )
            }
            Self::CoefficientNonFinite {
                channel,
                coefficient,
            } => write!(
                formatter,
                "working profile coefficient {channel}:{coefficient} is non-finite"
            ),
            Self::LutTooShort {
                channel,
                expected,
                actual,
            } => write!(
                formatter,
                "working profile LUT {channel} has {actual} values; expected {expected}"
            ),
            Self::LutNonFinite { channel, index } => write!(
                formatter,
                "working profile LUT {channel} value {index} is non-finite"
            ),
        }
    }
}

impl std::error::Error for BasecurveProfileError {}

/// Explicit ICC evidence required by the native working-profile luminance path.
///
/// `WorkingFrameDescriptor` is intentionally not accepted here: it carries no
/// ICC LUTs or unbounded coefficients and therefore cannot justify this path.
#[derive(Debug, Clone, PartialEq)]
pub struct BasecurveProfileEvidence {
    matrix_in: [[f32; 3]; 3],
    lut_in: [Vec<f32>; 3],
    unbounded_coeffs_in: [[f32; 3]; 3],
    lutsize: usize,
    nonlinearlut: bool,
}

impl BasecurveProfileEvidence {
    pub fn new(
        matrix_in: [[f32; 3]; 3],
        lut_in: [Vec<f32>; 3],
        unbounded_coeffs_in: [[f32; 3]; 3],
        lutsize: usize,
        nonlinearlut: bool,
    ) -> Result<Self, BasecurveProfileError> {
        if lutsize == 0 {
            return Err(BasecurveProfileError::ZeroLutSize);
        }
        if lutsize < 2 {
            return Err(BasecurveProfileError::LutSizeTooSmall { actual: lutsize });
        }
        if matrix_in.iter().flatten().any(|value| !value.is_finite()) {
            return Err(BasecurveProfileError::MatrixNonFinite);
        }
        for (channel, coefficients) in unbounded_coeffs_in.into_iter().enumerate() {
            for (coefficient, value) in coefficients.into_iter().enumerate() {
                if !value.is_finite() {
                    return Err(BasecurveProfileError::CoefficientNonFinite {
                        channel,
                        coefficient,
                    });
                }
            }
        }
        for (channel, values) in lut_in.iter().enumerate() {
            if values.first().is_some_and(|value| *value >= 0.0) {
                if values.len() < lutsize {
                    return Err(BasecurveProfileError::LutTooShort {
                        channel,
                        expected: lutsize,
                        actual: values.len(),
                    });
                }
                if let Some(index) = values[..lutsize]
                    .iter()
                    .position(|value| !value.is_finite())
                {
                    return Err(BasecurveProfileError::LutNonFinite { channel, index });
                }
            }
        }
        Ok(Self {
            matrix_in,
            lut_in,
            unbounded_coeffs_in,
            lutsize,
            nonlinearlut,
        })
    }

    /// Evaluates the native working-profile luminance equation.
    #[must_use]
    pub fn working_luminance(&self, input: [f32; 4]) -> f32 {
        let linear = if self.nonlinearlut {
            [
                self.apply_trc(0, input[0]),
                self.apply_trc(1, input[1]),
                self.apply_trc(2, input[2]),
            ]
        } else {
            [input[0], input[1], input[2]]
        };
        self.matrix_in[1][0] * linear[0]
            + self.matrix_in[1][1] * linear[1]
            + self.matrix_in[1][2] * linear[2]
    }

    fn apply_trc(&self, channel: usize, value: f32) -> f32 {
        let lut = &self.lut_in[channel];
        if !lut.first().is_some_and(|marker| *marker >= 0.0) {
            return value;
        }
        if value < 1.0 {
            let scaled = (value * (self.lutsize - 1) as f32).clamp(0.0, (self.lutsize - 1) as f32);
            let t = if scaled < (self.lutsize - 2) as f32 {
                scaled as usize
            } else {
                self.lutsize - 2
            };
            let fraction = scaled - t as f32;
            lut[t] * (1.0 - fraction) + lut[t + 1] * fraction
        } else {
            eval_exp(self.unbounded_coeffs_in[channel], value)
        }
    }
}

/// Explicitly rejects the tempting but incorrect profile inference boundary.
pub const fn unsupported_working_frame_profile()
-> Result<BasecurveProfileEvidence, BasecurveProfileError> {
    Err(BasecurveProfileError::UnsupportedCapability(
        "WorkingFrameDescriptor does not carry native ICC LUT evidence",
    ))
}
