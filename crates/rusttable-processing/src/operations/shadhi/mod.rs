//! Legacy Darktable shadows/highlights compatibility at the typed RGB boundary.
//!
//! Darktable's `src/iop/shadhi.c` is a Lab operation and its current default
//! bilateral path is not available in `RustTable`.  The safe implementation
//! therefore executes only the deterministic Gaussian variant on linear RGB,
//! using shared convolution and scalar plan infrastructure.  Lab, CFA, and
//! WGPU compatibility are deliberately not advertised.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;

use crate::operations::common::{
    OperationExecutionError, ReconstructionBudget, from_luma_chroma, luma, validate_shape,
};
use crate::operations::convolution::GaussianKernel;
use crate::{FiniteF32, LinearRgb, RasterDimensions};

pub const SHADHI_COMPATIBILITY_ID: &str = "shadhi";
pub const SHADHI_SCHEMA_VERSION: u16 = 5;

pub const SHADHI_V1_PARAMETER_BYTES: usize = 28;
pub const SHADHI_V2_PARAMETER_BYTES: usize = 36;
pub const SHADHI_V3_PARAMETER_BYTES: usize = 40;
pub const SHADHI_V4_PARAMETER_BYTES: usize = 44;
pub const SHADHI_V5_PARAMETER_BYTES: usize = 48;

pub const SHADHI_DEFAULT_ORDER: u32 = 0;
pub const SHADHI_DEFAULT_RADIUS: f32 = 100.0;
pub const SHADHI_DEFAULT_SHADOWS: f32 = 50.0;
pub const SHADHI_DEFAULT_WHITEPOINT: f32 = 0.0;
pub const SHADHI_DEFAULT_HIGHLIGHTS: f32 = -50.0;
pub const SHADHI_DEFAULT_COMPRESS: f32 = 50.0;
pub const SHADHI_DEFAULT_SHADOWS_COLOR: f32 = 100.0;
pub const SHADHI_DEFAULT_HIGHLIGHTS_COLOR: f32 = 50.0;
pub const SHADHI_DEFAULT_FLAGS: u32 = 127;
pub const SHADHI_DEFAULT_LOW_APPROXIMATION: f32 = 0.000_001;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadhiAlgorithm {
    Gaussian,
    Bilateral,
}

impl ShadhiAlgorithm {
    #[must_use]
    pub const fn id(self) -> u32 {
        match self {
            Self::Gaussian => 0,
            Self::Bilateral => 1,
        }
    }

    pub fn from_id(id: u32) -> Result<Self, ShadhiParameterError> {
        match id {
            0 => Ok(Self::Gaussian),
            1 => Ok(Self::Bilateral),
            _ => Err(ShadhiParameterError::UnknownAlgorithm(id)),
        }
    }
}

/// Darktable v1 payload, retained as a typed legacy layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiParametersV1 {
    pub order: u32,
    pub radius: f32,
    pub shadows: f32,
    pub reserved1: f32,
    pub highlights: f32,
    pub reserved2: f32,
    pub compress: f32,
}

/// Darktable v2 payload, retained as a typed legacy layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiParametersV2 {
    pub order: u32,
    pub radius: f32,
    pub shadows: f32,
    pub reserved1: f32,
    pub highlights: f32,
    pub reserved2: f32,
    pub compress: f32,
    pub shadows_ccorrect: f32,
    pub highlights_ccorrect: f32,
}

/// Darktable v3 payload, retained as a typed legacy layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiParametersV3 {
    pub order: u32,
    pub radius: f32,
    pub shadows: f32,
    pub reserved1: f32,
    pub highlights: f32,
    pub reserved2: f32,
    pub compress: f32,
    pub shadows_ccorrect: f32,
    pub highlights_ccorrect: f32,
    pub flags: u32,
}

/// Darktable v4 payload, retained as a typed legacy layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiParametersV4 {
    pub order: u32,
    pub radius: f32,
    pub shadows: f32,
    pub whitepoint: f32,
    pub highlights: f32,
    pub reserved2: f32,
    pub compress: f32,
    pub shadows_ccorrect: f32,
    pub highlights_ccorrect: f32,
    pub flags: u32,
    pub low_approximation: f32,
}

/// Current Darktable v5 payload, with the exact 48-byte scalar layout.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShadhiParametersV5 {
    pub order: u32,
    pub radius: f32,
    pub shadows: f32,
    pub whitepoint: f32,
    pub highlights: f32,
    pub reserved2: f32,
    pub compress: f32,
    pub shadows_ccorrect: f32,
    pub highlights_ccorrect: f32,
    pub flags: u32,
    pub low_approximation: f32,
    pub shadhi_algo: u32,
}

impl ShadhiParametersV5 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            order: SHADHI_DEFAULT_ORDER,
            radius: SHADHI_DEFAULT_RADIUS,
            shadows: SHADHI_DEFAULT_SHADOWS,
            whitepoint: SHADHI_DEFAULT_WHITEPOINT,
            highlights: SHADHI_DEFAULT_HIGHLIGHTS,
            reserved2: 0.0,
            compress: SHADHI_DEFAULT_COMPRESS,
            shadows_ccorrect: SHADHI_DEFAULT_SHADOWS_COLOR,
            highlights_ccorrect: SHADHI_DEFAULT_HIGHLIGHTS_COLOR,
            flags: SHADHI_DEFAULT_FLAGS,
            low_approximation: SHADHI_DEFAULT_LOW_APPROXIMATION,
            shadhi_algo: ShadhiAlgorithm::Bilateral.id(),
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SHADHI_V5_PARAMETER_BYTES] {
        encode_v5(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ShadhiCodecError> {
        if bytes.len() != SHADHI_V5_PARAMETER_BYTES {
            return Err(ShadhiCodecError::InvalidLength {
                expected: SHADHI_V5_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = Reader::new(bytes);
        let parameters = Self {
            order: read.u32(0),
            radius: read.f32(4),
            shadows: read.f32(8),
            whitepoint: read.f32(12),
            highlights: read.f32(16),
            reserved2: read.f32(20),
            compress: read.f32(24),
            shadows_ccorrect: read.f32(28),
            highlights_ccorrect: read.f32(32),
            flags: read.u32(36),
            low_approximation: read.f32(40),
            shadhi_algo: read.u32(44),
        };
        ShadhiConfig::try_from(parameters).map_err(ShadhiCodecError::Parameters)?;
        Ok(parameters)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShadhiHistory {
    V1(ShadhiParametersV1),
    V2(ShadhiParametersV2),
    V3(ShadhiParametersV3),
    V4(ShadhiParametersV4),
    V5(ShadhiParametersV5),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ShadhiHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ShadhiCodecError> {
        match version {
            1 => Ok(Self::V1(decode_v1(bytes)?)),
            2 => Ok(Self::V2(decode_v2(bytes)?)),
            3 => Ok(Self::V3(decode_v3(bytes)?)),
            4 => Ok(Self::V4(decode_v4(bytes)?)),
            5 => Ok(Self::V5(ShadhiParametersV5::from_bytes(bytes)?)),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(value) => encode_v1(*value).to_vec(),
            Self::V2(value) => encode_v2(*value).to_vec(),
            Self::V3(value) => encode_v3(*value).to_vec(),
            Self::V4(value) => encode_v4(*value).to_vec(),
            Self::V5(value) => value.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => 3,
            Self::V4(_) => 4,
            Self::V5(_) => 5,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadhiCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(ShadhiParameterError),
}

impl fmt::Display for ShadhiCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "shadhi payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid shadhi parameters: {error}"),
        }
    }
}

impl std::error::Error for ShadhiCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ShadhiConfig {
    order: u32,
    radius: FiniteF32,
    shadows: FiniteF32,
    whitepoint: FiniteF32,
    highlights: FiniteF32,
    reserved2: FiniteF32,
    compress: FiniteF32,
    shadows_ccorrect: FiniteF32,
    highlights_ccorrect: FiniteF32,
    flags: u32,
    low_approximation: FiniteF32,
    shadhi_algo: ShadhiAlgorithm,
}

impl ShadhiConfig {
    pub fn new(parameters: ShadhiParametersV5) -> Result<Self, ShadhiParameterError> {
        Self::try_from(parameters)
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(ShadhiParametersV5::defaults()).expect("shadhi defaults are valid")
    }

    #[must_use]
    pub const fn order(self) -> u32 {
        self.order
    }
    #[must_use]
    pub const fn radius(self) -> f32 {
        self.radius.get()
    }
    #[must_use]
    pub const fn shadows(self) -> f32 {
        self.shadows.get()
    }
    #[must_use]
    pub const fn whitepoint(self) -> f32 {
        self.whitepoint.get()
    }
    #[must_use]
    pub const fn highlights(self) -> f32 {
        self.highlights.get()
    }
    #[must_use]
    pub const fn reserved2(self) -> f32 {
        self.reserved2.get()
    }
    #[must_use]
    pub const fn compress(self) -> f32 {
        self.compress.get()
    }
    #[must_use]
    pub const fn shadows_ccorrect(self) -> f32 {
        self.shadows_ccorrect.get()
    }
    #[must_use]
    pub const fn highlights_ccorrect(self) -> f32 {
        self.highlights_ccorrect.get()
    }
    #[must_use]
    pub const fn flags(self) -> u32 {
        self.flags
    }
    #[must_use]
    pub const fn low_approximation(self) -> f32 {
        self.low_approximation.get()
    }
    #[must_use]
    pub const fn shadhi_algo(self) -> ShadhiAlgorithm {
        self.shadhi_algo
    }
}

impl TryFrom<ShadhiParametersV5> for ShadhiConfig {
    type Error = ShadhiParameterError;

    fn try_from(parameters: ShadhiParametersV5) -> Result<Self, Self::Error> {
        if parameters.order > 3 {
            return Err(ShadhiParameterError::InvalidOrder(parameters.order));
        }
        Ok(Self {
            order: parameters.order,
            radius: bounded("radius", parameters.radius, 0.1, 500.0)?,
            shadows: bounded("shadows", parameters.shadows, -100.0, 100.0)?,
            whitepoint: bounded("whitepoint", parameters.whitepoint, -10.0, 10.0)?,
            highlights: bounded("highlights", parameters.highlights, -100.0, 100.0)?,
            reserved2: finite("reserved2", parameters.reserved2)?,
            compress: bounded("compress", parameters.compress, 0.0, 100.0)?,
            shadows_ccorrect: bounded("shadows_ccorrect", parameters.shadows_ccorrect, 0.0, 100.0)?,
            highlights_ccorrect: bounded(
                "highlights_ccorrect",
                parameters.highlights_ccorrect,
                0.0,
                100.0,
            )?,
            flags: parameters.flags,
            low_approximation: bounded(
                "low_approximation",
                parameters.low_approximation,
                0.000_000_001,
                1.0,
            )?,
            shadhi_algo: ShadhiAlgorithm::from_id(parameters.shadhi_algo)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadhiParameterError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
    InvalidOrder(u32),
    UnknownAlgorithm(u32),
}

impl fmt::Display for ShadhiParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "shadhi {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "shadhi {name} is outside its range"),
            Self::InvalidOrder(value) => write!(formatter, "shadhi order {value} is unsupported"),
            Self::UnknownAlgorithm(value) => write!(formatter, "unknown shadhi algorithm {value}"),
        }
    }
}

impl std::error::Error for ShadhiParameterError {}

fn finite(name: &'static str, value: f32) -> Result<FiniteF32, ShadhiParameterError> {
    FiniteF32::new(value).map_err(|_| ShadhiParameterError::NonFinite(name))
}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, ShadhiParameterError> {
    let value = finite(name, value)?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(ShadhiParameterError::OutOfRange(name));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShadhiPlan {
    config: ShadhiConfig,
    dimensions: RasterDimensions,
    kernel: GaussianKernel,
}

impl ShadhiPlan {
    pub fn new(
        config: ShadhiConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        if config.shadhi_algo() == ShadhiAlgorithm::Bilateral {
            return Err(OperationExecutionError::UnsupportedCapability(
                "shadhi bilateral mode requires the unavailable Lab/bilateral boundary",
            ));
        }
        let diagonal = (dimensions.width() as f32).hypot(dimensions.height() as f32);
        let radius = (diagonal * config.radius() * 0.01).ceil().clamp(1.0, 256.0) as u32;
        let kernel = GaussianKernel::from_box_radius(radius).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            }
        })?;
        Ok(Self {
            config,
            dimensions,
            kernel,
        })
    }

    #[must_use]
    pub const fn radius_pixels(&self) -> u32 {
        self.kernel.support()
    }

    /// Executes a deterministic Gaussian RGB approximation of Darktable's
    /// Lab shadows/highlights overlay. Alpha is outside this three-channel
    /// image contract and is preserved by the pixelpipe bridge.
    pub fn execute(&self, input: &[LinearRgb]) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        if self.config.shadows().to_bits() == 0.0f32.to_bits()
            && self.config.highlights().to_bits() == 0.0f32.to_bits()
        {
            return Ok(input.to_vec());
        }
        let blurred =
            self.kernel
                .apply_rgb(input, self.dimensions, ReconstructionBudget::default())?;
        let whitepoint = (1.0 - self.config.whitepoint() / 100.0).max(0.01);
        let compress = (self.config.compress() / 100.0).clamp(0.0, 0.99);
        let shadows = (self.config.shadows() / 100.0).clamp(-1.0, 1.0) * 2.0;
        let highlights = (self.config.highlights() / 100.0).clamp(-1.0, 1.0) * 2.0;
        let shadow_cc =
            ((self.config.shadows_ccorrect() / 100.0 - 0.5) * sign(shadows) + 0.5).clamp(0.0, 1.0);
        let highlight_cc = ((self.config.highlights_ccorrect() / 100.0 - 0.5) * sign(-highlights)
            + 0.5)
            .clamp(0.0, 1.0);
        input
            .iter()
            .zip(blurred)
            .enumerate()
            .map(|(index, (source, base))| {
                let source_luma = (luma(*source) / whitepoint).clamp(0.0, 1.0);
                let base_luma = (luma(base) / whitepoint).clamp(0.0, 1.0);
                let shadow_mask = ((1.0 - base_luma - compress) / (1.0 - compress)).clamp(0.0, 1.0);
                let highlight_mask = ((base_luma - compress) / (1.0 - compress)).clamp(0.0, 1.0);
                let mut target = source_luma;
                target = overlay(target, base_luma, shadows, shadow_mask);
                target = overlay(target, base_luma, highlights, highlight_mask);
                target = target.clamp(0.0, 1.0) * whitepoint;
                let source_chroma = (
                    source.red().get() - luma(*source),
                    source.blue().get() - luma(*source),
                );
                let base_chroma = (
                    base.red().get() - luma(base),
                    base.blue().get() - luma(base),
                );
                let chroma = (
                    source_chroma.0 * (1.0 - shadow_cc) + base_chroma.0 * shadow_cc,
                    source_chroma.1 * (1.0 - highlight_cc) + base_chroma.1 * highlight_cc,
                );
                from_luma_chroma(target, chroma).ok_or(OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Red,
                })
            })
            .collect()
    }
}

fn sign(value: f32) -> f32 {
    if value < 0.0 { -1.0 } else { 1.0 }
}

fn overlay(source: f32, base: f32, amount: f32, mask: f32) -> f32 {
    let mut remaining = amount.abs();
    let direction = sign(amount);
    let mut value = source;
    while remaining > 0.0 {
        let chunk = remaining.min(1.0);
        let reflected = (base - 0.5) * direction + 0.5;
        let overlay = if value > 0.5 {
            1.0 - (1.0 - 2.0 * (value - 0.5)) * (1.0 - reflected)
        } else {
            2.0 * value * reflected
        };
        let opacity = chunk * mask;
        value += (overlay - value) * opacity;
        remaining -= chunk;
    }
    value
}

#[derive(Debug, Clone, Copy)]
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    const fn new(bytes: &[u8]) -> Reader<'_> {
        Reader(bytes)
    }

    fn f32(self, start: usize) -> f32 {
        f32::from_le_bytes(
            self.0[start..start + 4]
                .try_into()
                .expect("validated parameter range"),
        )
    }

    fn u32(self, start: usize) -> u32 {
        u32::from_le_bytes(
            self.0[start..start + 4]
                .try_into()
                .expect("validated parameter range"),
        )
    }
}

fn encode_v1(value: ShadhiParametersV1) -> [u8; SHADHI_V1_PARAMETER_BYTES] {
    let mut bytes = [0; SHADHI_V1_PARAMETER_BYTES];
    put_u32(&mut bytes, 0, value.order);
    for (index, item) in [
        value.radius,
        value.shadows,
        value.reserved1,
        value.highlights,
        value.reserved2,
        value.compress,
    ]
    .into_iter()
    .enumerate()
    {
        put_f32(&mut bytes, 4 + index * 4, item);
    }
    bytes
}

fn encode_v2(value: ShadhiParametersV2) -> [u8; SHADHI_V2_PARAMETER_BYTES] {
    let mut bytes = [0; SHADHI_V2_PARAMETER_BYTES];
    put_u32(&mut bytes, 0, value.order);
    for (index, item) in [
        value.radius,
        value.shadows,
        value.reserved1,
        value.highlights,
        value.reserved2,
        value.compress,
        value.shadows_ccorrect,
        value.highlights_ccorrect,
    ]
    .into_iter()
    .enumerate()
    {
        put_f32(&mut bytes, 4 + index * 4, item);
    }
    bytes
}

fn encode_v3(value: ShadhiParametersV3) -> [u8; SHADHI_V3_PARAMETER_BYTES] {
    let mut bytes = [0; SHADHI_V3_PARAMETER_BYTES];
    put_u32(&mut bytes, 0, value.order);
    for (index, item) in [
        value.radius,
        value.shadows,
        value.reserved1,
        value.highlights,
        value.reserved2,
        value.compress,
        value.shadows_ccorrect,
        value.highlights_ccorrect,
    ]
    .into_iter()
    .enumerate()
    {
        put_f32(&mut bytes, 4 + index * 4, item);
    }
    put_u32(&mut bytes, 36, value.flags);
    bytes
}

fn encode_v4(value: ShadhiParametersV4) -> [u8; SHADHI_V4_PARAMETER_BYTES] {
    let mut bytes = [0; SHADHI_V4_PARAMETER_BYTES];
    put_u32(&mut bytes, 0, value.order);
    for (index, item) in [
        value.radius,
        value.shadows,
        value.whitepoint,
        value.highlights,
        value.reserved2,
        value.compress,
        value.shadows_ccorrect,
        value.highlights_ccorrect,
    ]
    .into_iter()
    .enumerate()
    {
        put_f32(&mut bytes, 4 + index * 4, item);
    }
    put_u32(&mut bytes, 36, value.flags);
    put_f32(&mut bytes, 40, value.low_approximation);
    bytes
}

fn encode_v5(value: ShadhiParametersV5) -> [u8; SHADHI_V5_PARAMETER_BYTES] {
    let mut bytes = [0; SHADHI_V5_PARAMETER_BYTES];
    put_u32(&mut bytes, 0, value.order);
    for (index, item) in [
        value.radius,
        value.shadows,
        value.whitepoint,
        value.highlights,
        value.reserved2,
        value.compress,
        value.shadows_ccorrect,
        value.highlights_ccorrect,
    ]
    .into_iter()
    .enumerate()
    {
        put_f32(&mut bytes, 4 + index * 4, item);
    }
    put_u32(&mut bytes, 36, value.flags);
    put_f32(&mut bytes, 40, value.low_approximation);
    put_u32(&mut bytes, 44, value.shadhi_algo);
    bytes
}

fn put_f32(bytes: &mut [u8], start: usize, value: f32) {
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut [u8], start: usize, value: u32) {
    bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
}

fn decode_v1(bytes: &[u8]) -> Result<ShadhiParametersV1, ShadhiCodecError> {
    if bytes.len() != SHADHI_V1_PARAMETER_BYTES {
        return Err(length_error(SHADHI_V1_PARAMETER_BYTES, bytes.len()));
    }
    let read = Reader::new(bytes);
    Ok(ShadhiParametersV1 {
        order: read.u32(0),
        radius: read.f32(4),
        shadows: read.f32(8),
        reserved1: read.f32(12),
        highlights: read.f32(16),
        reserved2: read.f32(20),
        compress: read.f32(24),
    })
}

fn decode_v2(bytes: &[u8]) -> Result<ShadhiParametersV2, ShadhiCodecError> {
    if bytes.len() != SHADHI_V2_PARAMETER_BYTES {
        return Err(length_error(SHADHI_V2_PARAMETER_BYTES, bytes.len()));
    }
    let read = Reader::new(bytes);
    Ok(ShadhiParametersV2 {
        order: read.u32(0),
        radius: read.f32(4),
        shadows: read.f32(8),
        reserved1: read.f32(12),
        highlights: read.f32(16),
        reserved2: read.f32(20),
        compress: read.f32(24),
        shadows_ccorrect: read.f32(28),
        highlights_ccorrect: read.f32(32),
    })
}

fn decode_v3(bytes: &[u8]) -> Result<ShadhiParametersV3, ShadhiCodecError> {
    if bytes.len() != SHADHI_V3_PARAMETER_BYTES {
        return Err(length_error(SHADHI_V3_PARAMETER_BYTES, bytes.len()));
    }
    let read = Reader::new(bytes);
    Ok(ShadhiParametersV3 {
        order: read.u32(0),
        radius: read.f32(4),
        shadows: read.f32(8),
        reserved1: read.f32(12),
        highlights: read.f32(16),
        reserved2: read.f32(20),
        compress: read.f32(24),
        shadows_ccorrect: read.f32(28),
        highlights_ccorrect: read.f32(32),
        flags: read.u32(36),
    })
}

fn decode_v4(bytes: &[u8]) -> Result<ShadhiParametersV4, ShadhiCodecError> {
    if bytes.len() != SHADHI_V4_PARAMETER_BYTES {
        return Err(length_error(SHADHI_V4_PARAMETER_BYTES, bytes.len()));
    }
    let read = Reader::new(bytes);
    Ok(ShadhiParametersV4 {
        order: read.u32(0),
        radius: read.f32(4),
        shadows: read.f32(8),
        whitepoint: read.f32(12),
        highlights: read.f32(16),
        reserved2: read.f32(20),
        compress: read.f32(24),
        shadows_ccorrect: read.f32(28),
        highlights_ccorrect: read.f32(32),
        flags: read.u32(36),
        low_approximation: read.f32(40),
    })
}

fn length_error(expected: usize, actual: usize) -> ShadhiCodecError {
    ShadhiCodecError::InvalidLength { expected, actual }
}

#[must_use]
pub fn migrate_v1_to_v5(value: ShadhiParametersV1) -> ShadhiParametersV5 {
    ShadhiParametersV5 {
        order: value.order,
        radius: value.radius.abs(),
        shadows: value.shadows * 0.5,
        whitepoint: value.reserved1,
        highlights: value.highlights * -0.5,
        reserved2: value.reserved2,
        compress: value.compress,
        shadows_ccorrect: 100.0,
        highlights_ccorrect: 0.0,
        flags: 0,
        low_approximation: 0.01,
        shadhi_algo: u32::from(value.radius < 0.0),
    }
}

#[must_use]
pub fn migrate_v2_to_v5(value: ShadhiParametersV2) -> ShadhiParametersV5 {
    ShadhiParametersV5 {
        order: value.order,
        radius: value.radius.abs(),
        shadows: value.shadows,
        whitepoint: value.reserved1,
        highlights: value.highlights,
        reserved2: value.reserved2,
        compress: value.compress,
        shadows_ccorrect: value.shadows_ccorrect,
        highlights_ccorrect: value.highlights_ccorrect,
        flags: 0,
        low_approximation: 0.01,
        shadhi_algo: u32::from(value.radius < 0.0),
    }
}

#[must_use]
pub fn migrate_v3_to_v5(value: ShadhiParametersV3) -> ShadhiParametersV5 {
    ShadhiParametersV5 {
        order: value.order,
        radius: value.radius.abs(),
        shadows: value.shadows,
        whitepoint: value.reserved1,
        highlights: value.highlights,
        reserved2: value.reserved2,
        compress: value.compress,
        shadows_ccorrect: value.shadows_ccorrect,
        highlights_ccorrect: value.highlights_ccorrect,
        flags: value.flags,
        low_approximation: 0.01,
        shadhi_algo: u32::from(value.radius < 0.0),
    }
}

#[must_use]
pub fn migrate_v4_to_v5(value: ShadhiParametersV4) -> ShadhiParametersV5 {
    ShadhiParametersV5 {
        order: value.order,
        radius: value.radius.abs(),
        shadows: value.shadows,
        whitepoint: value.whitepoint,
        highlights: value.highlights,
        reserved2: value.reserved2,
        compress: value.compress,
        shadows_ccorrect: value.shadows_ccorrect,
        highlights_ccorrect: value.highlights_ccorrect,
        flags: value.flags,
        low_approximation: value.low_approximation,
        shadhi_algo: u32::from(value.radius < 0.0),
    }
}

mod descriptor;
pub use descriptor::shadhi_descriptor;
