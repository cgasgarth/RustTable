//! Deterministic dither and posterization for the typed RGB working image.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "legacy arithmetic and compact compatibility codecs are range-checked locally"
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
    UiHint,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions};

pub const DITHER_COMPATIBILITY_ID: &str = "dither";
pub const DITHER_SCHEMA_VERSION: u16 = 2;
pub const DITHER_V1_PARAMETER_BYTES: usize = 168;
pub const DITHER_V2_PARAMETER_BYTES: usize = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DitherMethod {
    Random,
    Fs1BitGray,
    Fs4BitGray,
    Fs8BitRgb,
    Fs16BitRgb,
    FsAuto,
    Fs1BitRgb,
    Fs2BitGray,
    Fs2BitRgb,
    Fs4BitRgb,
    Fs6BitGray,
    Posterize(u8),
}

impl DitherMethod {
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            Self::Random => 0,
            Self::Fs1BitGray => 1,
            Self::Fs4BitGray => 2,
            Self::Fs8BitRgb => 3,
            Self::Fs16BitRgb => 4,
            Self::FsAuto => 5,
            Self::Fs1BitRgb => 6,
            Self::Fs2BitGray => 7,
            Self::Fs2BitRgb => 8,
            Self::Fs4BitRgb => 9,
            Self::Fs6BitGray => 10,
            Self::Posterize(levels) => 0x100 + levels.saturating_sub(2) as u32,
        }
    }

    pub fn from_id(id: u32) -> Result<Self, DitherMethodError> {
        match id {
            0 => Ok(Self::Random),
            1 => Ok(Self::Fs1BitGray),
            2 => Ok(Self::Fs4BitGray),
            3 => Ok(Self::Fs8BitRgb),
            4 => Ok(Self::Fs16BitRgb),
            5 => Ok(Self::FsAuto),
            6 => Ok(Self::Fs1BitRgb),
            7 => Ok(Self::Fs2BitGray),
            8 => Ok(Self::Fs2BitRgb),
            9 => Ok(Self::Fs4BitRgb),
            10 => Ok(Self::Fs6BitGray),
            0x100..=0x107 => Ok(Self::Posterize((id - 0x100) as u8 + 2)),
            _ => Err(DitherMethodError::Unknown(id)),
        }
    }

    #[must_use]
    pub const fn is_floyd_steinberg(self) -> bool {
        !matches!(self, Self::Random | Self::Posterize(_))
    }

    #[must_use]
    pub const fn is_gray(self) -> bool {
        matches!(
            self,
            Self::Fs1BitGray | Self::Fs4BitGray | Self::Fs2BitGray | Self::Fs6BitGray
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherMethodError {
    Unknown(u32),
}

impl fmt::Display for DitherMethodError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown(id) => write!(formatter, "unknown dither method {id}"),
        }
    }
}

impl std::error::Error for DitherMethodError {}

#[derive(Debug, Clone, PartialEq)]
pub struct DitherParametersV1 {
    pub method_id: u32,
    pub palette: i32,
    pub radius: f32,
    pub range: [f32; 4],
    pub damping: f32,
    pub legacy_tail: [u8; DITHER_V1_PARAMETER_BYTES - DITHER_V2_PARAMETER_BYTES],
}

impl DitherParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            method_id: DitherMethod::FsAuto.id(),
            palette: 0,
            radius: 0.0,
            range: [0.0, 0.0, 1.0, 1.0],
            damping: -200.0,
            legacy_tail: [0; DITHER_V1_PARAMETER_BYTES - DITHER_V2_PARAMETER_BYTES],
        }
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; DITHER_V1_PARAMETER_BYTES] {
        let mut bytes = [0; DITHER_V1_PARAMETER_BYTES];
        encode_fields(
            &mut bytes[..DITHER_V2_PARAMETER_BYTES],
            self.method_id,
            self.palette,
            self.radius,
            self.range,
            self.damping,
        );
        bytes[DITHER_V2_PARAMETER_BYTES..].copy_from_slice(&self.legacy_tail);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DitherCodecError> {
        if bytes.len() != DITHER_V1_PARAMETER_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let (method_id, palette, radius, range, damping) = decode_fields(bytes);
        let mut legacy_tail = [0; DITHER_V1_PARAMETER_BYTES - DITHER_V2_PARAMETER_BYTES];
        legacy_tail.copy_from_slice(&bytes[DITHER_V2_PARAMETER_BYTES..]);
        Ok(Self {
            method_id,
            palette,
            radius,
            range,
            damping,
            legacy_tail,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DitherParametersV2 {
    pub method_id: u32,
    pub palette: i32,
    pub radius: f32,
    pub range: [f32; 4],
    pub damping: f32,
    pub padding: [u8; 4],
    opaque_source: Option<Vec<u8>>,
}

impl DitherParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            method_id: DitherMethod::FsAuto.id(),
            palette: 0,
            radius: 0.0,
            range: [0.0, 0.0, 1.0, 1.0],
            damping: -200.0,
            padding: [0; 4],
            opaque_source: None,
        }
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; DITHER_V2_PARAMETER_BYTES] {
        let mut bytes = [0; DITHER_V2_PARAMETER_BYTES];
        encode_fields(
            &mut bytes[..32],
            self.method_id,
            self.palette,
            self.radius,
            self.range,
            self.damping,
        );
        bytes[32..].copy_from_slice(&self.padding);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DitherCodecError> {
        if bytes.len() != DITHER_V2_PARAMETER_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let (method_id, palette, radius, range, damping) = decode_fields(bytes);
        let mut padding = [0; 4];
        padding.copy_from_slice(&bytes[32..]);
        Ok(Self {
            method_id,
            palette,
            radius,
            range,
            damping,
            padding,
            opaque_source: None,
        })
    }

    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }
}

fn encode_fields(
    bytes: &mut [u8],
    method_id: u32,
    palette: i32,
    radius: f32,
    range: [f32; 4],
    damping: f32,
) {
    bytes[0..4].copy_from_slice(&method_id.to_le_bytes());
    bytes[4..8].copy_from_slice(&palette.to_le_bytes());
    bytes[8..12].copy_from_slice(&radius.to_le_bytes());
    for (index, value) in range.into_iter().enumerate() {
        let start = 12 + index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
    bytes[28..32].copy_from_slice(&damping.to_le_bytes());
}

fn decode_fields(bytes: &[u8]) -> (u32, i32, f32, [f32; 4], f32) {
    let method_id = u32::from_le_bytes(bytes[0..4].try_into().expect("method range"));
    let palette = i32::from_le_bytes(bytes[4..8].try_into().expect("palette range"));
    let radius = f32::from_le_bytes(bytes[8..12].try_into().expect("radius range"));
    let mut range = [0.0; 4];
    for (index, value) in range.iter_mut().enumerate() {
        let start = 12 + index * 4;
        *value = f32::from_le_bytes(bytes[start..start + 4].try_into().expect("range"));
    }
    let damping = f32::from_le_bytes(bytes[28..32].try_into().expect("damping range"));
    (method_id, palette, radius, range, damping)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DitherHistory {
    V1(DitherParametersV1),
    V2(DitherParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl DitherHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, DitherCodecError> {
        match version {
            1 => {
                let parameters = DitherParametersV1::from_bytes(bytes)?;
                if DitherMethod::from_id(parameters.method_id).is_err() {
                    return Ok(Self::Opaque {
                        version,
                        bytes: bytes.to_vec(),
                    });
                }
                validate_fields(parameters.radius, parameters.range, parameters.damping)?;
                Ok(Self::V1(parameters))
            }
            2 => {
                let parameters = DitherParametersV2::from_bytes(bytes)?;
                if DitherMethod::from_id(parameters.method_id).is_err() {
                    return Ok(Self::Opaque {
                        version,
                        bytes: bytes.to_vec(),
                    });
                }
                validate_fields(parameters.radius, parameters.range, parameters.damping)?;
                Ok(Self::V2(parameters))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn migrate_v1(&self) -> Option<DitherParametersV2> {
        let Self::V1(parameters) = self else {
            return None;
        };
        Some(DitherParametersV2 {
            method_id: parameters.method_id,
            palette: parameters.palette,
            radius: parameters.radius,
            range: parameters.range,
            damping: parameters.damping,
            padding: parameters.legacy_tail[..4]
                .try_into()
                .expect("four padding bytes"),
            opaque_source: Some(parameters.to_bytes().to_vec()),
        })
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DitherCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownMethod(DitherMethodError),
    InvalidFields(DitherConfigError),
}

impl fmt::Display for DitherCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "dither payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnknownMethod(error) => error.fmt(formatter),
            Self::InvalidFields(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DitherCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DitherConfig {
    method: DitherMethod,
    damping: FiniteF32,
    seed: u64,
}

impl DitherConfig {
    pub fn new(method: DitherMethod, damping: f32) -> Result<Self, DitherConfigError> {
        if let DitherMethod::Posterize(levels) = method
            && !(2..=8).contains(&levels)
        {
            return Err(DitherConfigError::InvalidPosterizeLevels(levels));
        }
        let damping = FiniteF32::new(damping).map_err(|_| DitherConfigError::NonFiniteDamping)?;
        if !(-200.0..=0.0).contains(&damping.get()) {
            return Err(DitherConfigError::DampingOutOfRange(damping.get()));
        }
        Ok(Self {
            method,
            damping,
            seed: 0,
        })
    }

    #[must_use]
    pub const fn method(self) -> DitherMethod {
        self.method
    }

    #[must_use]
    pub const fn damping(self) -> FiniteF32 {
        self.damping
    }

    #[must_use]
    pub const fn seed(self) -> u64 {
        self.seed
    }

    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DitherConfigError {
    NonFiniteDamping,
    DampingOutOfRange(f32),
    InvalidPosterizeLevels(u8),
    NonFiniteReservedField,
    InvalidReservedRange,
}

impl fmt::Display for DitherConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteDamping => formatter.write_str("dither damping must be finite"),
            Self::DampingOutOfRange(value) => {
                write!(formatter, "dither damping {value} is outside -200..=0")
            }
            Self::InvalidPosterizeLevels(levels) => {
                write!(formatter, "posterize requires 2..=8 levels, got {levels}")
            }
            Self::NonFiniteReservedField => {
                formatter.write_str("dither reserved fields must be finite")
            }
            Self::InvalidReservedRange => {
                formatter.write_str("dither reserved range must be ordered and normalized")
            }
        }
    }
}

impl std::error::Error for DitherConfigError {}

fn validate_fields(radius: f32, range: [f32; 4], damping: f32) -> Result<(), DitherCodecError> {
    if !radius.is_finite() || range.iter().any(|value| !value.is_finite()) {
        return Err(DitherCodecError::InvalidFields(
            DitherConfigError::NonFiniteReservedField,
        ));
    }
    if range[0] > range[2]
        || range[1] > range[3]
        || range.iter().any(|value| !(0.0..=1.0).contains(value))
    {
        return Err(DitherCodecError::InvalidFields(
            DitherConfigError::InvalidReservedRange,
        ));
    }
    DitherConfig::new(DitherMethod::FsAuto, damping)
        .map(|_| ())
        .map_err(DitherCodecError::InvalidFields)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DitherBitDepth {
    BlackWhite,
    Int8,
    Int10,
    Int12,
    Int16,
    Float,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DitherRenderContext {
    scale: FiniteF32,
    bit_depth: DitherBitDepth,
    export: bool,
    preview: bool,
    gray: bool,
}

impl DitherRenderContext {
    pub fn new(
        scale: f32,
        bit_depth: DitherBitDepth,
        export: bool,
        preview: bool,
        gray: bool,
    ) -> Result<Self, DitherContextError> {
        let scale = FiniteF32::new(scale).map_err(|_| DitherContextError::NonFiniteScale)?;
        if scale.get() <= 0.0 {
            return Err(DitherContextError::InvalidScale);
        }
        Ok(Self {
            scale,
            bit_depth,
            export,
            preview,
            gray,
        })
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            scale: FiniteF32::from_proven_finite(1.0),
            bit_depth: DitherBitDepth::Float,
            export: false,
            preview: false,
            gray: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherContextError {
    NonFiniteScale,
    InvalidScale,
}

impl fmt::Display for DitherContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteScale => "dither scale must be finite",
            Self::InvalidScale => "dither scale must be greater than zero",
        })
    }
}

impl std::error::Error for DitherContextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherExecutionError {
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteOutput { pixel: usize, channel: usize },
}

impl fmt::Display for DitherExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => {
                write!(formatter, "dither expected {expected} pixels, got {actual}")
            }
            Self::NonFiniteOutput { pixel, channel } => {
                write!(
                    formatter,
                    "dither produced a non-finite value at {pixel}:{channel}"
                )
            }
        }
    }
}

impl std::error::Error for DitherExecutionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DitherPlan {
    config: DitherConfig,
    context: DitherRenderContext,
    dimensions: RasterDimensions,
}

impl DitherPlan {
    #[must_use]
    pub const fn new(config: DitherConfig, dimensions: RasterDimensions) -> Self {
        Self {
            config,
            context: DitherRenderContext::defaults(),
            dimensions,
        }
    }

    #[must_use]
    pub const fn with_context(
        config: DitherConfig,
        dimensions: RasterDimensions,
        context: DitherRenderContext,
    ) -> Self {
        Self {
            config,
            context,
            dimensions,
        }
    }

    pub fn execute(&self, pixels: &[LinearRgb]) -> Result<Vec<LinearRgb>, DitherExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count()).unwrap_or(usize::MAX);
        if pixels.len() != expected {
            return Err(DitherExecutionError::DimensionsMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        let values = pixels
            .iter()
            .map(|pixel| {
                [
                    clip_nan(pixel.red().get()),
                    clip_nan(pixel.green().get()),
                    clip_nan(pixel.blue().get()),
                ]
            })
            .collect::<Vec<_>>();
        let values = self.execute_values(values);
        values
            .into_iter()
            .enumerate()
            .map(|(pixel, value)| {
                let channels = value
                    .into_iter()
                    .enumerate()
                    .map(|(channel, value)| {
                        FiniteF32::new(value)
                            .map_err(|_| DitherExecutionError::NonFiniteOutput { pixel, channel })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(LinearRgb::new(channels[0], channels[1], channels[2]))
            })
            .collect()
    }

    fn execute_values(&self, values: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
        match self.config.method {
            DitherMethod::Random => self.random(values),
            DitherMethod::Posterize(levels) => posterize(values, u32::from(levels)),
            method => {
                let Some(levels) = self.levels(method) else {
                    return values;
                };
                floyd_steinberg(
                    values,
                    self.dimensions,
                    levels,
                    method.is_gray()
                        || (matches!(method, DitherMethod::FsAuto) && self.context.gray),
                )
            }
        }
    }

    fn levels(&self, method: DitherMethod) -> Option<u32> {
        let scale = self.context.scale.get();
        let l1 = (1.0 + (1.0 / scale).log2()).floor() as i32;
        let bds = if self.context.export {
            1
        } else {
            l1.saturating_mul(l1)
        };
        let bounded = |value: i32, maximum: i32| value.clamp(2, maximum) as u32;
        let levels = match method {
            DitherMethod::Fs1BitGray => bounded(bds + 1, 256),
            DitherMethod::Fs1BitRgb => bounded(bds + 1, 4),
            DitherMethod::Fs2BitGray | DitherMethod::Fs2BitRgb => 4,
            DitherMethod::Fs4BitGray => bounded(15 * bds + 1, 256),
            DitherMethod::Fs4BitRgb => 16,
            DitherMethod::Fs6BitGray => bounded(63 * bds + 1, 256),
            DitherMethod::Fs8BitRgb => 256,
            DitherMethod::Fs16BitRgb => 65_536,
            DitherMethod::FsAuto => {
                if self.context.preview {
                    return None;
                }
                match self.context.bit_depth {
                    DitherBitDepth::BlackWhite => 2,
                    DitherBitDepth::Int8 => 256,
                    DitherBitDepth::Int10 => 1024,
                    DitherBitDepth::Int12 => 4096,
                    DitherBitDepth::Int16 => 65_536,
                    DitherBitDepth::Float => return None,
                }
            }
            DitherMethod::Random | DitherMethod::Posterize(_) => return None,
        };
        Some(levels)
    }

    fn random(&self, values: Vec<[f32; 3]>) -> Vec<[f32; 3]> {
        let amplitude = 2.0_f32.powf(self.config.damping.get() / 10.0);
        values
            .into_iter()
            .enumerate()
            .map(|(pixel, value)| {
                let pixel = u64::try_from(pixel).expect("pixel index fits random key");
                let method = u64::from(self.config.method.id());
                let damping = u64::from(self.config.damping.get().to_bits());
                let mut output = [0.0; 3];
                for (channel, value) in value.into_iter().enumerate() {
                    let channel = u64::try_from(channel).expect("channel index fits");
                    let random = tpdf(tea_word(
                        self.config.seed ^ method ^ damping,
                        pixel.saturating_mul(4) | channel,
                    ));
                    output[usize::try_from(channel).expect("channel index fits usize")] =
                        (value + amplitude * random).clamp(0.0, 1.0);
                }
                output
            })
            .collect()
    }
}

fn clip_nan(value: f32) -> f32 {
    if value >= 0.0 {
        if value < 1.0 { value } else { 1.0 }
    } else if value.is_nan() {
        0.5
    } else {
        0.0
    }
}

#[must_use]
pub fn quantize_round_half_strictly_greater(value: f32, levels: u32) -> f32 {
    let value = clip_nan(value);
    let factor = levels.saturating_sub(1) as f32;
    let reciprocal = 1.0 / factor;
    reciprocal * (value * factor - 0.5).ceil()
}

#[must_use]
pub fn rgb_to_gray(pixel: [f32; 3]) -> f32 {
    0.30 * pixel[0] + 0.59 * pixel[1] + 0.11 * pixel[2]
}

fn posterize(values: Vec<[f32; 3]>, levels: u32) -> Vec<[f32; 3]> {
    values
        .into_iter()
        .map(|pixel| pixel.map(|value| quantize_round_half_strictly_greater(value, levels)))
        .collect()
}

fn floyd_steinberg(
    mut values: Vec<[f32; 3]>,
    dimensions: RasterDimensions,
    levels: u32,
    gray: bool,
) -> Vec<[f32; 3]> {
    let width = dimensions.width() as usize;
    let height = dimensions.height() as usize;
    if width < 3 || height < 3 {
        for pixel in &mut values {
            quantize_pixel(pixel, levels, gray);
        }
        return values;
    }
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let error = quantize_pixel(&mut values[index], levels, gray);
            if x + 1 < width {
                add_error(&mut values[index + 1], error, 7.0 / 16.0);
            }
            if y + 1 < height {
                if x > 0 {
                    add_error(&mut values[index + width - 1], error, 3.0 / 16.0);
                }
                add_error(&mut values[index + width], error, 5.0 / 16.0);
                if x + 1 < width {
                    add_error(&mut values[index + width + 1], error, 1.0 / 16.0);
                }
            }
        }
    }
    values
}

fn quantize_pixel(pixel: &mut [f32; 3], levels: u32, gray: bool) -> [f32; 3] {
    let old = *pixel;
    if gray {
        let quantized = quantize_round_half_strictly_greater(rgb_to_gray(old), levels);
        *pixel = [quantized; 3];
        [old[0] - quantized, old[1] - quantized, old[2] - quantized]
    } else {
        *pixel = old.map(|value| quantize_round_half_strictly_greater(value, levels));
        [old[0] - pixel[0], old[1] - pixel[1], old[2] - pixel[2]]
    }
}

fn add_error(pixel: &mut [f32; 3], error: [f32; 3], weight: f32) {
    for (value, error) in pixel.iter_mut().zip(error) {
        *value = clip_nan(*value + error * weight);
    }
}

fn tea_word(seed: u64, pixel: u64) -> u32 {
    let mut v0 = (seed as u32) ^ pixel as u32;
    let mut v1 = (seed >> 32) as u32 ^ (pixel >> 32) as u32;
    let key = [0xa341_316c_u32, 0xc801_3ea4, 0xad90_777d, 0x7e95_761d];
    let mut sum = 0_u32;
    for _ in 0..8 {
        sum = sum.wrapping_add(0x9e37_79b9);
        v0 = v0.wrapping_add(
            ((v1 << 4).wrapping_add(key[0]))
                ^ v1.wrapping_add(sum)
                ^ ((v1 >> 5).wrapping_add(key[1])),
        );
        v1 = v1.wrapping_add(
            ((v0 << 4).wrapping_add(key[2]))
                ^ v0.wrapping_add(sum)
                ^ ((v0 >> 5).wrapping_add(key[3])),
        );
    }
    v0
}

fn tpdf(random: u32) -> f32 {
    let fraction = random as f32 / u32::MAX as f32;
    if fraction < 0.5 {
        (2.0 * fraction).sqrt() - 1.0
    } else {
        1.0 - (2.0 * (1.0 - fraction)).sqrt()
    }
}

#[must_use]
pub fn dither_descriptor() -> OperationDescriptor {
    let scalar = |id: &str, minimum: f64, maximum: f64, default: f64| ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: 1,
        removed_version: None,
        unit: None,
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    };
    OperationDescriptor {
        id: DescriptorId::new(DITHER_COMPATIBILITY_ID, "rusttable.dither", 2, 2, 1)
            .expect("static ID"),
        parameters: vec![
            scalar("method", 0.0, 263.0, 5.0),
            scalar("palette", -2_147_483_648.0, 2_147_483_647.0, 0.0),
            scalar("radius", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range0", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range1", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range2", -f64::from(f32::MAX), f64::from(f32::MAX), 1.0),
            scalar("range3", -f64::from(f32::MAX), f64::from(f32::MAX), 1.0),
            scalar("damping", -200.0, 0.0, -200.0),
        ],
        flags: OperationFlags::FULL_IMAGE
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "scene-linear".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: 1,
            minimum_tile_edge: 1,
            preferred_tile_edge: 1024,
            temporary_multiplier_milli: 4000,
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
            modes: vec![
                "random".to_owned(),
                "posterize".to_owned(),
                "floyd-steinberg".to_owned(),
            ],
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
            source_versions: vec![1, 2],
            target_version: 2,
            opaque_unknown_allowed: true,
        },
        ui: Some(UiHint {
            label_key: "operation.dither".to_owned(),
            group_key: "group.correct".to_owned(),
            control: "dither".to_owned(),
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
