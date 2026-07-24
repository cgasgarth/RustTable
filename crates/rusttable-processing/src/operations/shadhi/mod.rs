//! Legacy Darktable shadows/highlights compatibility at the typed Lab boundary.
//!
//! Darktable's `src/iop/shadhi.c` consumes four-channel D50 Lab pixels.  The
//! plan below keeps that contract explicit: RGB callers cross the shared Lab
//! boundary in the evaluator, while direct callers can exercise the exact Lab
//! equations and the full-image filter plan.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]
#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use std::fmt;

use crate::FiniteF32;

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

const UNBOUND_L: u32 = 1;
const UNBOUND_A: u32 = 2;
const UNBOUND_B: u32 = 4;
const UNBOUND_SHADOWS_L: u32 = UNBOUND_L;
const UNBOUND_SHADOWS_A: u32 = UNBOUND_A;
const UNBOUND_SHADOWS_B: u32 = UNBOUND_B;
const UNBOUND_HIGHLIGHTS_L: u32 = UNBOUND_L << 3;
const UNBOUND_HIGHLIGHTS_A: u32 = UNBOUND_A << 3;
const UNBOUND_HIGHLIGHTS_B: u32 = UNBOUND_B << 3;
const UNBOUND_GAUSSIAN: u32 = 64;
const UNBOUND_BILATERAL: u32 = 128;
const LAB_MINIMUM: [f32; 4] = [0.0, -128.0, -128.0, 0.0];
const LAB_MAXIMUM: [f32; 4] = [100.0, 128.0, 128.0, 1.0];

mod execution;
pub use execution::{ShadhiBilateralRequest, ShadhiPixel, ShadhiPlan, ShadhiReceipt};

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
        if parameters.order > 2 {
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
