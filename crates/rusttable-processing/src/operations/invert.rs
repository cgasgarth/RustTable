//! Deprecated film-negative inversion for the typed RGB working-image contract.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "the small compatibility codec exposes conventional Result constructors"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

pub const INVERT_COMPATIBILITY_ID: &str = "invert";
pub const INVERT_SCHEMA_VERSION: u16 = 2;
pub const INVERT_V1_PARAMETER_BYTES: usize = 12;
pub const INVERT_V2_PARAMETER_BYTES: usize = 16;
pub const INVERT_CHANNEL4_SENTINEL: f32 = f32::NAN;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvertParametersV1 {
    pub color: [f32; 3],
}

impl InvertParametersV1 {
    #[must_use]
    pub const fn new(color: [f32; 3]) -> Self {
        Self { color }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; INVERT_V1_PARAMETER_BYTES] {
        let mut bytes = [0; INVERT_V1_PARAMETER_BYTES];
        for (index, value) in self.color.into_iter().enumerate() {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InvertCodecError> {
        if bytes.len() != INVERT_V1_PARAMETER_BYTES {
            return Err(InvertCodecError::InvalidLength {
                expected: INVERT_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut color = [0.0; 3];
        for (index, value) in color.iter_mut().enumerate() {
            let start = index * 4;
            *value = f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("validated parameter range"),
            );
        }
        Ok(Self { color })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InvertParametersV2 {
    pub color: [f32; 4],
}

impl InvertParametersV2 {
    #[must_use]
    pub const fn new(color: [f32; 4]) -> Self {
        Self { color }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; INVERT_V2_PARAMETER_BYTES] {
        let mut bytes = [0; INVERT_V2_PARAMETER_BYTES];
        for (index, value) in self.color.into_iter().enumerate() {
            let start = index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, InvertCodecError> {
        if bytes.len() != INVERT_V2_PARAMETER_BYTES {
            return Err(InvertCodecError::InvalidLength {
                expected: INVERT_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut color = [0.0; 4];
        for (index, value) in color.iter_mut().enumerate() {
            let start = index * 4;
            *value = f32::from_le_bytes(
                bytes[start..start + 4]
                    .try_into()
                    .expect("validated parameter range"),
            );
        }
        Ok(Self { color })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum InvertHistory {
    V1(InvertParametersV1),
    V2(InvertParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl InvertHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, InvertCodecError> {
        match version {
            1 => Ok(Self::V1(InvertParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(InvertParametersV2::from_bytes(bytes)?)),
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
}

#[must_use]
pub fn migrate_v1_to_v2(parameters: InvertParametersV1) -> InvertParametersV2 {
    InvertParametersV2::new([
        parameters.color[0],
        parameters.color[1],
        parameters.color[2],
        INVERT_CHANNEL4_SENTINEL,
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvertCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for InvertCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "invert payload has {actual} bytes; expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for InvertCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InvertConfig {
    film_color: [FiniteF32; 4],
    processed_maximum: [FiniteF32; 4],
}

impl InvertConfig {
    pub fn new(
        film_color: [f32; 4],
        processed_maximum: [f32; 4],
    ) -> Result<Self, InvertConfigError> {
        Ok(Self {
            film_color: finite_channels(film_color, InvertConfigError::NonFiniteFilmColor)?,
            processed_maximum: finite_channels(
                processed_maximum,
                InvertConfigError::NonFiniteProcessedMaximum,
            )?,
        })
    }

    pub fn from_v2(parameters: InvertParametersV2) -> Result<Self, InvertConfigError> {
        let mut color = parameters.color;
        if color[3].is_nan() {
            color[3] = 1.0;
        }
        Self::new(color, [1.0; 4])
    }

    #[must_use]
    pub const fn film_color(self) -> [FiniteF32; 4] {
        self.film_color
    }

    #[must_use]
    pub const fn processed_maximum(self) -> [FiniteF32; 4] {
        self.processed_maximum
    }

    pub fn with_processed_maximum(
        self,
        processed_maximum: [f32; 4],
    ) -> Result<Self, InvertConfigError> {
        Self::new(self.film_color.map(FiniteF32::get), processed_maximum)
    }
}

fn finite_channels(
    values: [f32; 4],
    error: InvertConfigError,
) -> Result<[FiniteF32; 4], InvertConfigError> {
    values
        .map(FiniteF32::new)
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .map(|values| values.try_into().expect("four channels are fixed"))
        .map_err(|_| error)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvertConfigError {
    NonFiniteFilmColor,
    NonFiniteProcessedMaximum,
}

impl fmt::Display for InvertConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteFilmColor => "invert film color must be finite",
            Self::NonFiniteProcessedMaximum => "invert processed maximum must be finite",
        })
    }
}

impl std::error::Error for InvertConfigError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvertExecutionError {
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteFilmProduct { channel: usize },
}

impl fmt::Display for InvertExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "invert expected {expected} pixels, got {actual}")
            }
            Self::NonFiniteFilmProduct { channel } => {
                write!(
                    formatter,
                    "invert film product is non-finite in channel {channel}"
                )
            }
        }
    }
}

impl std::error::Error for InvertExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvertPlan {
    config: InvertConfig,
    dimensions: RasterDimensions,
}

impl InvertPlan {
    #[must_use]
    pub const fn new(config: InvertConfig, dimensions: RasterDimensions) -> Self {
        Self { config, dimensions }
    }

    pub fn execute(&self, pixels: &[LinearRgb]) -> Result<Vec<LinearRgb>, InvertExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count()).unwrap_or(usize::MAX);
        if pixels.len() != expected {
            return Err(InvertExecutionError::DimensionsMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        let color = self.config.film_color();
        let maximum = self.config.processed_maximum();
        let products = color
            .into_iter()
            .zip(maximum)
            .enumerate()
            .map(|(channel, (color, maximum))| {
                let value = color.get() * maximum.get();
                value
                    .is_finite()
                    .then_some(value)
                    .ok_or(InvertExecutionError::NonFiniteFilmProduct { channel })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(pixels
            .iter()
            .map(|pixel| {
                LinearRgb::new(
                    clipped_difference(products[0], pixel.red().get()),
                    clipped_difference(products[1], pixel.green().get()),
                    clipped_difference(products[2], pixel.blue().get()),
                )
            })
            .collect())
    }

    #[must_use]
    pub const fn output_processed_maximum() -> [f32; 4] {
        [1.0; 4]
    }
}

fn clipped_difference(film: f32, input: f32) -> FiniteF32 {
    let difference = film - input;
    let value = difference.clamp(0.0, 1.0);
    FiniteF32::new(value).expect("finite film and input yield finite subtraction")
}

#[must_use]
pub fn invert_descriptor() -> OperationDescriptor {
    let parameters = ["red", "green", "blue", "four"]
        .into_iter()
        .map(|id| ParameterDescriptor {
            id: id.to_owned(),
            kind: ParameterKind::Scalar {
                minimum: -f64::from(f32::MAX),
                maximum: f64::from(f32::MAX),
            },
            default: ParameterDefault::Scalar(1.0),
            required: false,
            introduced_version: 1,
            removed_version: None,
            unit: None,
            step: Some(0.001),
            precision: 3,
            role: ParameterRole::Color,
            cache_affecting: true,
            animatable: false,
            ui_hint: None,
            condition: None,
        })
        .collect();
    OperationDescriptor {
        id: DescriptorId::new(INVERT_COMPATIBILITY_ID, "rusttable.invert", 2, 2, 1)
            .expect("static ID"),
        parameters,
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::HIDDEN)
            .insert(OperationFlags::HISTORY_VISIBLE),
        stage: "scene-linear".to_owned(),
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
            fallback_to_cpu: false,
            precision: "f32".to_owned(),
            modes: vec!["rgb".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: None,
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
