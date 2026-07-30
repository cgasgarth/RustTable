//! Darktable-compatible vignette at the typed scene-linear RGB boundary.
//!
//! CPU scalar execution is canonical. The plan uses absolute raster indices
//! and therefore remains tile-independent. No WGPU/Lab implementation is
//! claimed: this module consumes and produces only the current `LinearRgb`
//! contract. `unbound` is an explicit persisted clipping choice; the default
//! preserves HDR and negative scene-linear values.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::cast_precision_loss,
    clippy::too_many_lines,
    reason = "descriptor construction and the f32 image contract are intentionally explicit"
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

use super::common::{OperationExecutionError, counter_tpdf, full_image_coordinate, validate_shape};

pub const VIGNETTE_COMPATIBILITY_ID: &str = "vignette";
pub const VIGNETTE_SCHEMA_VERSION: u16 = 4;
pub const VIGNETTE_PARAMETER_BYTES: usize = 64;
pub const VIGNETTE_DEFAULT_SCALE: f32 = 80.0;
pub const VIGNETTE_DEFAULT_FALLOFF_SCALE: f32 = 50.0;
pub const VIGNETTE_DEFAULT_BRIGHTNESS: f32 = -0.5;
pub const VIGNETTE_DEFAULT_SATURATION: f32 = -0.5;

const LEGACY_VIGNETTE_PARAMETER_BYTES: [usize; 3] = [320, 464, 592];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum VignetteDither {
    Off = 0,
    EightBit = 1,
    SixteenBit = 2,
}

impl VignetteDither {
    pub fn from_id(id: u32) -> Result<Self, VignetteParameterError> {
        match id {
            0 => Ok(Self::Off),
            1 => Ok(Self::EightBit),
            2 => Ok(Self::SixteenBit),
            _ => Err(VignetteParameterError::UnsupportedDither(id)),
        }
    }

    const fn amplitude(self) -> f32 {
        match self {
            Self::Off => 0.0,
            Self::EightBit => 1.0 / 256.0,
            Self::SixteenBit => 1.0 / 65_536.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteParametersV4 {
    pub scale: f32,
    pub falloff_scale: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub center: [f32; 2],
    pub autoratio: bool,
    pub whratio: f32,
    pub shape: f32,
    pub dithering: VignetteDither,
    pub unbound: bool,
    pub padding: [u8; 20],
}

impl VignetteParametersV4 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            scale: VIGNETTE_DEFAULT_SCALE,
            falloff_scale: VIGNETTE_DEFAULT_FALLOFF_SCALE,
            brightness: VIGNETTE_DEFAULT_BRIGHTNESS,
            saturation: VIGNETTE_DEFAULT_SATURATION,
            center: [0.0, 0.0],
            autoratio: false,
            whratio: 1.0,
            shape: 1.0,
            dithering: VignetteDither::Off,
            unbound: true,
            padding: [0; 20],
        }
    }

    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        scale: f32,
        falloff_scale: f32,
        brightness: f32,
        saturation: f32,
        center: [f32; 2],
        autoratio: bool,
        whratio: f32,
        shape: f32,
        dithering: VignetteDither,
        unbound: bool,
    ) -> Self {
        Self {
            scale,
            falloff_scale,
            brightness,
            saturation,
            center,
            autoratio,
            whratio,
            shape,
            dithering,
            unbound,
            padding: [0; 20],
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; VIGNETTE_PARAMETER_BYTES] {
        let mut bytes = [0; VIGNETTE_PARAMETER_BYTES];
        for (offset, value) in [
            self.scale,
            self.falloff_scale,
            self.brightness,
            self.saturation,
            self.center[0],
            self.center[1],
        ]
        .into_iter()
        .enumerate()
        {
            bytes[offset * 4..offset * 4 + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[28..32].copy_from_slice(&self.whratio.to_le_bytes());
        bytes[32..36].copy_from_slice(&self.shape.to_le_bytes());
        bytes[24..28].copy_from_slice(&u32::from(self.autoratio).to_le_bytes());
        bytes[36..40].copy_from_slice(&(self.dithering as u32).to_le_bytes());
        bytes[40..44].copy_from_slice(&u32::from(self.unbound).to_le_bytes());
        bytes[44..].copy_from_slice(&self.padding);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, VignetteCodecError> {
        if bytes.len() != VIGNETTE_PARAMETER_BYTES {
            return Err(VignetteCodecError::InvalidLength {
                expected: VIGNETTE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |offset| {
            f32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("checked field"))
        };
        let autoratio = u32::from_le_bytes(bytes[24..28].try_into().expect("flag"));
        let unbound = u32::from_le_bytes(bytes[40..44].try_into().expect("flag"));
        let autoratio = flag(autoratio).ok_or(VignetteCodecError::InvalidFlag("autoratio"))?;
        let unbound = flag(unbound).ok_or(VignetteCodecError::InvalidFlag("unbound"))?;
        let dithering = u32::from_le_bytes(bytes[36..40].try_into().expect("dither"));
        let dithering =
            VignetteDither::from_id(dithering).map_err(VignetteCodecError::Parameters)?;
        let mut padding = [0; 20];
        padding.copy_from_slice(&bytes[44..]);
        Ok(Self {
            scale: read(0),
            falloff_scale: read(4),
            brightness: read(8),
            saturation: read(12),
            center: [read(16), read(20)],
            autoratio,
            whratio: read(28),
            shape: read(32),
            dithering,
            unbound,
            padding,
        })
    }
}

fn flag(value: u32) -> Option<bool> {
    match value {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum VignetteHistory {
    V4(VignetteParametersV4),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl VignetteHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, VignetteCodecError> {
        match version {
            VIGNETTE_SCHEMA_VERSION => Ok(Self::V4(VignetteParametersV4::from_bytes(bytes)?)),
            1..=3 if bytes.len() == LEGACY_VIGNETTE_PARAMETER_BYTES[usize::from(version - 1)] => {
                Ok(Self::Opaque {
                    version,
                    bytes: bytes.to_vec(),
                })
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V4(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V4(_) => VIGNETTE_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    /// Legacy v1-v3 blobs are retained but intentionally not guessed at.
    pub fn current(&self) -> Result<VignetteParametersV4, VignetteCodecError> {
        match self {
            Self::V4(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(VignetteCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VignetteCodecError {
    InvalidLength { expected: usize, actual: usize },
    InvalidFlag(&'static str),
    UnsupportedVersion(u16),
    Parameters(VignetteParameterError),
}

impl fmt::Display for VignetteCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "vignette payload has {actual} bytes; expected {expected}"
                )
            }
            Self::InvalidFlag(name) => write!(formatter, "vignette {name} is not a boolean"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "vignette version {version} is opaque and unsupported"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid vignette parameters: {error}"),
        }
    }
}

impl std::error::Error for VignetteCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VignetteParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
    DegenerateAspect,
    UnsupportedDither(u32),
}

impl fmt::Display for VignetteParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "vignette {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "vignette {name} is out of range"),
            Self::DegenerateAspect => formatter.write_str("vignette aspect ratio is degenerate"),
            Self::UnsupportedDither(id) => write!(formatter, "vignette dither {id} is unsupported"),
        }
    }
}

impl std::error::Error for VignetteParameterError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VignetteConfig {
    scale: FiniteF32,
    falloff_scale: FiniteF32,
    brightness: FiniteF32,
    saturation: FiniteF32,
    center: [FiniteF32; 2],
    autoratio: bool,
    whratio: FiniteF32,
    shape: FiniteF32,
    dithering: VignetteDither,
    unbound: bool,
}

impl TryFrom<VignetteParametersV4> for VignetteConfig {
    type Error = VignetteParameterError;

    fn try_from(parameters: VignetteParametersV4) -> Result<Self, Self::Error> {
        let bounded = |name, value, minimum, maximum| finite_bounded(name, value, minimum, maximum);
        let whratio = finite(parameters.whratio, "whratio")?;
        if !parameters.autoratio && !(0.0..2.0).contains(&whratio.get()) {
            return Err(VignetteParameterError::DegenerateAspect);
        }
        Ok(Self {
            scale: bounded("scale", parameters.scale, 0.0, 200.0)?,
            falloff_scale: bounded("falloff_scale", parameters.falloff_scale, 0.0, 200.0)?,
            brightness: bounded("brightness", parameters.brightness, -1.0, 1.0)?,
            saturation: bounded("saturation", parameters.saturation, -1.0, 1.0)?,
            center: [
                bounded("center_x", parameters.center[0], -1.0, 1.0)?,
                bounded("center_y", parameters.center[1], -1.0, 1.0)?,
            ],
            autoratio: parameters.autoratio,
            whratio,
            shape: bounded("shape", parameters.shape, f32::MIN_POSITIVE, 5.0)?,
            dithering: parameters.dithering,
            unbound: parameters.unbound,
        })
    }
}

impl VignetteConfig {
    pub fn new(parameters: VignetteParametersV4) -> Result<Self, VignetteParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(VignetteParametersV4::defaults()).expect("vignette defaults are valid")
    }

    #[must_use]
    pub const fn parameters(self) -> VignetteParametersV4 {
        VignetteParametersV4::new(
            self.scale.get(),
            self.falloff_scale.get(),
            self.brightness.get(),
            self.saturation.get(),
            [self.center[0].get(), self.center[1].get()],
            self.autoratio,
            self.whratio.get(),
            self.shape.get(),
            self.dithering,
            self.unbound,
        )
    }
}

fn finite(value: f32, name: &'static str) -> Result<FiniteF32, VignetteParameterError> {
    FiniteF32::new(value).map_err(|_| VignetteParameterError::NonFinite(name))
}

fn finite_bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, VignetteParameterError> {
    let value = finite(value, name)?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(VignetteParameterError::OutOfRange(name));
    }
    Ok(value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignettePlan {
    config: VignetteConfig,
    dimensions: RasterDimensions,
    xscale: f32,
    yscale: f32,
    center_x: f32,
    center_y: f32,
    dscale: f32,
    fscale: f32,
    exp1: f32,
    exp2: f32,
    seed: u64,
}

impl VignettePlan {
    pub fn new(
        config: VignetteConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        let (xscale, yscale) = if config.autoratio {
            (
                2.0 / dimensions.width() as f32,
                2.0 / dimensions.height() as f32,
            )
        } else {
            let basis = 2.0 / dimensions.width().max(dimensions.height()) as f32;
            if config.whratio.get() <= 1.0 {
                (basis / config.whratio.get(), basis)
            } else {
                (basis, basis / (2.0 - config.whratio.get()))
            }
        };
        let minimum_falloff = 100.0 / dimensions.width().min(dimensions.height()) as f32;
        let dscale = config.scale.get() / 100.0;
        let fscale = config.falloff_scale.get().max(minimum_falloff) / 100.0;
        let exp1 = 2.0 / config.shape.get();
        let exp2 = config.shape.get() / 2.0;
        if [xscale, yscale, fscale, exp1, exp2]
            .iter()
            .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return Err(OperationExecutionError::UnsupportedCapability(
                "vignette plan has degenerate geometry",
            ));
        }
        Ok(Self {
            config,
            dimensions,
            xscale,
            yscale,
            center_x: 1.0 + config.center[0].get() * dimensions.width() as f32 * xscale / 2.0,
            center_y: 1.0 + config.center[1].get() * dimensions.height() as f32 * yscale / 2.0,
            dscale,
            fscale,
            exp1,
            exp2,
            seed: 0,
        })
    }

    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
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
        let (normalized_x, normalized_y) = full_image_coordinate(self.dimensions, absolute_index);
        let point_x = (normalized_x + 1.0) * self.xscale;
        let point_y = (normalized_y + 1.0) * self.yscale;
        let dx = (point_x - self.center_x).abs();
        let dy = (point_y - self.center_y).abs();
        let cplen = (dx.powf(self.exp1) + dy.powf(self.exp1)).powf(self.exp2);
        let raw_weight = (cplen - self.dscale) / self.fscale;
        let mut weight = raw_weight.clamp(0.0, 1.0);
        let mut dither = 0.0;
        if (0.0..1.0).contains(&raw_weight) && !matches!(self.config.dithering, VignetteDither::Off)
        {
            weight = 0.5 - (std::f32::consts::PI * weight).cos() * 0.5;
            dither =
                self.config.dithering.amplitude() * counter_tpdf(self.seed, absolute_index as u64);
        }
        let mut values = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
        if weight > 0.0 {
            if self.config.brightness.get() < 0.0 {
                let factor = 1.0 + weight * self.config.brightness.get();
                values = values.map(|value| value * factor + dither);
            } else {
                let offset = weight * self.config.brightness.get();
                values = values.map(|value| value + offset + dither);
            }
            if !self.config.unbound {
                values = values.map(|value| value.clamp(0.0, 1.0));
            }
            let mean = (values[0] + values[1] + values[2]) / 3.0;
            let saturation = weight * self.config.saturation.get();
            values = values.map(|value| value - (mean - value) * saturation);
            if !self.config.unbound {
                values = values.map(|value| value.clamp(0.0, 1.0));
            }
        }
        Ok(LinearRgb::new(
            finite_result(values[0], absolute_index, RgbChannel::Red)?,
            finite_result(values[1], absolute_index, RgbChannel::Green)?,
            finite_result(values[2], absolute_index, RgbChannel::Blue)?,
        ))
    }
}

fn finite_result(
    value: f32,
    pixel: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignettePreset {
    pub name: &'static str,
    pub parameters: VignetteParametersV4,
}

pub const VIGNETTE_PRESETS: [VignettePreset; 1] = [VignettePreset {
    name: "lomo",
    parameters: VignetteParametersV4::defaults(),
}];

#[must_use]
pub const fn presets() -> &'static [VignettePreset; 1] {
    &VIGNETTE_PRESETS
}

#[must_use]
pub fn vignette_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64, role| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 4,
        removed_version: None,
        unit: None,
        step: Some(0.01),
        precision: 2,
        role,
        cache_affecting: true,
        animatable: true,
        ui_hint: None,
        condition: None,
    };
    let boolean = |id: &str, default: bool| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Bool,
        default: ParameterDefault::Bool(default),
        required: false,
        introduced_version: 4,
        removed_version: None,
        unit: None,
        step: None,
        precision: 0,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    OperationDescriptor {
        id: DescriptorId::new("vignette", "rusttable.vignette", 4, 4, 1).expect("static ID"),
        parameters: vec![
            scalar("scale", 0.0, 200.0, 80.0, ParameterRole::Processing),
            scalar("falloff_scale", 0.0, 200.0, 50.0, ParameterRole::Processing),
            scalar("brightness", -1.0, 1.0, -0.5, ParameterRole::Processing),
            scalar("saturation", -1.0, 1.0, -0.5, ParameterRole::Color),
            scalar("center_x", -1.0, 1.0, 0.0, ParameterRole::Geometry),
            scalar("center_y", -1.0, 1.0, 0.0, ParameterRole::Geometry),
            boolean("autoratio", false),
            scalar("whratio", 0.0, 2.0, 1.0, ParameterRole::Geometry),
            scalar("shape", 0.0, 5.0, 1.0, ParameterRole::Geometry),
            ParameterDescriptor {
                id: "dithering".to_owned(),
                kind: ParameterKind::Integer {
                    minimum: 0,
                    maximum: 2,
                },
                default: ParameterDefault::Integer(0),
                required: false,
                introduced_version: 4,
                removed_version: None,
                unit: None,
                step: Some(1.0),
                precision: 0,
                role: ParameterRole::Processing,
                cache_affecting: true,
                animatable: false,
                ui_hint: None,
                condition: None,
            },
            boolean("unbound", true),
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
            precision: "f32 scalar full-image coordinates".to_owned(),
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
            source_versions: vec![1, 2, 3, 4],
            target_version: 4,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.vignette".to_owned(),
            group_key: "group.effects".to_owned(),
            control: "vignette".to_owned(),
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
