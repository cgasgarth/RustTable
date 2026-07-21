//! Darktable-compatible graduated neutral-density filter at the typed RGB boundary.
//!
//! This is the CPU scalar point transform from Darktable's `graduatednd`
//! contract. Rotation and offset are compiled into one full-image line
//! equation; callers may execute row/tile windows without changing it. The
//! current processing boundary is scene-linear RGB, so this module does not
//! claim Lab conversion, WGPU parity, or hidden clipping.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::too_many_lines,
    reason = "the persisted f32 contract and explicit descriptor are compatibility boundaries"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel};

use super::common::{OperationExecutionError, full_image_coordinate, validate_shape};

pub const GRADUATED_ND_COMPATIBILITY_ID: &str = "graduatednd";
pub const GRADUATED_ND_SCHEMA_VERSION: u16 = 1;
pub const GRADUATED_ND_PARAMETER_BYTES: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraduatedNdParametersV1 {
    pub density: f32,
    pub hardness: f32,
    pub rotation: f32,
    pub offset: f32,
    pub hue: f32,
    pub saturation: f32,
}

impl GraduatedNdParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            density: 1.0,
            hardness: 0.0,
            rotation: 0.0,
            offset: 50.0,
            hue: 0.0,
            saturation: 0.0,
        }
    }

    #[must_use]
    pub const fn new(
        density: f32,
        hardness: f32,
        rotation: f32,
        offset: f32,
        hue: f32,
        saturation: f32,
    ) -> Self {
        Self {
            density,
            hardness,
            rotation,
            offset,
            hue,
            saturation,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; GRADUATED_ND_PARAMETER_BYTES] {
        let mut bytes = [0; GRADUATED_ND_PARAMETER_BYTES];
        for (index, value) in [
            self.density,
            self.hardness,
            self.rotation,
            self.offset,
            self.hue,
            self.saturation,
        ]
        .into_iter()
        .enumerate()
        {
            bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, GraduatedNdCodecError> {
        if bytes.len() != GRADUATED_ND_PARAMETER_BYTES {
            return Err(GraduatedNdCodecError::InvalidLength {
                expected: GRADUATED_ND_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |index: usize| {
            f32::from_le_bytes(bytes[index * 4..index * 4 + 4].try_into().expect("field"))
        };
        Ok(Self::new(
            read(0),
            read(1),
            read(2),
            read(3),
            read(4),
            read(5),
        ))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GraduatedNdHistory {
    V1(GraduatedNdParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl GraduatedNdHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, GraduatedNdCodecError> {
        if version == GRADUATED_ND_SCHEMA_VERSION {
            Ok(Self::V1(GraduatedNdParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => GRADUATED_ND_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraduatedNdCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(GraduatedNdParameterError),
}

impl fmt::Display for GraduatedNdCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "graduatednd payload has {actual} bytes; expected {expected}"
            ),
            Self::Parameters(error) => write!(formatter, "invalid graduatednd parameters: {error}"),
        }
    }
}

impl std::error::Error for GraduatedNdCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraduatedNdParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for GraduatedNdParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "graduatednd {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "graduatednd {name} is out of range"),
        }
    }
}

impl std::error::Error for GraduatedNdParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GraduatedNdConfig {
    density: FiniteF32,
    hardness: FiniteF32,
    rotation: FiniteF32,
    offset: FiniteF32,
    hue: FiniteF32,
    saturation: FiniteF32,
}

impl TryFrom<GraduatedNdParametersV1> for GraduatedNdConfig {
    type Error = GraduatedNdParameterError;

    fn try_from(parameters: GraduatedNdParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            density: bounded("density", parameters.density, -8.0, 8.0)?,
            hardness: bounded("hardness", parameters.hardness, 0.0, 100.0)?,
            rotation: bounded("rotation", parameters.rotation, -180.0, 180.0)?,
            offset: finite(parameters.offset, "offset")?,
            hue: bounded("hue", parameters.hue, 0.0, 1.0)?,
            saturation: bounded("saturation", parameters.saturation, 0.0, 1.0)?,
        })
    }
}

impl GraduatedNdConfig {
    pub fn new(parameters: GraduatedNdParametersV1) -> Result<Self, GraduatedNdParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(GraduatedNdParametersV1::defaults()).expect("graduatednd defaults are valid")
    }

    #[must_use]
    pub const fn parameters(self) -> GraduatedNdParametersV1 {
        GraduatedNdParametersV1::new(
            self.density.get(),
            self.hardness.get(),
            self.rotation.get(),
            self.offset.get(),
            self.hue.get(),
            self.saturation.get(),
        )
    }
}

fn finite(value: f32, name: &'static str) -> Result<FiniteF32, GraduatedNdParameterError> {
    FiniteF32::new(value).map_err(|_| GraduatedNdParameterError::NonFinite(name))
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, GraduatedNdParameterError> {
    let value = finite(value, name)?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(GraduatedNdParameterError::OutOfRange(name));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraduatedNdPlan {
    config: GraduatedNdConfig,
    dimensions: RasterDimensions,
    sinv: f32,
    cosv: f32,
    transition_scale: f32,
    color: [f32; 3],
    color1: [f32; 3],
}

impl GraduatedNdPlan {
    pub fn new(
        config: GraduatedNdConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let angle = -config.rotation.get().to_radians();
        let sinv = angle.sin();
        let cosv = angle.cos();
        let filter_radius = ((dimensions.width() as f32).hypot(dimensions.height() as f32))
            / dimensions.height() as f32;
        let hardness = 0.5 - config.hardness.get() / 100.0 * 0.45;
        let transition_scale = 0.5 / (filter_radius * hardness);
        if !transition_scale.is_finite() || transition_scale <= 0.0 {
            return Err(OperationExecutionError::UnsupportedCapability(
                "graduatednd transition is degenerate",
            ));
        }
        let mut color = hsl_to_rgb(config.hue.get(), config.saturation.get());
        if config.density.get() < 0.0 {
            color = color.map(|value| 1.0 - value);
        }
        Ok(Self {
            config,
            dimensions,
            sinv,
            cosv,
            transition_scale,
            color,
            color1: color.map(|value| 1.0 - value),
        })
    }

    pub fn execute(&self, input: &[LinearRgb]) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        self.execute_window(input, 0)
    }

    pub fn execute_window(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        let total = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        let end = pixel_index_offset.checked_add(input.len()).ok_or(
            OperationExecutionError::DimensionsMismatch {
                expected: total,
                actual: input.len(),
            },
        )?;
        if end > total {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected: total,
                actual: end,
            });
        }
        input
            .iter()
            .enumerate()
            .map(|(local_index, pixel)| self.transform(*pixel, pixel_index_offset + local_index))
            .collect()
    }

    fn transform(
        &self,
        pixel: LinearRgb,
        absolute_index: usize,
    ) -> Result<LinearRgb, OperationExecutionError> {
        let (x, y) = full_image_coordinate(self.dimensions, absolute_index);
        let offset = self.config.offset.get() / 100.0 * 2.0;
        let length = (self.sinv * x + self.cosv * y - 1.0 + offset) * self.transition_scale;
        let d = 2.0f32.powf(self.config.density.get() * (0.5 + length).clamp(0.0, 1.0));
        let factor = [
            self.color[0] + self.color1[0] * d,
            self.color[1] + self.color1[1] * d,
            self.color[2] + self.color1[2] * d,
        ];
        let values = if self.config.density.get() >= 0.0 {
            [
                pixel.red().get() / factor[0],
                pixel.green().get() / factor[1],
                pixel.blue().get() / factor[2],
            ]
        } else {
            [
                pixel.red().get() * factor[0],
                pixel.green().get() * factor[1],
                pixel.blue().get() * factor[2],
            ]
        };
        Ok(LinearRgb::new(
            finite_result(values[0], absolute_index, RgbChannel::Red)?,
            finite_result(values[1], absolute_index, RgbChannel::Green)?,
            finite_result(values[2], absolute_index, RgbChannel::Blue)?,
        ))
    }
}

fn hsl_to_rgb(hue: f32, saturation: f32) -> [f32; 3] {
    if saturation.to_bits() == 0 {
        return [0.5; 3];
    }
    let hue = hue * 6.0;
    let chroma = saturation;
    let x = chroma * (1.0 - ((hue % 2.0) - 1.0).abs());
    let rgb = match hue as u32 {
        0 => [chroma, x, 0.0],
        1 => [x, chroma, 0.0],
        2 => [0.0, chroma, x],
        3 => [0.0, x, chroma],
        4 => [x, 0.0, chroma],
        _ => [chroma, 0.0, x],
    };
    rgb.map(|value| value + (1.0 - chroma) * 0.5)
}

fn finite_result(
    value: f32,
    pixel: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraduatedNdPreset {
    pub name: &'static str,
    pub parameters: GraduatedNdParametersV1,
}

pub const GRADUATED_ND_PRESETS: [GraduatedNdPreset; 13] = [
    preset("neutral gray | ND2 (soft)", 1.0, 0.0, 0.0, 50.0, 0.0, 0.0),
    preset("neutral gray | ND4 (soft)", 2.0, 0.0, 0.0, 50.0, 0.0, 0.0),
    preset("neutral gray | ND8 (soft)", 3.0, 0.0, 0.0, 50.0, 0.0, 0.0),
    preset("neutral gray | ND2 (hard)", 1.0, 75.0, 0.0, 50.0, 0.0, 0.0),
    preset("neutral gray | ND4 (hard)", 2.0, 75.0, 0.0, 50.0, 0.0, 0.0),
    preset("neutral gray | ND8 (hard)", 3.0, 75.0, 0.0, 50.0, 0.0, 0.0),
    preset(
        "tinted | orange ND2 (soft)",
        1.0,
        0.0,
        0.0,
        50.0,
        0.102_439,
        0.8,
    ),
    preset(
        "tinted | yellow ND2 (soft)",
        1.0,
        0.0,
        0.0,
        50.0,
        0.151_220,
        0.5,
    ),
    preset(
        "tinted | purple ND2 (soft)",
        1.0,
        0.0,
        0.0,
        50.0,
        0.824_390,
        0.5,
    ),
    preset(
        "tinted | green ND2 (soft)",
        1.0,
        0.0,
        0.0,
        50.0,
        0.302_439,
        0.5,
    ),
    preset("tinted | red ND2 (soft)", 1.0, 0.0, 0.0, 50.0, 0.0, 0.5),
    preset(
        "tinted | blue ND2 (soft)",
        1.0,
        0.0,
        0.0,
        50.0,
        0.663_415,
        0.5,
    ),
    preset(
        "tinted | brown ND4 (soft)",
        2.0,
        0.0,
        0.0,
        50.0,
        0.082_927,
        0.25,
    ),
];

const fn preset(
    name: &'static str,
    density: f32,
    hardness: f32,
    rotation: f32,
    offset: f32,
    hue: f32,
    saturation: f32,
) -> GraduatedNdPreset {
    GraduatedNdPreset {
        name,
        parameters: GraduatedNdParametersV1::new(
            density, hardness, rotation, offset, hue, saturation,
        ),
    }
}

#[must_use]
pub const fn presets() -> &'static [GraduatedNdPreset; 13] {
    &GRADUATED_ND_PRESETS
}

#[must_use]
pub fn graduatednd_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64, unit: &str, role| {
        ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar { minimum, maximum },
            default: ParameterDefault::Scalar(default),
            required: false,
            introduced_version: 1,
            removed_version: None,
            unit: Some(unit.to_owned()),
            step: Some(0.01),
            precision: 2,
            role,
            cache_affecting: true,
            animatable: true,
            ui_hint: None,
            condition: None,
        }
    };
    OperationDescriptor {
        id: DescriptorId::new("graduatednd", "rusttable.graduatednd", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar("density", -8.0, 8.0, 1.0, "ev", ParameterRole::Processing),
            scalar(
                "hardness",
                0.0,
                100.0,
                0.0,
                "percent",
                ParameterRole::Processing,
            ),
            scalar(
                "rotation",
                -180.0,
                180.0,
                0.0,
                "degrees",
                ParameterRole::Geometry,
            ),
            scalar(
                "offset",
                -10_000.0,
                10_000.0,
                50.0,
                "percent",
                ParameterRole::Geometry,
            ),
            scalar("hue", 0.0, 1.0, 0.0, "normalized", ParameterRole::Color),
            scalar(
                "saturation",
                0.0,
                1.0,
                0.0,
                "normalized",
                ParameterRole::Color,
            ),
        ],
        flags: OperationFlags::STYLE_ELIGIBLE
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear-rgb".to_owned(),
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
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: Vec::new(),
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 scalar full-image line".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1],
            target_version: 1,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.graduatednd".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "graduatednd".to_owned(),
        }),
    }
}

fn rgb_io() -> InputOutputContract {
    let image = ImagePredicate {
        channels: 3,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LinearSrgbD65],
        nonfinite: NonFinitePolicy::Reject,
    };
    InputOutputContract {
        input: image.clone(),
        output: image,
        derives_output_encoding: false,
    }
}
