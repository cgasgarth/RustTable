//! Deprecated Darktable fill-light compatibility at the typed D50 Lab boundary.
//!
//! Darktable's `src/iop/relight.c` consumes four-channel Lab pixels and changes
//! only channel 0. RGB working frames cross this boundary in the evaluator;
//! this module owns the native Lab equation and preserves the remaining
//! channels exactly.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use crate::operations::common::OperationExecutionError;
use crate::{FiniteF32, RasterDimensions, RgbChannel};

pub const RELIGHT_COMPATIBILITY_ID: &str = "relight";
pub const RELIGHT_SCHEMA_VERSION: u16 = 1;
pub const RELIGHT_PARAMETER_BYTES: usize = 12;

pub const RELIGHT_DEFAULT_EV: f32 = 0.33;
pub const RELIGHT_DEFAULT_CENTER: f32 = 0.0;
pub const RELIGHT_DEFAULT_WIDTH: f32 = 4.0;

/// The exact fill-light presets declared by Darktable's legacy module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelightPreset {
    pub name: &'static str,
    pub parameters: RelightParametersV1,
}

pub const RELIGHT_PRESETS: [RelightPreset; 2] = [
    RelightPreset {
        name: "fill-light 0.25EV with 4 zones",
        parameters: RelightParametersV1::new(0.25, 0.25, 4.0),
    },
    RelightPreset {
        name: "fill-shadow -0.25EV with 4 zones",
        parameters: RelightParametersV1::new(-0.25, 0.25, 4.0),
    },
];

#[must_use]
pub const fn presets() -> &'static [RelightPreset; 2] {
    &RELIGHT_PRESETS
}

/// The v1 payload from `dt_iop_relight_params_t`, encoded as little-endian
/// scalar bytes at the `RustTable` history boundary.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelightParametersV1 {
    pub ev: f32,
    pub center: f32,
    pub width: f32,
}

impl RelightParametersV1 {
    #[must_use]
    pub const fn new(ev: f32, center: f32, width: f32) -> Self {
        Self { ev, center, width }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            RELIGHT_DEFAULT_EV,
            RELIGHT_DEFAULT_CENTER,
            RELIGHT_DEFAULT_WIDTH,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; RELIGHT_PARAMETER_BYTES] {
        let mut bytes = [0; RELIGHT_PARAMETER_BYTES];
        for (index, value) in [self.ev, self.center, self.width].into_iter().enumerate() {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RelightCodecError> {
        if bytes.len() != RELIGHT_PARAMETER_BYTES {
            return Err(RelightCodecError::InvalidLength {
                expected: RELIGHT_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |start| {
            f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("validated parameter range"),
            )
        };
        let parameters = Self::new(read(0), read(4), read(8));
        RelightConfig::try_from(parameters).map_err(RelightCodecError::Parameters)?;
        Ok(parameters)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RelightHistory {
    V1(RelightParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl RelightHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, RelightCodecError> {
        if version == RELIGHT_SCHEMA_VERSION {
            Ok(Self::V1(RelightParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => RELIGHT_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelightCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(RelightParameterError),
}

impl fmt::Display for RelightCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "relight payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid relight parameters: {error}"),
        }
    }
}

impl std::error::Error for RelightCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RelightConfig {
    ev: FiniteF32,
    center: FiniteF32,
    width: FiniteF32,
}

/// Four-channel D50 Lab sample in Darktable's native scale: L in 0..100,
/// a/b in -128..128, and an alpha/spare channel in 0..1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelightPixel {
    channels: [f32; 4],
}

impl RelightPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
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

    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

impl RelightConfig {
    pub fn new(ev: f32, center: f32, width: f32) -> Result<Self, RelightParameterError> {
        Self::try_from(RelightParametersV1::new(ev, center, width))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(RelightParametersV1::defaults()).expect("relight defaults are valid")
    }

    #[must_use]
    pub const fn ev(self) -> f32 {
        self.ev.get()
    }

    #[must_use]
    pub const fn center(self) -> f32 {
        self.center.get()
    }

    #[must_use]
    pub const fn width(self) -> f32 {
        self.width.get()
    }
}

impl TryFrom<RelightParametersV1> for RelightConfig {
    type Error = RelightParameterError;

    fn try_from(parameters: RelightParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            ev: bounded("ev", parameters.ev, -2.0, 2.0)?,
            center: bounded("center", parameters.center, 0.0, 1.0)?,
            width: bounded("width", parameters.width, 2.0, 10.0)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelightParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
}

impl fmt::Display for RelightParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "relight {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "relight {name} is outside its range"),
        }
    }
}

impl std::error::Error for RelightParameterError {}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, RelightParameterError> {
    if !value.is_finite() {
        return Err(RelightParameterError::NonFinite(name));
    }
    if !(minimum..=maximum).contains(&value) {
        return Err(RelightParameterError::OutOfRange(name));
    }
    Ok(FiniteF32::new(value).expect("finite value was checked"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelightPlan {
    config: RelightConfig,
    dimensions: RasterDimensions,
}

impl RelightPlan {
    #[must_use]
    pub const fn new(config: RelightConfig, dimensions: RasterDimensions) -> Self {
        Self { config, dimensions }
    }

    /// Executes Darktable's deterministic fill-light transform on D50 Lab.
    pub fn execute_lab<F: FnMut() -> bool>(
        &self,
        input: &[RelightPixel],
        opacity: f32,
        mut cancelled: F,
    ) -> Result<Vec<RelightPixel>, OperationExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        if expected != input.len() {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(OperationExecutionError::NonFiniteResult {
                pixel: 0,
                channel: RgbChannel::Red,
            });
        }
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if self.config.ev().to_bits() == 0.0f32.to_bits() || opacity.to_bits() == 0.0f32.to_bits() {
            return Ok(input.to_vec());
        }
        let b = -1.0 + self.config.center() * 2.0;
        let c = (self.config.width() / 10.0) / 2.0;
        input
            .iter()
            .enumerate()
            .map(|(index, pixel)| {
                if index % usize::try_from(self.dimensions.width()).expect("width fits usize") == 0
                    && cancelled()
                {
                    return Err(OperationExecutionError::Cancelled);
                }
                let source = *pixel;
                if source.channels().iter().any(|value| !value.is_finite()) {
                    return Err(OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: RgbChannel::Red,
                    });
                }
                let candidate = relit_lightness(source.lightness(), self.config, b, c);
                let lightness = source.lightness() + (candidate - source.lightness()) * opacity;
                if !lightness.is_finite() {
                    return Err(OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: RgbChannel::Red,
                    });
                }
                Ok(RelightPixel::new(
                    lightness,
                    source.a(),
                    source.b(),
                    source.alpha(),
                ))
            })
            .collect()
    }

    /// Executes the native Lab operation at full opacity without cancellation.
    pub fn execute(
        &self,
        input: &[RelightPixel],
    ) -> Result<Vec<RelightPixel>, OperationExecutionError> {
        self.execute_lab(input, 1.0, || false)
    }
}

fn relit_lightness(lightness: f32, config: RelightConfig, center: f32, width: f32) -> f32 {
    let normalized = lightness / 100.0;
    let x = -1.0 + normalized * 2.0;
    let gaussian = (-(x - center) * (x - center) / (width * width)).exp();
    let relight = 2.0f32.powf(config.ev() * gaussian.clamp(0.0, 1.0));
    100.0 * (normalized * relight).clamp(0.0, 1.0)
}

#[must_use]
pub fn relight_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new("relight", "rusttable.relight", 1, 1, 1).expect("static ID"),
        parameters: vec![
            scalar("ev", -2.0, 2.0, f64::from(RELIGHT_DEFAULT_EV), "ev"),
            scalar(
                "center",
                0.0,
                1.0,
                f64::from(RELIGHT_DEFAULT_CENTER),
                "normalized",
            ),
            scalar(
                "width",
                2.0,
                10.0,
                f64::from(RELIGHT_DEFAULT_WIDTH),
                "zones",
            ),
        ],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-lab".to_owned(),
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
            required_features: vec!["lab-boundary".to_owned()],
            required_formats: vec!["lab-f32x4".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "f32 scalar Lab lightness".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: lab_io(),
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
            label_key: "operation.relight".to_owned(),
            group_key: "group.tone".to_owned(),
            control: "deprecated-fill-light".to_owned(),
        }),
    }
}

fn scalar(id: &str, minimum: f64, maximum: f64, default: f64, unit: &str) -> ParameterDescriptor {
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
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
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
