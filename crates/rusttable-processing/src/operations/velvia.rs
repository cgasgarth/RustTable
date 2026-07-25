//! Darktable-compatible Velvia saturation at the scene-linear RGB boundary.
//!
//! The history codec retains the exact native `float` field order and uses an
//! explicit little-endian persistence boundary. CPU execution follows
//! `src/iop/velvia.c`, including its comparison-based `CLAMPS` behavior.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "the compatibility codec and closed f32 point operation have conventional contracts"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use crate::{FiniteF32, LinearRgb};

pub const VELVIA_COMPATIBILITY_ID: &str = "velvia";
pub const VELVIA_RUST_ID: &str = "rusttable.velvia";
pub const VELVIA_SCHEMA_VERSION: u16 = 2;
pub const VELVIA_V1_PARAMETER_BYTES: usize = 16;
pub const VELVIA_V2_PARAMETER_BYTES: usize = 8;
pub const VELVIA_DEFAULT_STRENGTH: f32 = 25.0;
pub const VELVIA_DEFAULT_BIAS: f32 = 1.0;
/// Generated shader-registry identity for the Velvia point pass.
pub const VELVIA_WGPU_PASS_ID: &str = "rusttable.point.velvia";
/// Minimum device tier for the generated Velvia point pass.
pub const VELVIA_GPU_TIER: u8 = 1;

/// WGPU passes required by the qualified, mask-free full-opacity path.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    [VELVIA_WGPU_PASS_ID]
}

/// Native v1 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelviaParametersV1 {
    pub saturation: f32,
    pub vibrance: f32,
    pub luminance: f32,
    pub clarity: f32,
}

impl VelviaParametersV1 {
    #[must_use]
    pub const fn new(saturation: f32, vibrance: f32, luminance: f32, clarity: f32) -> Self {
        Self {
            saturation,
            vibrance,
            luminance,
            clarity,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; VELVIA_V1_PARAMETER_BYTES] {
        encode_f32s([self.saturation, self.vibrance, self.luminance, self.clarity])
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VelviaCodecError> {
        let values = decode_f32s::<4>(bytes, VELVIA_V1_PARAMETER_BYTES)?;
        Ok(Self::new(values[0], values[1], values[2], values[3]))
    }
}

/// Current native v2 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelviaParametersV2 {
    pub strength: f32,
    pub bias: f32,
}

impl VelviaParametersV2 {
    #[must_use]
    pub const fn new(strength: f32, bias: f32) -> Self {
        Self { strength, bias }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(VELVIA_DEFAULT_STRENGTH, VELVIA_DEFAULT_BIAS)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; VELVIA_V2_PARAMETER_BYTES] {
        encode_f32s([self.strength, self.bias])
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VelviaCodecError> {
        let values = decode_f32s::<2>(bytes, VELVIA_V2_PARAMETER_BYTES)?;
        Ok(Self::new(values[0], values[1]))
    }
}

fn encode_f32s<const FIELDS: usize, const BYTES: usize>(values: [f32; FIELDS]) -> [u8; BYTES] {
    debug_assert_eq!(FIELDS * std::mem::size_of::<f32>(), BYTES);
    let mut bytes = [0; BYTES];
    for (index, value) in values.into_iter().enumerate() {
        let start = index * std::mem::size_of::<f32>();
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn decode_f32s<const FIELDS: usize>(
    bytes: &[u8],
    expected: usize,
) -> Result<[f32; FIELDS], VelviaCodecError> {
    if bytes.len() != expected {
        return Err(VelviaCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(std::array::from_fn(|index| {
        let start = index * std::mem::size_of::<f32>();
        f32::from_le_bytes(
            bytes[start..start + 4]
                .try_into()
                .expect("payload length was checked"),
        )
    }))
}

/// Typed known history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum VelviaHistory {
    V1(VelviaParametersV1),
    V2(VelviaParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl VelviaHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, VelviaCodecError> {
        match version {
            1 => Ok(Self::V1(VelviaParametersV1::from_bytes(bytes)?)),
            VELVIA_SCHEMA_VERSION => Ok(Self::V2(VelviaParametersV2::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => VELVIA_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
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

    pub fn current(&self) -> Result<VelviaParametersV2, VelviaCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(VelviaCodecError::UnsupportedVersion(*version)),
        }
    }
}

/// Reproduces Darktable's v1 migration using the source f32 operation order.
#[must_use]
pub fn migrate_v1_to_v2(parameters: VelviaParametersV1) -> VelviaParametersV2 {
    VelviaParametersV2::new(
        parameters.saturation * parameters.vibrance / 100.0_f32,
        parameters.luminance,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VelviaCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for VelviaCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Velvia payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Velvia version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for VelviaCodecError {}

/// Finite runtime/history state.
///
/// The descriptor keeps Darktable's slider bounds, but persisted parameters
/// are not clamped by native `commit_params`, so execution accepts every
/// finite source value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VelviaConfig {
    strength: FiniteF32,
    bias: FiniteF32,
}

impl VelviaConfig {
    pub fn new(strength: f32, bias: f32) -> Result<Self, VelviaParameterError> {
        Self::try_from(VelviaParametersV2::new(strength, bias))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(VelviaParametersV2::defaults()).expect("Velvia defaults are valid")
    }

    #[must_use]
    pub const fn strength(self) -> f32 {
        self.strength.get()
    }

    /// Percent strength normalized exactly where native processing does so.
    #[must_use]
    pub const fn normalized_strength(self) -> f32 {
        self.strength.get() / 100.0_f32
    }

    #[must_use]
    pub const fn bias(self) -> f32 {
        self.bias.get()
    }

    #[must_use]
    pub const fn parameters(self) -> VelviaParametersV2 {
        VelviaParametersV2::new(self.strength.get(), self.bias.get())
    }
}

impl TryFrom<VelviaParametersV2> for VelviaConfig {
    type Error = VelviaParameterError;

    fn try_from(parameters: VelviaParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            strength: finite("strength", parameters.strength)?,
            bias: finite("bias", parameters.bias)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VelviaParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for VelviaParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "Velvia {name} is non-finite"),
        }
    }
}

impl std::error::Error for VelviaParameterError {}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, VelviaParameterError> {
    FiniteF32::new(value).map_err(|_| VelviaParameterError::NonFinite(name))
}

/// Four-channel native sample used to prove alpha and byte-copy behavior.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelviaPixel {
    channels: [f32; 4],
}

impl VelviaPixel {
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

    #[must_use]
    pub const fn red(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn green(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn blue(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

/// Immutable point-operation state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VelviaPlan {
    config: VelviaConfig,
}

impl VelviaPlan {
    #[must_use]
    pub const fn new(config: VelviaConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(self) -> VelviaConfig {
        self.config
    }

    /// Applies the native RGB equation. Nonpositive normalized strength
    /// returns a direct copy, preserving every channel bit.
    #[must_use]
    pub fn execute(&self, input: &[LinearRgb]) -> Vec<LinearRgb> {
        let strength = self.config.normalized_strength();
        if strength <= 0.0 {
            return input.to_vec();
        }
        input
            .iter()
            .map(|pixel| {
                let output = velvia_rgb(
                    [pixel.red().get(), pixel.green().get(), pixel.blue().get()],
                    strength,
                    self.config.bias(),
                );
                LinearRgb::new(
                    FiniteF32::new(output[0]).expect("CLAMPS produces finite red"),
                    FiniteF32::new(output[1]).expect("CLAMPS produces finite green"),
                    FiniteF32::new(output[2]).expect("CLAMPS produces finite blue"),
                )
            })
            .collect()
    }

    /// Native four-channel compatibility seam. RGB is processed and alpha is
    /// copied without arithmetic.
    #[must_use]
    pub fn execute_rgba(&self, input: &[VelviaPixel]) -> Vec<VelviaPixel> {
        let strength = self.config.normalized_strength();
        if strength <= 0.0 {
            return input.to_vec();
        }
        input
            .iter()
            .map(|pixel| {
                let output = velvia_rgb(
                    [pixel.red(), pixel.green(), pixel.blue()],
                    strength,
                    self.config.bias(),
                );
                VelviaPixel::new(output[0], output[1], output[2], pixel.alpha())
            })
            .collect()
    }
}

#[allow(
    clippy::manual_midpoint,
    reason = "the native `(max + min) / 2` grouping intentionally retains f32 overflow behavior"
)]
fn velvia_rgb(input: [f32; 3], strength: f32, bias: f32) -> [f32; 3] {
    let pmax = input[0].max(input[1]).max(input[2]);
    let pmin = input[0].min(input[1]).min(input[2]);
    let plum = (pmax + pmin) / 2.0_f32;
    let psat = if plum <= 0.5_f32 {
        (pmax - pmin) / (1e-5_f32 + pmax + pmin)
    } else {
        (pmax - pmin) / (1e-5_f32 + darktable_max(0.0_f32, 2.0_f32 - pmax - pmin))
    };
    let pweight = darktable_clamps(
        ((1.0_f32 - (1.5_f32 * psat))
            + ((1.0_f32 + (plum - 0.5_f32).abs() * 2.0_f32) * (1.0_f32 - bias)))
            / (1.0_f32 + (1.0_f32 - bias)),
        0.0_f32,
        1.0_f32,
    );
    let saturation = strength * pweight;
    [
        velvia_channel(input[0], input[1], input[2], saturation),
        velvia_channel(input[1], input[2], input[0], saturation),
        velvia_channel(input[2], input[0], input[1], saturation),
    ]
}

#[allow(
    clippy::manual_midpoint,
    reason = "the native `0.5 * (other1 + other2)` grouping intentionally retains f32 overflow behavior"
)]
fn velvia_channel(channel: f32, other1: f32, other2: f32, saturation: f32) -> f32 {
    darktable_clamps(
        channel + saturation * (channel - 0.5_f32 * (other1 + other2)),
        0.0_f32,
        1.0_f32,
    )
}

/// Comparison order from `GLib`'s `MAX`, used by the CPU source expression.
fn darktable_max(first: f32, second: f32) -> f32 {
    if first > second { first } else { second }
}

/// Exact comparison order from `CLAMPS(A, L, H)`.
fn darktable_clamps(value: f32, lower: f32, upper: f32) -> f32 {
    if value > lower {
        if value < upper { value } else { upper }
    } else {
        lower
    }
}

#[must_use]
pub fn velvia_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            VELVIA_COMPATIBILITY_ID,
            VELVIA_RUST_ID,
            VELVIA_SCHEMA_VERSION,
            VELVIA_SCHEMA_VERSION,
            1,
        )
        .expect("static Velvia ID"),
        parameters: vec![
            scalar(
                "strength",
                0.0,
                100.0,
                f64::from(VELVIA_DEFAULT_STRENGTH),
                "percent",
                1.0,
            ),
            scalar(
                "bias",
                0.0,
                1.0,
                f64::from(VELVIA_DEFAULT_BIAS),
                "normalized",
                0.01,
            ),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::DETERMINISTIC_GPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
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
            gpu_tier: Some(VELVIA_GPU_TIER),
            required_features: vec![
                "f32-storage".to_owned(),
                "deterministic-row-major".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 scalar with native CLAMPS".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: rgb_io(),
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![1, 2],
            target_version: VELVIA_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.velvia".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "velvia".to_owned(),
        }),
    }
}

fn scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
    step: f64,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: VELVIA_SCHEMA_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(step),
        precision: 2,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
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
