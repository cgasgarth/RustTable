#![expect(
    clippy::suboptimal_flops,
    reason = "Native Vibrance arithmetic order is preserved for IEEE-754 parity."
)]

//! Darktable-compatible Vibrance processing at the Lab D50 boundary.
//!
//! The parameter codec, CPU equation, module flags, and presentation metadata
//! are derived from `src/iop/vibrance.c`. The point equation also maps
//! `data/kernels/extended.cl::vibrance`; GPU execution is bound separately.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    reason = "the compatibility codec and closed f32 point operation have conventional contracts"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::FiniteF32;
use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};

pub const VIBRANCE_COMPATIBILITY_ID: &str = "vibrance";
pub const VIBRANCE_RUST_ID: &str = "rusttable.vibrance";
pub const VIBRANCE_SCHEMA_VERSION: u16 = 2;
pub const VIBRANCE_V2_PARAMETER_BYTES: usize = 4;
pub const VIBRANCE_DEFAULT_AMOUNT: f32 = 25.0;
/// Stable identity of the source-derived Vibrance point primitive.
pub const VIBRANCE_WGPU_PASS_ID: &str = "darktable.vibrance.point.v1";
/// Minimum device tier for the Vibrance point primitive.
pub const VIBRANCE_GPU_TIER: u8 = 1;

/// WGPU passes required by the qualified, mask-free, full-opacity path.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    [VIBRANCE_WGPU_PASS_ID]
}

/// Current native v2 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VibranceParametersV2 {
    pub amount: f32,
}

impl VibranceParametersV2 {
    #[must_use]
    pub const fn new(amount: f32) -> Self {
        Self { amount }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(VIBRANCE_DEFAULT_AMOUNT)
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; VIBRANCE_V2_PARAMETER_BYTES] {
        self.amount.to_le_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VibranceCodecError> {
        let amount = <&[u8; VIBRANCE_V2_PARAMETER_BYTES]>::try_from(bytes).map_err(|_| {
            VibranceCodecError::InvalidLength {
                expected: VIBRANCE_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            }
        })?;
        Ok(Self::new(f32::from_le_bytes(*amount)))
    }
}

/// Typed current history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum VibranceHistory {
    V2(VibranceParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl VibranceHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, VibranceCodecError> {
        if version == VIBRANCE_SCHEMA_VERSION {
            Ok(Self::V2(VibranceParametersV2::from_bytes(bytes)?))
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
            Self::V2(_) => VIBRANCE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub const fn current(&self) -> Result<VibranceParametersV2, VibranceCodecError> {
        match self {
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(VibranceCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VibranceCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for VibranceCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Vibrance payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Vibrance version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for VibranceCodecError {}

/// Finite runtime/history state.
///
/// Native `commit_params` does not clamp persisted values to the slider range,
/// so execution accepts every finite amount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VibranceConfig {
    amount: FiniteF32,
}

impl VibranceConfig {
    pub fn new(amount: f32) -> Result<Self, VibranceParameterError> {
        Self::try_from(VibranceParametersV2::new(amount))
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            amount: FiniteF32::from_proven_finite(VIBRANCE_DEFAULT_AMOUNT),
        }
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.amount.get()
    }

    /// Percent amount normalized exactly where native processing does so.
    #[must_use]
    pub const fn normalized_amount(self) -> f32 {
        self.amount.get() * 0.01_f32
    }

    #[must_use]
    pub const fn parameters(self) -> VibranceParametersV2 {
        VibranceParametersV2::new(self.amount.get())
    }
}

impl TryFrom<VibranceParametersV2> for VibranceConfig {
    type Error = VibranceParameterError;

    fn try_from(parameters: VibranceParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            amount: FiniteF32::new(parameters.amount)
                .map_err(|_| VibranceParameterError::NonFinite("amount"))?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VibranceParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for VibranceParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "Vibrance {name} is non-finite"),
        }
    }
}

impl std::error::Error for VibranceParameterError {}

/// Native four-channel Lab sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VibrancePixel {
    channels: [f32; 4],
}

impl VibrancePixel {
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

/// Immutable native point-operation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VibrancePlan {
    config: VibranceConfig,
}

impl VibrancePlan {
    #[must_use]
    pub const fn new(config: VibranceConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(self) -> VibranceConfig {
        self.config
    }

    /// Applies the exact per-pixel Lab equation from the native CPU path.
    #[must_use]
    pub fn execute_lab(&self, input: &[VibrancePixel]) -> Vec<VibrancePixel> {
        let amount = self.config.normalized_amount();
        input
            .iter()
            .map(|pixel| {
                let saturation_weight =
                    (pixel.a() * pixel.a() + pixel.b() * pixel.b()).sqrt() / 256.0_f32;
                let lightness_scale = 1.0_f32 - amount * saturation_weight * 0.25_f32;
                let chroma_scale = 1.0_f32 + amount * saturation_weight;
                VibrancePixel::new(
                    pixel.lightness() * lightness_scale,
                    pixel.a() * chroma_scale,
                    pixel.b() * chroma_scale,
                    pixel.alpha(),
                )
            })
            .collect()
    }

    /// Applies Darktable's default unbounded Lab blend after the module.
    ///
    /// Straight image alpha is carried separately by the production boundary;
    /// channel three here records local blend coverage like the native Lab
    /// normal-blend implementation.
    #[must_use]
    pub fn execute_lab_normal_blend(
        &self,
        input: &[VibrancePixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<VibrancePixel> {
        debug_assert!(mask.is_none_or(|values| values.len() == input.len()));
        let candidates = self.execute_lab(input);
        let inverse_scale = [
            1.0_f32 / 100.0_f32,
            1.0_f32 / 128.0_f32,
            1.0_f32 / 128.0_f32,
        ];
        let scale = [100.0_f32, 128.0_f32, 128.0_f32];

        input
            .iter()
            .zip(candidates)
            .enumerate()
            .map(|(index, (source, candidate))| {
                let source = source.channels();
                let candidate = candidate.channels();
                let coverage = mask.map_or(opacity, |values| values[index] * opacity);
                let channels = std::array::from_fn(|channel| {
                    if channel == 3 {
                        coverage
                    } else {
                        let source = source[channel] * inverse_scale[channel];
                        let candidate = candidate[channel] * inverse_scale[channel];
                        (source * (1.0_f32 - coverage) + candidate * coverage) * scale[channel]
                    }
                });
                VibrancePixel::from_channels(channels)
            })
            .collect()
    }
}

#[must_use]
///
/// # Panics
///
/// Panics only if the checked-in Vibrance descriptor identity is invalid.
pub fn vibrance_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            VIBRANCE_COMPATIBILITY_ID,
            VIBRANCE_RUST_ID,
            VIBRANCE_SCHEMA_VERSION,
            VIBRANCE_SCHEMA_VERSION,
            1,
        )
        .expect("static Vibrance ID"),
        parameters: vec![ParameterDescriptor {
            id: "amount".to_owned(),
            kind: ParameterKind::Scalar {
                minimum: 0.0,
                maximum: 100.0,
            },
            default: ParameterDefault::Scalar(f64::from(VIBRANCE_DEFAULT_AMOUNT)),
            required: false,
            introduced_version: VIBRANCE_SCHEMA_VERSION,
            removed_version: None,
            unit: Some("percent".to_owned()),
            step: Some(1.0),
            precision: 2,
            role: ParameterRole::Processing,
            cache_affecting: true,
            animatable: true,
            ui_hint: Some("slider".to_owned()),
            condition: None,
        }],
        flags: OperationFlags::DEPRECATED
            .insert(OperationFlags::MULTI_INSTANCE)
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
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
            gpu_tier: Some(VIBRANCE_GPU_TIER),
            required_features: vec![
                "f32-storage".to_owned(),
                "deterministic-row-major".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 Lab D50 chroma-weighted scaling".to_owned(),
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
            source_versions: vec![VIBRANCE_SCHEMA_VERSION],
            target_version: VIBRANCE_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.vibrance".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "vibrance".to_owned(),
        }),
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
