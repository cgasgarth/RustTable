//! Darktable-compatible Color Zones history, curve compilation, and scalar execution.
//!
//! The byte layouts, direct legacy migrations, enum tags, defaults, and active
//! curve-node contract are derived from `src/iop/colorzones.c`; curve sampling
//! maps the retained `src/common/curve_tools.c` and `src/common/splines.cpp`
//! implementations.

#![forbid(unsafe_code)]
#![allow(
    clippy::large_types_passed_by_value,
    clippy::missing_errors_doc,
    clippy::trivially_copy_pass_by_ref,
    reason = "the source-shaped raw codec keeps explicit by-value migrations and uniform borrowed encoders"
)]

mod codec;
mod curve;
mod execution;

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::FiniteF32;
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

pub use codec::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_LEGACY_BANDS,
    COLORZONES_MAX_NODES, COLORZONES_RUST_ID, COLORZONES_SCHEMA_VERSION, COLORZONES_V1_BANDS,
    COLORZONES_V1_PARAMETER_BYTES, COLORZONES_V2_PARAMETER_BYTES, COLORZONES_V3_PARAMETER_BYTES,
    COLORZONES_V4_PARAMETER_BYTES, COLORZONES_V5_PARAMETER_BYTES, ColorZonesCodecError,
    ColorZonesHistory, ColorZonesNode, ColorZonesParametersV1, ColorZonesParametersV2,
    ColorZonesParametersV3, ColorZonesParametersV4, ColorZonesParametersV5, migrate_v1_to_v5,
    migrate_v2_to_v5, migrate_v3_to_v5, migrate_v4_to_v5,
};
pub use curve::{COLORZONES_LUT_RESOLUTION, ColorZonesCompileError};
pub use execution::{ColorZonesPixel, ColorZonesPlan};

/// `init()` leaves Color Zones disabled in a newly constructed native module.
pub const COLORZONES_DEFAULT_ENABLED: bool = false;
/// Dedicated source-compatible WGPU execution uses the core-compute tier.
pub const COLORZONES_GPU_TIER: u8 = 1;
/// Stable identity of the dedicated source-compatible Color Zones point shader.
pub const COLORZONES_WGPU_PASS_ID: &str = "darktable.colorzones.point.v1";

/// WGPU passes used by the qualified dedicated path.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    [COLORZONES_WGPU_PASS_ID]
}

/// Native selection channel stored in Color Zones history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesChannel {
    Lightness = 0,
    Chroma = 1,
    Hue = 2,
}

impl ColorZonesChannel {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Lightness),
            1 => Some(Self::Chroma),
            2 => Some(Self::Hue),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Lightness => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Lightness => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }
}

/// Native point-processing branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesMode {
    Smooth = 0,
    Strong = 1,
}

impl ColorZonesMode {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Smooth),
            1 => Some(Self::Strong),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Smooth => 0,
            Self::Strong => 1,
        }
    }
}

/// Native curve interpolation tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesCurveType {
    Cubic = 0,
    Catmull = 1,
    Monotone = 2,
}

impl ColorZonesCurveType {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Cubic),
            1 => Some(Self::Catmull),
            2 => Some(Self::Monotone),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Cubic => 0,
            Self::Catmull => 1,
            Self::Monotone => 2,
        }
    }
}

/// Native curve-boundary implementation version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesSplinesVersion {
    V1 = 0,
    V2 = 1,
}

impl ColorZonesSplinesVersion {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::V1),
            1 => Some(Self::V2),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::V1 => 0,
            Self::V2 => 1,
        }
    }
}

/// One finite active curve point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorZonesPoint {
    x: FiniteF32,
    y: FiniteF32,
}

impl ColorZonesPoint {
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x.get()
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y.get()
    }
}

/// One checked curve containing only the active native prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorZonesCurve {
    curve_type: ColorZonesCurveType,
    points: Vec<ColorZonesPoint>,
}

impl ColorZonesCurve {
    #[must_use]
    pub const fn curve_type(&self) -> ColorZonesCurveType {
        self.curve_type
    }

    #[must_use]
    pub fn points(&self) -> &[ColorZonesPoint] {
        &self.points
    }

    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.points.len()
    }
}

/// Checked Color Zones parameters ready for later curve compilation.
///
/// Native history is not clamped or sorted here. Only active nodes are
/// inspected; inactive v5 tail storage is deliberately ignored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorZonesConfig {
    channel: ColorZonesChannel,
    curves: [ColorZonesCurve; COLORZONES_CHANNELS],
    strength: FiniteF32,
    mode: ColorZonesMode,
    splines_version: ColorZonesSplinesVersion,
}

impl ColorZonesConfig {
    /// Returns Darktable's checked native v5 defaults.
    ///
    /// # Panics
    ///
    /// Panics if the source-derived raw defaults stop satisfying this module's
    /// semantic validation contract.
    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(&ColorZonesParametersV5::defaults())
            .expect("native Color Zones defaults are valid")
    }

    #[must_use]
    pub const fn channel(&self) -> ColorZonesChannel {
        self.channel
    }

    #[must_use]
    pub const fn curves(&self) -> &[ColorZonesCurve; COLORZONES_CHANNELS] {
        &self.curves
    }

    #[must_use]
    pub const fn curve(&self, channel: ColorZonesChannel) -> &ColorZonesCurve {
        &self.curves[channel.index()]
    }

    #[must_use]
    pub const fn strength(&self) -> f32 {
        self.strength.get()
    }

    #[must_use]
    pub const fn mode(&self) -> ColorZonesMode {
        self.mode
    }

    #[must_use]
    pub const fn splines_version(&self) -> ColorZonesSplinesVersion {
        self.splines_version
    }
}

impl TryFrom<&ColorZonesParametersV5> for ColorZonesConfig {
    type Error = ColorZonesParameterError;

    fn try_from(parameters: &ColorZonesParametersV5) -> Result<Self, Self::Error> {
        let channel = ColorZonesChannel::from_raw(parameters.channel).ok_or(
            ColorZonesParameterError::InvalidEnum {
                parameter: "channel",
                value: parameters.channel,
            },
        )?;
        let mode = ColorZonesMode::from_raw(parameters.mode).ok_or(
            ColorZonesParameterError::InvalidEnum {
                parameter: "mode",
                value: parameters.mode,
            },
        )?;
        let splines_version = ColorZonesSplinesVersion::from_raw(parameters.splines_version)
            .ok_or(ColorZonesParameterError::InvalidEnum {
                parameter: "splines_version",
                value: parameters.splines_version,
            })?;
        let strength = FiniteF32::new(parameters.strength)
            .map_err(|_| ColorZonesParameterError::NonFiniteStrength)?;

        let mut curves = Vec::with_capacity(COLORZONES_CHANNELS);
        for curve_channel in 0..COLORZONES_CHANNELS {
            let count = parameters.curve_num_nodes[curve_channel];
            let minimum_node_count = match splines_version {
                ColorZonesSplinesVersion::V1 => 2,
                ColorZonesSplinesVersion::V2 => 1,
            };
            if !(minimum_node_count
                ..=i32::try_from(COLORZONES_MAX_NODES).expect("node limit fits i32"))
                .contains(&count)
            {
                return Err(ColorZonesParameterError::InvalidNodeCount {
                    channel: curve_channel,
                    count,
                });
            }
            let curve_type = ColorZonesCurveType::from_raw(parameters.curve_type[curve_channel])
                .ok_or(ColorZonesParameterError::InvalidCurveType {
                    channel: curve_channel,
                    value: parameters.curve_type[curve_channel],
                })?;
            let active = usize::try_from(count).expect("validated node count is positive");
            let mut points = Vec::with_capacity(active);
            for node in 0..active {
                let raw = parameters.curves[curve_channel][node];
                let x = FiniteF32::new(raw.x).map_err(|_| {
                    ColorZonesParameterError::NonFiniteActiveNode {
                        channel: curve_channel,
                        node,
                        coordinate: "x",
                    }
                })?;
                let y = FiniteF32::new(raw.y).map_err(|_| {
                    ColorZonesParameterError::NonFiniteActiveNode {
                        channel: curve_channel,
                        node,
                        coordinate: "y",
                    }
                })?;
                points.push(ColorZonesPoint { x, y });
            }
            curves.push(ColorZonesCurve { curve_type, points });
        }
        let curves = <[ColorZonesCurve; COLORZONES_CHANNELS]>::try_from(curves)
            .expect("one checked curve is built for every native channel");
        Ok(Self {
            channel,
            curves,
            strength,
            mode,
            splines_version,
        })
    }
}

impl TryFrom<ColorZonesParametersV5> for ColorZonesConfig {
    type Error = ColorZonesParameterError;

    fn try_from(parameters: ColorZonesParametersV5) -> Result<Self, Self::Error> {
        Self::try_from(&parameters)
    }
}

/// Invalid semantic values in an otherwise losslessly decoded v5 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesParameterError {
    InvalidEnum {
        parameter: &'static str,
        value: i32,
    },
    InvalidNodeCount {
        channel: usize,
        count: i32,
    },
    InvalidCurveType {
        channel: usize,
        value: i32,
    },
    NonFiniteStrength,
    NonFiniteActiveNode {
        channel: usize,
        node: usize,
        coordinate: &'static str,
    },
}

impl fmt::Display for ColorZonesParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnum { parameter, value } => {
                write!(formatter, "Color Zones {parameter} tag {value} is invalid")
            }
            Self::InvalidNodeCount { channel, count } => write!(
                formatter,
                "Color Zones curve {channel} has {count} active nodes; expected 2..={COLORZONES_MAX_NODES} for spline v1 or 1..={COLORZONES_MAX_NODES} for spline v2"
            ),
            Self::InvalidCurveType { channel, value } => write!(
                formatter,
                "Color Zones curve {channel} interpolation tag {value} is invalid"
            ),
            Self::NonFiniteStrength => formatter.write_str("Color Zones strength is non-finite"),
            Self::NonFiniteActiveNode {
                channel,
                node,
                coordinate,
            } => write!(
                formatter,
                "Color Zones curve {channel} active node {node} {coordinate} coordinate is non-finite"
            ),
        }
    }
}

impl std::error::Error for ColorZonesParameterError {}

/// Canonical editable Color Zones v5 descriptor.
///
/// Native Color Zones is disabled by default. The registry therefore exposes
/// it as an optional, non-mandatory operation while preserving the source
/// style, blending, and tiling capabilities.
///
/// # Panics
///
/// Panics only if the checked-in Color Zones descriptor identity or native
/// fixed-size limits stop fitting their documented representations.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "Color Zones descriptor fields mirror the native parameter contract in one declaration."
)]
pub fn colorzones_descriptor() -> OperationDescriptor {
    let defaults = ColorZonesParametersV5::defaults();
    let mut parameters = vec![enum_parameter(
        "channel",
        &["lightness", "chroma", "hue"],
        "hue",
        1,
        "selection-channel",
    )];
    for curve in 0..COLORZONES_CHANNELS {
        for node in 0..COLORZONES_MAX_NODES {
            parameters.push(scalar_parameter(
                &crate::operation::colorzones::point_name(curve, node, 'x'),
                0.0,
                1.0,
                f64::from(defaults.curves[curve][node].x),
                4,
                None,
                "curve-point-x",
            ));
            parameters.push(scalar_parameter(
                &crate::operation::colorzones::point_name(curve, node, 'y'),
                0.0,
                1.0,
                f64::from(defaults.curves[curve][node].y),
                4,
                None,
                "curve-point-y",
            ));
        }
    }
    for curve in 0..COLORZONES_CHANNELS {
        parameters.push(integer_parameter(
            &crate::operation::colorzones::curve_count_name(curve),
            1,
            i64::try_from(COLORZONES_MAX_NODES).expect("Color Zones node limit fits i64"),
            i64::from(defaults.curve_num_nodes[curve]),
            4,
            "curve-node-count",
        ));
    }
    for curve in 0..COLORZONES_CHANNELS {
        parameters.push(enum_parameter(
            &crate::operation::colorzones::curve_type_name(curve),
            &["cubic", "catmull-rom", "monotone-hermite"],
            "catmull-rom",
            4,
            "curve-interpolation",
        ));
    }
    parameters.push(scalar_parameter(
        "strength",
        -200.0,
        200.0,
        f64::from(defaults.strength),
        3,
        Some("percent"),
        "slider",
    ));
    parameters.push(enum_parameter(
        "mode",
        &["smooth", "strong"],
        "smooth",
        4,
        "processing-mode",
    ));
    parameters.push(enum_parameter(
        "splines_version",
        &["v1", "v2"],
        "v2",
        COLORZONES_SCHEMA_VERSION,
        "spline-version",
    ));

    OperationDescriptor {
        id: DescriptorId::new(
            COLORZONES_COMPATIBILITY_ID,
            COLORZONES_RUST_ID,
            COLORZONES_SCHEMA_VERSION,
            COLORZONES_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Zones ID"),
        parameters,
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-lab-d50".to_owned(),
        roi: RoiKind::Identity,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 1000,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: Some(COLORZONES_GPU_TIER),
            required_features: vec!["lab-boundary".to_owned(), "nearest-lut-storage".to_owned()],
            required_formats: vec!["lab-f32x4".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 Lab D50 with interpolating CPU LUTs and source-exact nearest WGPU LUTs"
                .to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: lab_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: (1..=COLORZONES_SCHEMA_VERSION).collect(),
            target_version: COLORZONES_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.colorzones".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "colorzones".to_owned(),
        }),
    }
}

fn enum_parameter(
    id: &str,
    tags: &[&str],
    default: &str,
    introduced_version: u16,
    ui_hint: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Enum {
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
        },
        default: ParameterDefault::Enum(default.to_owned()),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some(ui_hint.to_owned()),
        condition: None,
    }
}

fn integer_parameter(
    id: &str,
    minimum: i64,
    maximum: i64,
    default: i64,
    introduced_version: u16,
    ui_hint: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer { minimum, maximum },
        default: ParameterDefault::Integer(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: Some(1.0),
        precision: 0,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: false,
        ui_hint: Some(ui_hint.to_owned()),
        condition: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    introduced_version: u16,
    unit: Option<&str>,
    ui_hint: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: unit.map(str::to_owned),
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Color,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some(ui_hint.to_owned()),
        condition: None,
    }
}

fn lab_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
