#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Contrast arithmetic order is preserved for IEEE-754 parity."
)]

//! Darktable-compatible `colorcontrast` parameters and Lab D50 point processing.
//!
//! The byte codec and scalar equations are derived from
//! `src/iop/colorcontrast.c`. Production evaluation enters this operation only
//! through the explicit Lab D50 boundary in `crate::evaluate::lab_boundary`.

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

pub const COLOR_CONTRAST_COMPATIBILITY_ID: &str = "colorcontrast";
pub const COLOR_CONTRAST_RUST_ID: &str = "rusttable.colorcontrast";
pub const COLOR_CONTRAST_SCHEMA_VERSION: u16 = 2;
pub const COLOR_CONTRAST_V1_PARAMETER_BYTES: usize = 16;
pub const COLOR_CONTRAST_V2_PARAMETER_BYTES: usize = 20;
pub const COLOR_CONTRAST_DEFAULT_A_STEEPNESS: f32 = 1.0;
pub const COLOR_CONTRAST_DEFAULT_A_OFFSET: f32 = 0.0;
pub const COLOR_CONTRAST_DEFAULT_B_STEEPNESS: f32 = 1.0;
pub const COLOR_CONTRAST_DEFAULT_B_OFFSET: f32 = 0.0;
pub const COLOR_CONTRAST_DEFAULT_UNBOUND: i32 = 1;
/// Stable identity of the exact Color Contrast point primitive.
///
/// The registry exposes this binding only after the pixelpipe proves the
/// surrounding Lab D50 boundary and fallback contract.
pub const COLOR_CONTRAST_WGPU_PASS_ID: &str = "darktable.colorcontrast.point.v1";
/// Minimum device tier for the Color Contrast point primitive.
pub const COLOR_CONTRAST_GPU_TIER: u8 = 1;

/// WGPU passes used by the qualified point path.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    [COLOR_CONTRAST_WGPU_PASS_ID]
}

/// Native v1 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorContrastParametersV1 {
    pub a_steepness: f32,
    pub a_offset: f32,
    pub b_steepness: f32,
    pub b_offset: f32,
}

impl ColorContrastParametersV1 {
    #[must_use]
    pub const fn new(a_steepness: f32, a_offset: f32, b_steepness: f32, b_offset: f32) -> Self {
        Self {
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLOR_CONTRAST_V1_PARAMETER_BYTES] {
        encode_f32s([
            self.a_steepness,
            self.a_offset,
            self.b_steepness,
            self.b_offset,
        ])
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        let values = decode_f32s::<4>(bytes, COLOR_CONTRAST_V1_PARAMETER_BYTES)?;
        Ok(Self::new(values[0], values[1], values[2], values[3]))
    }
}

/// Current native v2 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorContrastParametersV2 {
    pub a_steepness: f32,
    pub a_offset: f32,
    pub b_steepness: f32,
    pub b_offset: f32,
    /// Exact native `int`; every nonzero value selects the unbounded branch.
    pub unbound: i32,
}

impl ColorContrastParametersV2 {
    #[must_use]
    pub const fn new(
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: i32,
    ) -> Self {
        Self {
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            COLOR_CONTRAST_DEFAULT_A_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_A_OFFSET,
            COLOR_CONTRAST_DEFAULT_B_STEEPNESS,
            COLOR_CONTRAST_DEFAULT_B_OFFSET,
            COLOR_CONTRAST_DEFAULT_UNBOUND,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLOR_CONTRAST_V2_PARAMETER_BYTES] {
        let mut bytes = [0; COLOR_CONTRAST_V2_PARAMETER_BYTES];
        bytes[..COLOR_CONTRAST_V1_PARAMETER_BYTES].copy_from_slice(
            &ColorContrastParametersV1::new(
                self.a_steepness,
                self.a_offset,
                self.b_steepness,
                self.b_offset,
            )
            .to_bytes(),
        );
        bytes[COLOR_CONTRAST_V1_PARAMETER_BYTES..].copy_from_slice(&self.unbound.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        if bytes.len() != COLOR_CONTRAST_V2_PARAMETER_BYTES {
            return Err(ColorContrastCodecError::InvalidLength {
                expected: COLOR_CONTRAST_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let Some(v1_bytes) = bytes.get(..COLOR_CONTRAST_V1_PARAMETER_BYTES) else {
            return Err(ColorContrastCodecError::InvalidLength {
                expected: COLOR_CONTRAST_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        };
        let Some(unbound_bytes) = bytes
            .get(COLOR_CONTRAST_V1_PARAMETER_BYTES..)
            .and_then(|bytes| <&[u8; 4]>::try_from(bytes).ok())
        else {
            return Err(ColorContrastCodecError::InvalidLength {
                expected: COLOR_CONTRAST_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        };
        let v1 = ColorContrastParametersV1::from_bytes(v1_bytes)?;
        let unbound = i32::from_le_bytes(*unbound_bytes);
        Ok(Self::new(
            v1.a_steepness,
            v1.a_offset,
            v1.b_steepness,
            v1.b_offset,
            unbound,
        ))
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
) -> Result<[f32; FIELDS], ColorContrastCodecError> {
    if bytes.len() != expected {
        return Err(ColorContrastCodecError::InvalidLength {
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
pub enum ColorContrastHistory {
    V1(ColorContrastParametersV1),
    V2(ColorContrastParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorContrastHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorContrastCodecError> {
        match version {
            1 => Ok(Self::V1(ColorContrastParametersV1::from_bytes(bytes)?)),
            COLOR_CONTRAST_SCHEMA_VERSION => {
                Ok(Self::V2(ColorContrastParametersV2::from_bytes(bytes)?))
            }
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
            Self::V2(_) => COLOR_CONTRAST_SCHEMA_VERSION,
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

    pub const fn current(&self) -> Result<ColorContrastParametersV2, ColorContrastCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ColorContrastCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

/// Reproduces Darktable's v1 migration, including the legacy bounded mode.
#[must_use]
pub const fn migrate_v1_to_v2(parameters: ColorContrastParametersV1) -> ColorContrastParametersV2 {
    ColorContrastParametersV2::new(
        parameters.a_steepness,
        parameters.a_offset,
        parameters.b_steepness,
        parameters.b_offset,
        0,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorContrastCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorContrastCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Color Contrast payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Color Contrast version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for ColorContrastCodecError {}

/// Finite runtime/history state.
///
/// Native `commit_params` does not clamp persisted floats, so this accepts
/// every finite value and retains the exact signed `unbound` integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorContrastConfig {
    a_steepness: FiniteF32,
    a_offset: FiniteF32,
    b_steepness: FiniteF32,
    b_offset: FiniteF32,
    unbound: i32,
}

impl ColorContrastConfig {
    pub fn new(
        a_steepness: f32,
        a_offset: f32,
        b_steepness: f32,
        b_offset: f32,
        unbound: i32,
    ) -> Result<Self, ColorContrastParameterError> {
        Self::try_from(ColorContrastParametersV2::new(
            a_steepness,
            a_offset,
            b_steepness,
            b_offset,
            unbound,
        ))
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            a_steepness: FiniteF32::from_proven_finite(COLOR_CONTRAST_DEFAULT_A_STEEPNESS),
            a_offset: FiniteF32::from_proven_finite(COLOR_CONTRAST_DEFAULT_A_OFFSET),
            b_steepness: FiniteF32::from_proven_finite(COLOR_CONTRAST_DEFAULT_B_STEEPNESS),
            b_offset: FiniteF32::from_proven_finite(COLOR_CONTRAST_DEFAULT_B_OFFSET),
            unbound: COLOR_CONTRAST_DEFAULT_UNBOUND,
        }
    }

    #[must_use]
    pub const fn a_steepness(self) -> f32 {
        self.a_steepness.get()
    }

    #[must_use]
    pub const fn a_offset(self) -> f32 {
        self.a_offset.get()
    }

    #[must_use]
    pub const fn b_steepness(self) -> f32 {
        self.b_steepness.get()
    }

    #[must_use]
    pub const fn b_offset(self) -> f32 {
        self.b_offset.get()
    }

    #[must_use]
    pub const fn unbound(self) -> i32 {
        self.unbound
    }

    #[must_use]
    pub const fn is_unbound(self) -> bool {
        self.unbound != 0
    }

    #[must_use]
    pub const fn parameters(self) -> ColorContrastParametersV2 {
        ColorContrastParametersV2::new(
            self.a_steepness.get(),
            self.a_offset.get(),
            self.b_steepness.get(),
            self.b_offset.get(),
            self.unbound,
        )
    }
}

impl TryFrom<ColorContrastParametersV2> for ColorContrastConfig {
    type Error = ColorContrastParameterError;

    fn try_from(parameters: ColorContrastParametersV2) -> Result<Self, Self::Error> {
        Ok(Self {
            a_steepness: finite("a_steepness", parameters.a_steepness)?,
            a_offset: finite("a_offset", parameters.a_offset)?,
            b_steepness: finite("b_steepness", parameters.b_steepness)?,
            b_offset: finite("b_offset", parameters.b_offset)?,
            unbound: parameters.unbound,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorContrastParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for ColorContrastParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "Color Contrast {name} is non-finite"),
        }
    }
}

impl std::error::Error for ColorContrastParameterError {}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, ColorContrastParameterError> {
    FiniteF32::new(value).map_err(|_| ColorContrastParameterError::NonFinite(name))
}

/// Native four-channel Lab sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorContrastPixel {
    channels: [f32; 4],
}

impl ColorContrastPixel {
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
pub struct ColorContrastPlan {
    config: ColorContrastConfig,
}

impl ColorContrastPlan {
    #[must_use]
    pub const fn new(config: ColorContrastConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(self) -> ColorContrastConfig {
        self.config
    }

    /// Applies the exact per-channel Lab equation from the native CPU path.
    #[must_use]
    pub fn execute_lab(&self, input: &[ColorContrastPixel]) -> Vec<ColorContrastPixel> {
        let slope = [
            1.0_f32,
            self.config.a_steepness(),
            self.config.b_steepness(),
            1.0_f32,
        ];
        let offset = [
            0.0_f32,
            self.config.a_offset(),
            self.config.b_offset(),
            0.0_f32,
        ];
        let low = [-f32::MAX, -128.0_f32, -128.0_f32, -f32::MAX];
        let high = [f32::MAX, 128.0_f32, 128.0_f32, f32::MAX];

        input
            .iter()
            .map(|pixel| {
                let channels = pixel.channels();
                let output = std::array::from_fn(|channel| {
                    let scaled = channels[channel] * slope[channel] + offset[channel];
                    if self.config.is_unbound() {
                        scaled
                    } else {
                        darktable_clamps(scaled, low[channel], high[channel])
                    }
                });
                ColorContrastPixel::from_channels(output)
            })
            .collect()
    }

    /// Applies Darktable's default unbounded Lab blend after the module.
    ///
    /// This is the production seam for authored `RustTable` opacity and raster
    /// coverage. Imported Darktable rows remain non-executable because their
    /// arbitrary blend modes, blend-if state, and masks are still opaque.
    ///
    /// As in `_blend_normal_unbounded` from
    /// `src/develop/blends/blendif_lab.c`, channel three receives local
    /// coverage rather than image alpha. `RustTable` carries straight image
    /// alpha in a separate plane and must preserve that plane independently.
    #[must_use]
    pub fn execute_lab_normal_blend(
        &self,
        input: &[ColorContrastPixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<ColorContrastPixel> {
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
                ColorContrastPixel::from_channels(channels)
            })
            .collect()
    }
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
pub fn colorcontrast_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId {
            compatibility_name: COLOR_CONTRAST_COMPATIBILITY_ID.to_owned(),
            rust_id: COLOR_CONTRAST_RUST_ID.to_owned(),
            schema_version: COLOR_CONTRAST_SCHEMA_VERSION,
            parameter_version: COLOR_CONTRAST_SCHEMA_VERSION,
            implementation_version: 1,
        },
        parameters: vec![
            visible_scalar(
                "a_steepness",
                0.0,
                5.0,
                f64::from(COLOR_CONTRAST_DEFAULT_A_STEEPNESS),
                1,
            ),
            hidden_scalar("a_offset", f64::from(COLOR_CONTRAST_DEFAULT_A_OFFSET), 1),
            visible_scalar(
                "b_steepness",
                0.0,
                5.0,
                f64::from(COLOR_CONTRAST_DEFAULT_B_STEEPNESS),
                1,
            ),
            hidden_scalar("b_offset", f64::from(COLOR_CONTRAST_DEFAULT_B_OFFSET), 1),
            hidden_integer(
                "unbound",
                i64::from(COLOR_CONTRAST_DEFAULT_UNBOUND),
                COLOR_CONTRAST_SCHEMA_VERSION,
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
            gpu_tier: Some(COLOR_CONTRAST_GPU_TIER),
            required_features: vec![
                "f32-storage".to_owned(),
                "deterministic-row-major".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 Lab D50 slope/offset with native CLAMPS".to_owned(),
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
            source_versions: vec![1, 2],
            target_version: COLOR_CONTRAST_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.colorcontrast".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "colorcontrast".to_owned(),
        }),
    }
}

fn visible_scalar(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    introduced_version: u16,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: Some("factor".to_owned()),
        step: Some(0.01),
        precision: 2,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn hidden_scalar(id: &str, default: f64, introduced_version: u16) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: f64::from(-f32::MAX),
            maximum: f64::from(f32::MAX),
        },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: None,
        precision: 6,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

fn hidden_integer(id: &str, default: i64, introduced_version: u16) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Integer {
            minimum: i64::from(i32::MIN),
            maximum: i64::from(i32::MAX),
        },
        default: ParameterDefault::Integer(default),
        required: false,
        introduced_version,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
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
