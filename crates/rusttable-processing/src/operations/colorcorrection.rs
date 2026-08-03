#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Correction arithmetic order is preserved for IEEE-754 parity."
)]

//! Darktable-compatible Color Correction v1 processing at the Lab D50 boundary.
//!
//! The parameter layout, committed coefficients, presets, CPU equation, flags,
//! and presentation metadata are derived from `src/iop/colorcorrection.c`.
//! The same point equation appears in `data/kernels/basic.cl::colorcorrection`;
//! the qualified WGPU path consumes the committed coefficients below.

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

pub const COLORCORRECTION_COMPATIBILITY_ID: &str = "colorcorrection";
pub const COLORCORRECTION_RUST_ID: &str = "rusttable.colorcorrection";
pub const COLORCORRECTION_SCHEMA_VERSION: u16 = 1;
pub const COLORCORRECTION_V1_PARAMETER_BYTES: usize = 20;
pub const COLORCORRECTION_DEFAULT_HIA: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_HIB: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_LOA: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_LOB: f32 = 0.0;
pub const COLORCORRECTION_DEFAULT_SATURATION: f32 = 1.0;
/// Stable identity of the qualified native Color Correction point primitive.
pub const COLORCORRECTION_WGPU_PASS_ID: &str = "darktable.colorcorrection.point.v1";
/// Minimum device tier for the Color Correction point primitive.
pub const COLORCORRECTION_GPU_TIER: u8 = 1;

/// WGPU passes required by the qualified, mask-free, full-opacity path.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 1] {
    [COLORCORRECTION_WGPU_PASS_ID]
}

/// Native v1 payload in source declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionParametersV1 {
    pub hia: f32,
    pub hib: f32,
    pub loa: f32,
    pub lob: f32,
    pub saturation: f32,
}

impl ColorCorrectionParametersV1 {
    #[must_use]
    pub const fn new(hia: f32, hib: f32, loa: f32, lob: f32, saturation: f32) -> Self {
        Self {
            hia,
            hib,
            loa,
            lob,
            saturation,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            COLORCORRECTION_DEFAULT_HIA,
            COLORCORRECTION_DEFAULT_HIB,
            COLORCORRECTION_DEFAULT_LOA,
            COLORCORRECTION_DEFAULT_LOB,
            COLORCORRECTION_DEFAULT_SATURATION,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORCORRECTION_V1_PARAMETER_BYTES] {
        let mut bytes = [0; COLORCORRECTION_V1_PARAMETER_BYTES];
        for (index, value) in [self.hia, self.hib, self.loa, self.lob, self.saturation]
            .into_iter()
            .enumerate()
        {
            let start = index * std::mem::size_of::<f32>();
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorCorrectionCodecError> {
        if bytes.len() != COLORCORRECTION_V1_PARAMETER_BYTES {
            return Err(ColorCorrectionCodecError::InvalidLength {
                expected: COLORCORRECTION_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut payload = [0_u8; COLORCORRECTION_V1_PARAMETER_BYTES];
        payload.copy_from_slice(bytes);
        let mut values = [0.0_f32; 5];
        let (encoded_values, remainder) = payload.as_chunks::<4>();
        debug_assert!(remainder.is_empty());
        for (value, encoded) in values.iter_mut().zip(encoded_values) {
            *value = f32::from_le_bytes(*encoded);
        }
        let [hia, hib, loa, lob, saturation] = values;
        Ok(Self::new(hia, hib, loa, lob, saturation))
    }
}

/// Typed v1 history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorCorrectionHistory {
    V1(ColorCorrectionParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorCorrectionHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorCorrectionCodecError> {
        if version == COLORCORRECTION_SCHEMA_VERSION {
            Ok(Self::V1(ColorCorrectionParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => COLORCORRECTION_SCHEMA_VERSION,
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

    pub const fn current(&self) -> Result<ColorCorrectionParametersV1, ColorCorrectionCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ColorCorrectionCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorCorrectionCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorCorrectionCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Color Correction payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Color Correction version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for ColorCorrectionCodecError {}

/// Finite runtime/history state in native source order.
///
/// Native `commit_params` does not clamp persisted floats to the GUI bounds,
/// so this accepts every finite value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorCorrectionConfig {
    hia: FiniteF32,
    hib: FiniteF32,
    loa: FiniteF32,
    lob: FiniteF32,
    saturation: FiniteF32,
}

impl ColorCorrectionConfig {
    pub fn new(
        hia: f32,
        hib: f32,
        loa: f32,
        lob: f32,
        saturation: f32,
    ) -> Result<Self, ColorCorrectionParameterError> {
        Self::try_from(ColorCorrectionParametersV1::new(
            hia, hib, loa, lob, saturation,
        ))
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            hia: FiniteF32::from_proven_finite(COLORCORRECTION_DEFAULT_HIA),
            hib: FiniteF32::from_proven_finite(COLORCORRECTION_DEFAULT_HIB),
            loa: FiniteF32::from_proven_finite(COLORCORRECTION_DEFAULT_LOA),
            lob: FiniteF32::from_proven_finite(COLORCORRECTION_DEFAULT_LOB),
            saturation: FiniteF32::from_proven_finite(COLORCORRECTION_DEFAULT_SATURATION),
        }
    }

    #[must_use]
    pub const fn hia(self) -> f32 {
        self.hia.get()
    }

    #[must_use]
    pub const fn hib(self) -> f32 {
        self.hib.get()
    }

    #[must_use]
    pub const fn loa(self) -> f32 {
        self.loa.get()
    }

    #[must_use]
    pub const fn lob(self) -> f32 {
        self.lob.get()
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }

    #[must_use]
    pub const fn parameters(self) -> ColorCorrectionParametersV1 {
        ColorCorrectionParametersV1::new(
            self.hia.get(),
            self.hib.get(),
            self.loa.get(),
            self.lob.get(),
            self.saturation.get(),
        )
    }

    /// Reproduces native `commit_params`, including f32 arithmetic order.
    #[must_use]
    pub const fn committed_coefficients(self) -> ColorCorrectionCoefficients {
        ColorCorrectionCoefficients::new(
            (self.hia.get() - self.loa.get()) / 100.0_f32,
            self.loa.get(),
            (self.hib.get() - self.lob.get()) / 100.0_f32,
            self.lob.get(),
            self.saturation.get(),
        )
    }
}

impl TryFrom<ColorCorrectionParametersV1> for ColorCorrectionConfig {
    type Error = ColorCorrectionParameterError;

    fn try_from(parameters: ColorCorrectionParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            hia: finite("hia", parameters.hia)?,
            hib: finite("hib", parameters.hib)?,
            loa: finite("loa", parameters.loa)?,
            lob: finite("lob", parameters.lob)?,
            saturation: finite("saturation", parameters.saturation)?,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for ColorCorrectionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "Color Correction {name} is non-finite"),
        }
    }
}

impl std::error::Error for ColorCorrectionParameterError {}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, ColorCorrectionParameterError> {
    FiniteF32::new(value).map_err(|_| ColorCorrectionParameterError::NonFinite(name))
}

/// Immutable coefficients generated by native `commit_params`.
///
/// These are plain f32 values because subtracting two valid finite persisted
/// endpoints can overflow exactly as it does in the native implementation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionCoefficients {
    a_scale: f32,
    a_base: f32,
    b_scale: f32,
    b_base: f32,
    saturation: f32,
}

impl ColorCorrectionCoefficients {
    #[must_use]
    pub const fn new(
        a_scale: f32,
        a_base: f32,
        b_scale: f32,
        b_base: f32,
        saturation: f32,
    ) -> Self {
        Self {
            a_scale,
            a_base,
            b_scale,
            b_base,
            saturation,
        }
    }

    #[must_use]
    pub const fn a_scale(self) -> f32 {
        self.a_scale
    }

    #[must_use]
    pub const fn a_base(self) -> f32 {
        self.a_base
    }

    #[must_use]
    pub const fn b_scale(self) -> f32 {
        self.b_scale
    }

    #[must_use]
    pub const fn b_base(self) -> f32 {
        self.b_base
    }

    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation
    }

    /// Source argument order used by the native `OpenCL` kernel.
    #[must_use]
    pub const fn as_array(self) -> [f32; 5] {
        [
            self.saturation,
            self.a_scale,
            self.a_base,
            self.b_scale,
            self.b_base,
        ]
    }
}

/// Native four-channel Lab sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPixel {
    channels: [f32; 4],
}

impl ColorCorrectionPixel {
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPlan {
    config: ColorCorrectionConfig,
    coefficients: ColorCorrectionCoefficients,
}

impl ColorCorrectionPlan {
    #[must_use]
    pub const fn new(config: ColorCorrectionConfig) -> Self {
        Self {
            config,
            coefficients: config.committed_coefficients(),
        }
    }

    #[must_use]
    pub const fn config(self) -> ColorCorrectionConfig {
        self.config
    }

    /// Stable committed-coefficient seam shared by CPU and qualified GPU paths.
    #[must_use]
    pub const fn coefficients(self) -> ColorCorrectionCoefficients {
        self.coefficients
    }

    /// Applies the exact per-pixel Lab equation from the native CPU path.
    #[must_use]
    pub fn execute_lab(&self, input: &[ColorCorrectionPixel]) -> Vec<ColorCorrectionPixel> {
        let coefficients = self.coefficients;
        input
            .iter()
            .map(|pixel| {
                ColorCorrectionPixel::new(
                    pixel.lightness(),
                    coefficients.saturation()
                        * (pixel.a()
                            + pixel.lightness() * coefficients.a_scale()
                            + coefficients.a_base()),
                    coefficients.saturation()
                        * (pixel.b()
                            + pixel.lightness() * coefficients.b_scale()
                            + coefficients.b_base()),
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
        input: &[ColorCorrectionPixel],
        mask: Option<&[f32]>,
        opacity: f32,
    ) -> Vec<ColorCorrectionPixel> {
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
                ColorCorrectionPixel::from_channels(channels)
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionPreset {
    pub name: &'static str,
    pub parameters: ColorCorrectionParametersV1,
    pub enabled: bool,
    pub blend_color_space: ColorCorrectionPresetBlendColorSpace,
}

/// Native blend colorspace stored with each built-in Color Correction preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionPresetBlendColorSpace {
    RgbDisplay,
}

pub const COLORCORRECTION_PRESETS: [ColorCorrectionPreset; 3] = [
    ColorCorrectionPreset {
        name: "warm tone",
        parameters: ColorCorrectionParametersV1::new(0.0, 3.0, 0.0, 0.0, 1.0),
        enabled: true,
        blend_color_space: ColorCorrectionPresetBlendColorSpace::RgbDisplay,
    },
    ColorCorrectionPreset {
        name: "warming filter",
        parameters: ColorCorrectionParametersV1::new(-0.95, 4.5, 3.55, 0.0, 1.0),
        enabled: true,
        blend_color_space: ColorCorrectionPresetBlendColorSpace::RgbDisplay,
    },
    ColorCorrectionPreset {
        name: "cooling filter",
        parameters: ColorCorrectionParametersV1::new(0.95, -4.5, -3.55, -0.0, 1.0),
        enabled: true,
        blend_color_space: ColorCorrectionPresetBlendColorSpace::RgbDisplay,
    },
];

#[must_use]
pub const fn presets() -> &'static [ColorCorrectionPreset; 3] {
    &COLORCORRECTION_PRESETS
}

#[must_use]
///
/// # Panics
///
/// Panics only if the checked-in Color Correction descriptor identity is invalid.
pub fn colorcorrection_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId::new(
            COLORCORRECTION_COMPATIBILITY_ID,
            COLORCORRECTION_RUST_ID,
            COLORCORRECTION_SCHEMA_VERSION,
            COLORCORRECTION_SCHEMA_VERSION,
            1,
        )
        .expect("static Color Correction ID"),
        parameters: vec![
            endpoint_parameter("hia", COLORCORRECTION_DEFAULT_HIA),
            endpoint_parameter("hib", COLORCORRECTION_DEFAULT_HIB),
            endpoint_parameter("loa", COLORCORRECTION_DEFAULT_LOA),
            endpoint_parameter("lob", COLORCORRECTION_DEFAULT_LOB),
            saturation_parameter(),
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
            gpu_tier: Some(COLORCORRECTION_GPU_TIER),
            required_features: vec![
                "f32-storage".to_owned(),
                "deterministic-row-major".to_owned(),
            ],
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: true,
            fallback_to_cpu: true,
            precision: "f32 Lab D50 endpoint interpolation without clamping".to_owned(),
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
            source_versions: vec![COLORCORRECTION_SCHEMA_VERSION],
            target_version: COLORCORRECTION_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.colorcorrection".to_owned(),
            group_key: "group.grading".to_owned(),
            control: "colorcorrection".to_owned(),
        }),
    }
}

fn endpoint_parameter(id: &str, default: f32) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: -40.0,
            maximum: 40.0,
        },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: COLORCORRECTION_SCHEMA_VERSION,
        removed_version: None,
        unit: Some("Lab opponent".to_owned()),
        step: Some(0.1),
        precision: 2,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("color-plane".to_owned()),
        condition: None,
    }
}

fn saturation_parameter() -> ParameterDescriptor {
    ParameterDescriptor {
        id: "saturation".to_owned(),
        kind: ParameterKind::Scalar {
            minimum: -3.0,
            maximum: 3.0,
        },
        default: ParameterDefault::Scalar(f64::from(COLORCORRECTION_DEFAULT_SATURATION)),
        required: false,
        introduced_version: COLORCORRECTION_SCHEMA_VERSION,
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
