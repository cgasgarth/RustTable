#![expect(
    clippy::suboptimal_flops,
    reason = "Native Dither arithmetic order is preserved for IEEE-754 parity."
)]

//! Bounded CPU leaf for `src/iop/dither.c` from the pinned Darktable baseline.
//!
//! This module owns the native history structs, explicitly separate qualified
//! repository-envelope helpers, and scalar CPU arithmetic. Shared history
//! materialization, blending/masks, imageio pipe qualification, GPU policy, and
//! the source-shaped GTK editor remain integration responsibilities.

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
pub const DITHER_RUST_ID: &str = "rusttable.dither";
pub const DITHER_SCHEMA_VERSION: u16 = 2;
/// Bytes occupied by the source-declared enum, integer, and six `float` fields.
pub const DITHER_NATIVE_FIELD_BYTES: usize = 32;
/// Public native v1 history payload size. Native v1 and v2 have the same layout.
pub const DITHER_V1_PARAMETER_BYTES: usize = DITHER_NATIVE_FIELD_BYTES;
/// Public native v2 history payload size. Native v1 and v2 have the same layout.
pub const DITHER_V2_PARAMETER_BYTES: usize = DITHER_NATIVE_FIELD_BYTES;
/// Size of the legacy qualified repository slot, kept separate from the native ABI.
pub const DITHER_V1_ENVELOPE_BYTES: usize = 168;
/// Size of the current qualified repository slot, kept separate from the native ABI.
pub const DITHER_V2_ENVELOPE_BYTES: usize = 36;
pub const DITHER_DEFAULT_DAMPING: f32 = -100.0;
pub const DITHER_PRESET_DAMPING: f32 = -200.0;
pub const DITHER_DEFAULT_ENABLED: bool = false;
pub const DITHER_LEGACY_ORDER: f32 = 67.5;
pub const DITHER_V30_ORDER: i32 = 75;
pub const DITHER_V50_ORDER: i32 = 75;
/// Generated operation-inventory ordinal; distinct from native iop-order values.
pub const DITHER_GENERATED_INVENTORY_ORDER: u32 = 90;
pub const DITHER_CANCELLATION_INTERVAL: usize = 1_024;

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
            Self::Posterize(levels) => 0x0ff + levels as u32,
        }
    }

    pub const fn from_id(id: u32) -> Result<Self, DitherMethodError> {
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
            0x101..=0x107 => Ok(Self::Posterize((id - 0x0ff) as u8)),
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherParametersV1 {
    pub method_id: u32,
    pub palette: i32,
    pub radius: f32,
    pub range: [f32; 4],
    pub damping: f32,
}

impl DitherParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            method_id: DitherMethod::FsAuto.id(),
            palette: 0,
            radius: 0.0,
            range: [0.0, 0.0, 1.0, 1.0],
            damping: DITHER_DEFAULT_DAMPING,
        }
    }

    #[must_use]
    pub fn to_native_bytes(&self) -> [u8; DITHER_NATIVE_FIELD_BYTES] {
        let mut bytes = [0; DITHER_NATIVE_FIELD_BYTES];
        encode_fields(
            &mut bytes,
            self.method_id,
            self.palette,
            self.radius,
            self.range,
            self.damping,
        );
        bytes
    }

    /// Decodes the exact source-declared 32-byte v1 struct.
    #[must_use]
    pub fn from_native_bytes(bytes: &[u8; DITHER_NATIVE_FIELD_BYTES]) -> Self {
        let (method_id, palette, radius, range, damping) = decode_fields(bytes);
        Self {
            method_id,
            palette,
            radius,
            range,
            damping,
        }
    }

    /// Encodes the public native history payload, not a repository envelope.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; DITHER_V1_PARAMETER_BYTES] {
        self.to_native_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DitherCodecError> {
        if bytes.len() != DITHER_V1_PARAMETER_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let native = bytes
            .try_into()
            .expect("native v1 length checked before decoding");
        Ok(Self::from_native_bytes(native))
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DitherParametersV2 {
    pub method_id: u32,
    pub palette: i32,
    pub radius: f32,
    pub range: [f32; 4],
    pub damping: f32,
}

impl DitherParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            method_id: DitherMethod::FsAuto.id(),
            palette: 0,
            radius: 0.0,
            range: [0.0, 0.0, 1.0, 1.0],
            damping: DITHER_DEFAULT_DAMPING,
        }
    }

    /// Source preset registered by `init_presets`; it intentionally differs
    /// from the introspection-derived module default only in damping.
    #[must_use]
    pub const fn native_preset() -> Self {
        let mut parameters = Self::defaults();
        parameters.damping = DITHER_PRESET_DAMPING;
        parameters
    }

    #[must_use]
    pub fn to_native_bytes(&self) -> [u8; DITHER_NATIVE_FIELD_BYTES] {
        let mut bytes = [0; DITHER_NATIVE_FIELD_BYTES];
        encode_fields(
            &mut bytes,
            self.method_id,
            self.palette,
            self.radius,
            self.range,
            self.damping,
        );
        bytes
    }

    /// Decodes the exact source-declared 32-byte v2 struct.
    #[must_use]
    pub fn from_native_bytes(bytes: &[u8; DITHER_NATIVE_FIELD_BYTES]) -> Self {
        let (method_id, palette, radius, range, damping) = decode_fields(bytes);
        Self {
            method_id,
            palette,
            radius,
            range,
            damping,
        }
    }

    /// Encodes the public native history payload, not a repository envelope.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; DITHER_V2_PARAMETER_BYTES] {
        self.to_native_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DitherCodecError> {
        if bytes.len() != DITHER_V2_PARAMETER_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let native = bytes
            .try_into()
            .expect("native v2 length checked before decoding");
        Ok(Self::from_native_bytes(native))
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

/// Qualified repository slots are not the native ABI. These helpers preserve
/// their tails only for callers that explicitly opt into that storage format.
pub mod repository_envelope {
    use super::{
        DITHER_NATIVE_FIELD_BYTES, DITHER_V1_ENVELOPE_BYTES, DITHER_V2_ENVELOPE_BYTES,
        DitherCodecError, DitherParametersV1, DitherParametersV2,
    };

    pub const V1_TAIL_BYTES: usize = DITHER_V1_ENVELOPE_BYTES - DITHER_NATIVE_FIELD_BYTES;
    pub const V2_PADDING_BYTES: usize = DITHER_V2_ENVELOPE_BYTES - DITHER_NATIVE_FIELD_BYTES;

    #[must_use]
    pub fn encode_v1(
        parameters: &DitherParametersV1,
        tail: &[u8; V1_TAIL_BYTES],
    ) -> [u8; DITHER_V1_ENVELOPE_BYTES] {
        let mut bytes = [0; DITHER_V1_ENVELOPE_BYTES];
        bytes[..DITHER_NATIVE_FIELD_BYTES].copy_from_slice(&parameters.to_native_bytes());
        bytes[DITHER_NATIVE_FIELD_BYTES..].copy_from_slice(tail);
        bytes
    }

    pub fn decode_v1(
        bytes: &[u8],
    ) -> Result<(DitherParametersV1, [u8; V1_TAIL_BYTES]), DitherCodecError> {
        if bytes.len() != DITHER_V1_ENVELOPE_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V1_ENVELOPE_BYTES,
                actual: bytes.len(),
            });
        }
        let native: &[u8; DITHER_NATIVE_FIELD_BYTES] = bytes[..DITHER_NATIVE_FIELD_BYTES]
            .try_into()
            .expect("qualified v1 native prefix length");
        let mut tail = [0; V1_TAIL_BYTES];
        tail.copy_from_slice(&bytes[DITHER_NATIVE_FIELD_BYTES..]);
        Ok((DitherParametersV1::from_native_bytes(native), tail))
    }

    #[must_use]
    pub fn encode_v2(
        parameters: &DitherParametersV2,
        padding: &[u8; V2_PADDING_BYTES],
    ) -> [u8; DITHER_V2_ENVELOPE_BYTES] {
        let mut bytes = [0; DITHER_V2_ENVELOPE_BYTES];
        bytes[..DITHER_NATIVE_FIELD_BYTES].copy_from_slice(&parameters.to_native_bytes());
        bytes[DITHER_NATIVE_FIELD_BYTES..].copy_from_slice(padding);
        bytes
    }

    pub fn decode_v2(
        bytes: &[u8],
    ) -> Result<(DitherParametersV2, [u8; V2_PADDING_BYTES]), DitherCodecError> {
        if bytes.len() != DITHER_V2_ENVELOPE_BYTES {
            return Err(DitherCodecError::InvalidLength {
                expected: DITHER_V2_ENVELOPE_BYTES,
                actual: bytes.len(),
            });
        }
        let native: &[u8; DITHER_NATIVE_FIELD_BYTES] = bytes[..DITHER_NATIVE_FIELD_BYTES]
            .try_into()
            .expect("qualified v2 native prefix length");
        let mut padding = [0; V2_PADDING_BYTES];
        padding.copy_from_slice(&bytes[DITHER_NATIVE_FIELD_BYTES..]);
        Ok((DitherParametersV2::from_native_bytes(native), padding))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DitherHistory {
    V1(DitherParametersV1),
    V2(DitherParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl DitherHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, DitherCodecError> {
        Self::decode_with_copy_limit(version, bytes, usize::MAX)
    }

    fn decode_with_copy_limit(
        version: u16,
        bytes: &[u8],
        allocation_limit: usize,
    ) -> Result<Self, DitherCodecError> {
        match version {
            1 => {
                let parameters = DitherParametersV1::from_bytes(bytes)?;
                if DitherMethod::from_id(parameters.method_id).is_err() {
                    return Ok(Self::Opaque {
                        version,
                        bytes: fallible_copy_with_limit(bytes, allocation_limit)?,
                    });
                }
                Ok(Self::V1(parameters))
            }
            2 => {
                let parameters = DitherParametersV2::from_bytes(bytes)?;
                if DitherMethod::from_id(parameters.method_id).is_err() {
                    return Ok(Self::Opaque {
                        version,
                        bytes: fallible_copy_with_limit(bytes, allocation_limit)?,
                    });
                }
                Ok(Self::V2(parameters))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: fallible_copy_with_limit(bytes, allocation_limit)?,
            }),
        }
    }

    #[must_use]
    pub const fn migrate_v1(&self) -> Option<DitherParametersV2> {
        let Self::V1(parameters) = self else {
            return None;
        };
        Some(migrate_v1_parameters(parameters))
    }

    pub fn payload(&self) -> Result<Vec<u8>, DitherCodecError> {
        self.payload_with_copy_limit(usize::MAX)
    }

    fn payload_with_copy_limit(
        &self,
        allocation_limit: usize,
    ) -> Result<Vec<u8>, DitherCodecError> {
        match self {
            Self::V1(parameters) => {
                fallible_copy_with_limit(&parameters.to_bytes(), allocation_limit)
            }
            Self::V2(parameters) => {
                fallible_copy_with_limit(&parameters.to_bytes(), allocation_limit)
            }
            Self::Opaque { bytes, .. } => fallible_copy_with_limit(bytes, allocation_limit),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => DITHER_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub const fn current(&self) -> Result<DitherParametersV2, DitherCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_parameters(parameters)),
            Self::V2(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(DitherCodecError::UnsupportedVersion(*version)),
        }
    }
}

const fn migrate_v1_parameters(parameters: &DitherParametersV1) -> DitherParametersV2 {
    DitherParametersV2 {
        method_id: parameters.method_id,
        palette: parameters.palette,
        radius: parameters.radius,
        range: parameters.range,
        damping: parameters.damping,
    }
}

fn fallible_copy_with_limit(
    bytes: &[u8],
    allocation_limit: usize,
) -> Result<Vec<u8>, DitherCodecError> {
    if bytes.len() > allocation_limit {
        return Err(DitherCodecError::AllocationFailed {
            required: bytes.len(),
        });
    }
    let mut copied = Vec::new();
    copied
        .try_reserve_exact(bytes.len())
        .map_err(|_| DitherCodecError::AllocationFailed {
            required: bytes.len(),
        })?;
    copied.extend_from_slice(bytes);
    Ok(copied)
}

#[derive(Debug, Clone, PartialEq)]
pub enum DitherCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownMethod(DitherMethodError),
    InvalidFields(DitherConfigError),
    AllocationFailed { required: usize },
    UnsupportedVersion(u16),
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
            Self::AllocationFailed { required } => {
                write!(formatter, "dither could not allocate {required} bytes")
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported dither history version {version}")
            }
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

    /// Source-derived module defaults (`DITHER_FSAUTO`, `-100 dB`).
    #[must_use]
    pub fn defaults() -> Self {
        Self::new(DitherMethod::FsAuto, DITHER_DEFAULT_DAMPING)
            .expect("source dither defaults are finite and in range")
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

impl TryFrom<&DitherParametersV2> for DitherConfig {
    type Error = DitherConfigError;

    fn try_from(parameters: &DitherParametersV2) -> Result<Self, Self::Error> {
        let method = DitherMethod::from_id(parameters.method_id)
            .map_err(|_| DitherConfigError::UnknownMethod(parameters.method_id))?;
        Self::new(method, parameters.damping)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DitherConfigError {
    UnknownMethod(u32),
    NonFiniteDamping,
    DampingOutOfRange(f32),
    InvalidPosterizeLevels(u8),
    NonFiniteReservedField,
    InvalidReservedRange,
}

impl fmt::Display for DitherConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownMethod(method) => write!(formatter, "unknown dither method {method}"),
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
        let native_log_input = 1.0_f32 / scale.get();
        if !native_log_input.is_finite() || !native_log_input.log2().is_finite() {
            return Err(DitherContextError::ScaleOutOfNativeRange);
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
    ScaleOutOfNativeRange,
}

impl fmt::Display for DitherContextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFiniteScale => "dither scale must be finite",
            Self::InvalidScale => "dither scale must be greater than zero",
            Self::ScaleOutOfNativeRange => {
                "dither scale is outside the finite native log2 calculation range"
            }
        })
    }
}

impl std::error::Error for DitherContextError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DitherExecutionError {
    Cancelled,
    AllocationFailed { required: usize },
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteOutput { pixel: usize, channel: usize },
}

impl fmt::Display for DitherExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("dither execution was cancelled"),
            Self::AllocationFailed { required } => {
                write!(
                    formatter,
                    "dither could not allocate {required} output bytes"
                )
            }
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
        self.execute_with_cancellation(pixels, || false)
    }

    /// Executes into private candidate buffers, polling cancellation at bounded
    /// intervals. No output is returned unless the complete raster succeeds.
    pub fn execute_with_cancellation<F>(
        &self,
        pixels: &[LinearRgb],
        mut cancelled: F,
    ) -> Result<Vec<LinearRgb>, DitherExecutionError>
    where
        F: FnMut() -> bool,
    {
        let expected = usize::try_from(self.dimensions.pixel_count()).unwrap_or(usize::MAX);
        if pixels.len() != expected {
            return Err(DitherExecutionError::DimensionsMismatch {
                expected,
                actual: pixels.len(),
            });
        }
        if cancelled() {
            return Err(DitherExecutionError::Cancelled);
        }

        let mut values = try_vec_with_capacity::<[f32; 3]>(pixels.len())?;
        for (index, pixel) in pixels.iter().enumerate() {
            if cancellation_interval_elapsed(index) && cancelled() {
                return Err(DitherExecutionError::Cancelled);
            }
            values.push([pixel.red().get(), pixel.green().get(), pixel.blue().get()]);
        }
        self.execute_values_with_cancellation(&mut values, &mut cancelled)?;

        let mut output = try_vec_with_capacity::<LinearRgb>(values.len())?;
        for (pixel, value) in values.into_iter().enumerate() {
            if cancellation_interval_elapsed(pixel) && cancelled() {
                return Err(DitherExecutionError::Cancelled);
            }
            let red = finite_output(value[0], pixel, 0)?;
            let green = finite_output(value[1], pixel, 1)?;
            let blue = finite_output(value[2], pixel, 2)?;
            output.push(LinearRgb::new(red, green, blue));
        }
        if cancelled() {
            return Err(DitherExecutionError::Cancelled);
        }
        Ok(output)
    }

    fn execute_values_with_cancellation<F>(
        &self,
        values: &mut [[f32; 3]],
        cancelled: &mut F,
    ) -> Result<(), DitherExecutionError>
    where
        F: FnMut() -> bool,
    {
        match self.config.method {
            DitherMethod::Random => self.random(values, cancelled),
            DitherMethod::Posterize(levels) => posterize(values, u32::from(levels), cancelled),
            method => {
                let Some(levels) = self.levels(method) else {
                    return clip_values(values, cancelled);
                };
                floyd_steinberg(
                    values,
                    self.dimensions,
                    levels,
                    method.is_gray()
                        || (matches!(method, DitherMethod::FsAuto) && self.context.gray),
                    cancelled,
                )
            }
        }
    }

    fn levels(&self, method: DitherMethod) -> Option<u32> {
        let scale = self.context.scale.get();
        let l1 = (1.0_f32 + (1.0_f32 / scale).log2()).floor() as i32;
        let bds = if self.context.export {
            1
        } else {
            l1.saturating_mul(l1)
        };
        let levels = match method {
            DitherMethod::Fs1BitGray => clamp_levels(bds.saturating_add(1), 2, 256),
            DitherMethod::Fs1BitRgb => clamp_levels(bds.saturating_add(1), 2, 4),
            DitherMethod::Fs2BitGray | DitherMethod::Fs2BitRgb => 4,
            DitherMethod::Fs4BitGray => {
                clamp_levels(15_i32.saturating_mul(bds).saturating_add(1), 16, 256)
            }
            DitherMethod::Fs4BitRgb => 16,
            DitherMethod::Fs6BitGray => {
                clamp_levels(63_i32.saturating_mul(bds).saturating_add(1), 64, 256)
            }
            DitherMethod::Fs8BitRgb => 256,
            DitherMethod::Fs16BitRgb => 65_536,
            DitherMethod::FsAuto => {
                if self.context.preview {
                    return None;
                }
                match self.context.bit_depth {
                    DitherBitDepth::BlackWhite => 2,
                    DitherBitDepth::Int8 => 256,
                    DitherBitDepth::Int10 => 1_024,
                    DitherBitDepth::Int12 => 4_096,
                    DitherBitDepth::Int16 => 65_536,
                    DitherBitDepth::Float => return None,
                }
            }
            DitherMethod::Random | DitherMethod::Posterize(_) => return None,
        };
        Some(levels)
    }

    fn random<F>(
        &self,
        values: &mut [[f32; 3]],
        cancelled: &mut F,
    ) -> Result<(), DitherExecutionError>
    where
        F: FnMut() -> bool,
    {
        let amplitude = 2.0_f32.powf(self.config.damping.get() / 10.0_f32);
        let width = usize::try_from(self.dimensions.width()).expect("validated width fits usize");
        let height =
            usize::try_from(self.dimensions.height()).expect("validated height fits usize");
        // `DitherConfig::seed` predates the strict source audit. Native
        // arithmetic has no seed parameter, so it deliberately has no effect.
        let mut tea_state = [0_u32; 2];
        for row in 0..height {
            if cancelled() {
                return Err(DitherExecutionError::Cancelled);
            }
            tea_state[0] = u32::try_from(row)
                .unwrap_or(u32::MAX)
                .wrapping_mul(u32::try_from(height).unwrap_or(u32::MAX));
            for column in 0..width {
                let pixel = row * width + column;
                if cancellation_interval_elapsed(pixel) && cancelled() {
                    return Err(DitherExecutionError::Cancelled);
                }
                encrypt_tea(&mut tea_state);
                let dither = amplitude * tpdf(tea_state[0]);
                for value in &mut values[pixel] {
                    *value = clip_nan(*value + dither);
                }
            }
        }
        Ok(())
    }
}

fn try_vec_with_capacity<T>(length: usize) -> Result<Vec<T>, DitherExecutionError> {
    let required = std::mem::size_of::<T>().saturating_mul(length);
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| DitherExecutionError::AllocationFailed { required })?;
    Ok(output)
}

fn finite_output(
    value: f32,
    pixel: usize,
    channel: usize,
) -> Result<FiniteF32, DitherExecutionError> {
    FiniteF32::new(value).map_err(|_| DitherExecutionError::NonFiniteOutput { pixel, channel })
}

const fn cancellation_interval_elapsed(index: usize) -> bool {
    index != 0 && index.is_multiple_of(DITHER_CANCELLATION_INTERVAL)
}

fn clamp_levels(value: i32, minimum: i32, maximum: i32) -> u32 {
    value.clamp(minimum, maximum) as u32
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
    quantize_native(clip_nan(value), levels)
}

/// Native `_quantize`: one contracted f32 multiply-add before `ceilf`, then
/// one rounded f32 reciprocal multiplication. It intentionally does not clip.
fn quantize_native(value: f32, levels: u32) -> f32 {
    let factor = levels.saturating_sub(1) as f32;
    let reciprocal = 1.0_f32 / factor;
    reciprocal * value.mul_add(factor, -0.5_f32).ceil()
}

#[must_use]
pub fn rgb_to_gray(pixel: [f32; 3]) -> f32 {
    let red = 0.30_f32 * pixel[0];
    let red_green = 0.59_f32.mul_add(pixel[1], red);
    0.11_f32.mul_add(pixel[2], red_green)
}

fn clip_values<F>(values: &mut [[f32; 3]], cancelled: &mut F) -> Result<(), DitherExecutionError>
where
    F: FnMut() -> bool,
{
    for (index, pixel) in values.iter_mut().enumerate() {
        if cancellation_interval_elapsed(index) && cancelled() {
            return Err(DitherExecutionError::Cancelled);
        }
        *pixel = pixel.map(clip_nan);
    }
    Ok(())
}

fn posterize<F>(
    values: &mut [[f32; 3]],
    levels: u32,
    cancelled: &mut F,
) -> Result<(), DitherExecutionError>
where
    F: FnMut() -> bool,
{
    for (index, pixel) in values.iter_mut().enumerate() {
        if cancellation_interval_elapsed(index) && cancelled() {
            return Err(DitherExecutionError::Cancelled);
        }
        *pixel = pixel.map(|value| quantize_native(value, levels));
    }
    Ok(())
}

fn floyd_steinberg<F>(
    values: &mut [[f32; 3]],
    dimensions: RasterDimensions,
    levels: u32,
    gray: bool,
    cancelled: &mut F,
) -> Result<(), DitherExecutionError>
where
    F: FnMut() -> bool,
{
    clip_values(values, cancelled)?;
    let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
    let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
    if width < 3 || height < 3 {
        for (index, pixel) in values.iter_mut().enumerate() {
            if cancellation_interval_elapsed(index) && cancelled() {
                return Err(DitherExecutionError::Cancelled);
            }
            quantize_pixel(pixel, levels, gray);
        }
        return Ok(());
    }
    for y in 0..height {
        if cancelled() {
            return Err(DitherExecutionError::Cancelled);
        }
        for x in 0..width {
            let index = y * width + x;
            if cancellation_interval_elapsed(index) && cancelled() {
                return Err(DitherExecutionError::Cancelled);
            }
            let error = quantize_pixel(&mut values[index], levels, gray);
            if x + 1 < width {
                add_error(&mut values[index + 1], error, 7.0_f32 / 16.0_f32);
            }
            if y + 1 < height {
                if x > 0 {
                    add_error(&mut values[index + width - 1], error, 3.0_f32 / 16.0_f32);
                }
                add_error(&mut values[index + width], error, 5.0_f32 / 16.0_f32);
                if x + 1 < width {
                    add_error(&mut values[index + width + 1], error, 1.0_f32 / 16.0_f32);
                }
            }
        }
    }
    Ok(())
}

fn quantize_pixel(pixel: &mut [f32; 3], levels: u32, gray: bool) -> [f32; 3] {
    let old = *pixel;
    if gray {
        let quantized = quantize_native(rgb_to_gray(old), levels);
        *pixel = [quantized; 3];
        [old[0] - quantized, old[1] - quantized, old[2] - quantized]
    } else {
        *pixel = old.map(|value| quantize_native(value, levels));
        [old[0] - pixel[0], old[1] - pixel[1], old[2] - pixel[2]]
    }
}

fn add_error(pixel: &mut [f32; 3], error: [f32; 3], weight: f32) {
    for (value, error) in pixel.iter_mut().zip(error) {
        *value = error.mul_add(weight, *value);
    }
}

fn encrypt_tea(state: &mut [u32; 2]) {
    let key = [0xa341_316c_u32, 0xc801_3ea4, 0xad90_777d, 0x7e95_761e];
    let mut first = state[0];
    let mut second = state[1];
    let mut sum = 0_u32;
    for _ in 0..8 {
        sum = sum.wrapping_add(0x9e37_79b9);
        first = first.wrapping_add(
            ((second << 4).wrapping_add(key[0]))
                ^ second.wrapping_add(sum)
                ^ ((second >> 5).wrapping_add(key[1])),
        );
        second = second.wrapping_add(
            ((first << 4).wrapping_add(key[2]))
                ^ first.wrapping_add(sum)
                ^ ((first >> 5).wrapping_add(key[3])),
        );
    }
    state[0] = first;
    state[1] = second;
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
        id: DescriptorId::new(DITHER_COMPATIBILITY_ID, DITHER_RUST_ID, 2, 2, 1).expect("static ID"),
        parameters: vec![
            scalar("method", 0.0, 263.0, 5.0),
            scalar("palette", -2_147_483_648.0, 2_147_483_647.0, 0.0),
            scalar("radius", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range0", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range1", -f64::from(f32::MAX), f64::from(f32::MAX), 0.0),
            scalar("range2", -f64::from(f32::MAX), f64::from(f32::MAX), 1.0),
            scalar("range3", -f64::from(f32::MAX), f64::from(f32::MAX), 1.0),
            scalar("damping", -200.0, 0.0, f64::from(DITHER_DEFAULT_DAMPING)),
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

#[cfg(test)]
mod strict_tests {
    use super::{DitherCodecError, DitherExecutionError, DitherHistory, try_vec_with_capacity};

    #[test]
    fn allocation_capacity_overflow_is_fallible() {
        assert_eq!(
            try_vec_with_capacity::<u8>(usize::MAX),
            Err(DitherExecutionError::AllocationFailed {
                required: usize::MAX,
            })
        );
    }

    #[test]
    fn opaque_decode_and_reencode_use_typed_fallible_copies() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        assert_eq!(
            DitherHistory::decode_with_copy_limit(77, &bytes, 0),
            Err(DitherCodecError::AllocationFailed { required: 4 })
        );
        let history = DitherHistory::decode(77, &bytes).expect("opaque history");
        assert_eq!(
            history.payload_with_copy_limit(0),
            Err(DitherCodecError::AllocationFailed { required: 4 })
        );
        assert_eq!(history.payload().expect("opaque round trip"), bytes);
    }
}
