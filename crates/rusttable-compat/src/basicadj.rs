//! Source-faithful Basic Adjustments history compatibility from `src/iop/basicadj.c`.
//!
//! Darktable stores v1 as ten native fields (nine `float`s and one `int`) and
//! v2 as those fields with `vibrance` inserted before `clip`.  The decoder
//! accepts only the exact 40-byte and 44-byte little-endian layouts, rejects
//! non-finite float fields, and never clamps finite values to UI annotations.
//! A valid core row is typed but remains explicitly pending until the shared
//! import dispatcher proves blend/mask and multi-instance semantics. Invalid or
//! unsupported rows remain byte-preserved.

use std::{fmt, mem::size_of};

use crate::{CompatHistoryStep, EnabledState};

/// Darktable's persisted operation name for this module.
pub const BASICADJ_COMPATIBILITY_NAME: &str = "basicadj";
/// Alias matching the operation's source spelling.
pub const BASIC_ADJUSTMENTS_COMPATIBILITY_NAME: &str = BASICADJ_COMPATIBILITY_NAME;
/// The legacy Basic Adjustments parameter version.
pub const BASICADJ_V1_VERSION: u16 = 1;
/// The current Basic Adjustments parameter version.
pub const BASICADJ_V2_VERSION: u16 = 2;
/// Native v1 payload size: nine `float`s followed by one four-byte native `int`.
pub const BASICADJ_V1_PARAMETER_BYTES: usize = 40;
/// Native v2 payload size: ten `float`s followed by one four-byte native `int`.
pub const BASICADJ_V2_PARAMETER_BYTES: usize = 44;
/// Word-separated aliases for callers that use the operation's display name.
pub const BASIC_ADJUSTMENTS_V1_PARAMETER_BYTES: usize = BASICADJ_V1_PARAMETER_BYTES;
pub const BASIC_ADJUSTMENTS_V2_PARAMETER_BYTES: usize = BASICADJ_V2_PARAMETER_BYTES;

/// Native `dt_iop_rgb_norms_t` values declared by `src/common/rgb_norms.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjPreserveColors {
    None = 0,
    Luminance = 1,
    Max = 2,
    Average = 3,
    Sum = 4,
    Norm = 5,
    Power = 6,
}

impl BasicAdjPreserveColors {
    const fn from_native(raw: i32) -> Result<Self, BasicAdjCodecError> {
        match raw {
            0 => Ok(Self::None),
            1 => Ok(Self::Luminance),
            2 => Ok(Self::Max),
            3 => Ok(Self::Average),
            4 => Ok(Self::Sum),
            5 => Ok(Self::Norm),
            6 => Ok(Self::Power),
            raw => Err(BasicAdjCodecError::InvalidPreserveColors { raw }),
        }
    }

    const fn native(self) -> i32 {
        self as i32
    }
}

/// Native v1 fields in the exact declaration order from `basicadj.c`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV1 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: BasicAdjPreserveColors,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub clip: f32,
}

impl BasicAdjParametersV1 {
    /// Decodes exactly the native v1 payload in little-endian field order.
    ///
    /// Finite values outside the source UI annotations are accepted because
    /// native `commit_params()` consumes persisted values without clamping.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-native length, a non-finite float, or an
    /// undeclared preserve-colors enum value.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, BasicAdjCodecError> {
        if bytes.len() != BASICADJ_V1_PARAMETER_BYTES {
            return Err(BasicAdjCodecError::WrongLength {
                expected: BASICADJ_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let parameters = Self {
            black_point: read_f32_le(bytes, 0),
            exposure: read_f32_le(bytes, 4),
            hlcompr: read_f32_le(bytes, 8),
            hlcomprthresh: read_f32_le(bytes, 12),
            contrast: read_f32_le(bytes, 16),
            preserve_colors: BasicAdjPreserveColors::from_native(read_i32_le(bytes, 20))?,
            middle_grey: read_f32_le(bytes, 24),
            brightness: read_f32_le(bytes, 28),
            saturation: read_f32_le(bytes, 32),
            clip: read_f32_le(bytes, 36),
        };
        parameters.validate_finite()?;
        Ok(parameters)
    }

    /// Encodes fields in the exact native v1 little-endian order.
    #[must_use]
    pub fn to_native_le(self) -> [u8; BASICADJ_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; BASICADJ_V1_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.black_point);
        write_f32(&mut bytes, 4, self.exposure);
        write_f32(&mut bytes, 8, self.hlcompr);
        write_f32(&mut bytes, 12, self.hlcomprthresh);
        write_f32(&mut bytes, 16, self.contrast);
        bytes[20..24].copy_from_slice(&self.preserve_colors.native().to_le_bytes());
        write_f32(&mut bytes, 24, self.middle_grey);
        write_f32(&mut bytes, 28, self.brightness);
        write_f32(&mut bytes, 32, self.saturation);
        write_f32(&mut bytes, 36, self.clip);
        bytes
    }

    fn validate_finite(self) -> Result<(), BasicAdjCodecError> {
        for (field, value) in [
            ("black_point", self.black_point),
            ("exposure", self.exposure),
            ("hlcompr", self.hlcompr),
            ("hlcomprthresh", self.hlcomprthresh),
            ("contrast", self.contrast),
            ("middle_grey", self.middle_grey),
            ("brightness", self.brightness),
            ("saturation", self.saturation),
            ("clip", self.clip),
        ] {
            if !value.is_finite() {
                return Err(BasicAdjCodecError::NonFinite { field });
            }
        }
        Ok(())
    }
}

/// Native v2 fields in the exact declaration order from `basicadj.c`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV2 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: BasicAdjPreserveColors,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub clip: f32,
}

impl BasicAdjParametersV2 {
    /// Decodes exactly the native v2 payload in little-endian field order.
    ///
    /// # Errors
    ///
    /// Returns an error for a non-native length, a non-finite float, or an
    /// undeclared preserve-colors enum value.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, BasicAdjCodecError> {
        if bytes.len() != BASICADJ_V2_PARAMETER_BYTES {
            return Err(BasicAdjCodecError::WrongLength {
                expected: BASICADJ_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let parameters = Self {
            black_point: read_f32_le(bytes, 0),
            exposure: read_f32_le(bytes, 4),
            hlcompr: read_f32_le(bytes, 8),
            hlcomprthresh: read_f32_le(bytes, 12),
            contrast: read_f32_le(bytes, 16),
            preserve_colors: BasicAdjPreserveColors::from_native(read_i32_le(bytes, 20))?,
            middle_grey: read_f32_le(bytes, 24),
            brightness: read_f32_le(bytes, 28),
            saturation: read_f32_le(bytes, 32),
            vibrance: read_f32_le(bytes, 36),
            clip: read_f32_le(bytes, 40),
        };
        parameters.validate_finite()?;
        Ok(parameters)
    }

    /// Encodes fields in the exact native v2 little-endian order.
    #[must_use]
    pub fn to_native_le(self) -> [u8; BASICADJ_V2_PARAMETER_BYTES] {
        let mut bytes = [0_u8; BASICADJ_V2_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.black_point);
        write_f32(&mut bytes, 4, self.exposure);
        write_f32(&mut bytes, 8, self.hlcompr);
        write_f32(&mut bytes, 12, self.hlcomprthresh);
        write_f32(&mut bytes, 16, self.contrast);
        bytes[20..24].copy_from_slice(&self.preserve_colors.native().to_le_bytes());
        write_f32(&mut bytes, 24, self.middle_grey);
        write_f32(&mut bytes, 28, self.brightness);
        write_f32(&mut bytes, 32, self.saturation);
        write_f32(&mut bytes, 36, self.vibrance);
        write_f32(&mut bytes, 40, self.clip);
        bytes
    }

    fn validate_finite(self) -> Result<(), BasicAdjCodecError> {
        for (field, value) in [
            ("black_point", self.black_point),
            ("exposure", self.exposure),
            ("hlcompr", self.hlcompr),
            ("hlcomprthresh", self.hlcomprthresh),
            ("contrast", self.contrast),
            ("middle_grey", self.middle_grey),
            ("brightness", self.brightness),
            ("saturation", self.saturation),
            ("vibrance", self.vibrance),
            ("clip", self.clip),
        ] {
            if !value.is_finite() {
                return Err(BasicAdjCodecError::NonFinite { field });
            }
        }
        Ok(())
    }
}

/// The direct native `legacy_params()` v1-to-v2 migration.
#[must_use]
pub const fn migrate_v1_to_v2(value: BasicAdjParametersV1) -> BasicAdjParametersV2 {
    BasicAdjParametersV2 {
        black_point: value.black_point,
        exposure: value.exposure,
        hlcompr: value.hlcompr,
        hlcomprthresh: value.hlcomprthresh,
        contrast: value.contrast,
        preserve_colors: value.preserve_colors,
        middle_grey: value.middle_grey,
        brightness: value.brightness,
        saturation: value.saturation,
        vibrance: 0.0,
        clip: value.clip,
    }
}

/// Alias matching the native migration terminology.
#[must_use]
pub const fn migrate_v1(value: BasicAdjParametersV1) -> BasicAdjParametersV2 {
    migrate_v1_to_v2(value)
}

/// Why a Basic Adjustments payload could not become typed history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasicAdjCodecError {
    WrongLength { expected: usize, actual: usize },
    NonFinite { field: &'static str },
    InvalidPreserveColors { raw: i32 },
}

impl fmt::Display for BasicAdjCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(formatter, "expected {expected} bytes, found {actual}")
            }
            Self::NonFinite { field } => write!(formatter, "{field} is not finite"),
            Self::InvalidPreserveColors { raw } => {
                write!(
                    formatter,
                    "preserve-colors enum value {raw} is not declared"
                )
            }
        }
    }
}

impl std::error::Error for BasicAdjCodecError {}

/// Version-aware Basic Adjustments payload that retains unknown versions.
#[derive(Debug, Clone, PartialEq)]
pub enum BasicAdjHistory {
    V1(BasicAdjParametersV1),
    V2(BasicAdjParametersV2),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl BasicAdjHistory {
    /// Decodes v1/v2 and retains future versions without interpretation.
    ///
    /// # Errors
    ///
    /// Returns an error when a known v1 or v2 payload has the wrong native
    /// length, a non-finite float, or an undeclared preserve-colors enum.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, BasicAdjCodecError> {
        match version {
            BASICADJ_V1_VERSION => Ok(Self::V1(BasicAdjParametersV1::from_native_le(bytes)?)),
            BASICADJ_V2_VERSION => Ok(Self::V2(BasicAdjParametersV2::from_native_le(bytes)?)),
            version => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    /// Returns the exact native payload for known or opaque history.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_native_le().to_vec(),
            Self::V2(parameters) => parameters.to_native_le().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => BASICADJ_V1_VERSION,
            Self::V2(_) => BASICADJ_V2_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    /// Resolves either known version to canonical v2, leaving opaque versions unavailable.
    #[must_use]
    pub const fn migrate_v1(&self) -> Option<BasicAdjParametersV2> {
        match self {
            Self::V1(parameters) => Some(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Some(*parameters),
            Self::Opaque { .. } => None,
        }
    }
}

/// Stable reason a Basic Adjustments history row remains preserved or pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjHistoryDecodeFindingCode {
    /// Darktable did not persist a module parameter version.
    MissingModuleVersion,
    /// The persisted version is outside the compatibility version domain.
    InvalidModuleVersion,
    /// The persisted version is unknown or future.
    UnsupportedParameterVersion,
    /// A known-version payload is malformed, non-finite, or has an invalid enum.
    InvalidOperationParameters,
    /// The enabled column is missing or outside Darktable's boolean domain.
    InvalidEnabledState,
    /// Core fields are typed, but blend/mask and instance semantics are not.
    OpaqueBlendSemantics,
}

/// Actionable source-compatibility evidence for a preserved or pending row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicAdjHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: BasicAdjHistoryDecodeFindingCode,
    /// Bounded human-readable source compatibility evidence.
    pub detail: String,
}

/// Typed Basic Adjustments core whose complete history row remains non-executable.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedBasicAdjHistoryStep {
    /// Exact original row, including blend/mask and multi-instance bytes.
    pub source: CompatHistoryStep,
    /// Checked canonical v2 core parameters.
    pub parameters: BasicAdjParametersV2,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v2 parameter bytes.
    pub canonical_parameters: [u8; BASICADJ_V2_PARAMETER_BYTES],
    /// Whether v1 migration produced the canonical parameters.
    pub migrated: bool,
    /// Native enabled state without creating an executable operation.
    pub enabled: bool,
    /// Explicit blocker for the unported remainder of the row.
    pub execution_blocker: BasicAdjHistoryDecodeFinding,
}

/// Result of decoding one Basic Adjustments history row.
#[derive(Debug, Clone, PartialEq)]
pub enum BasicAdjHistoryStepDecode {
    /// Core fields are typed; blend/mask and multi-instance behavior remain
    /// pending, so no executable imported operation is emitted.
    BasicAdjPendingBlend(DecodedBasicAdjHistoryStep),
    /// The exact original row is retained for an unknown or invalid case.
    Preserved {
        source: CompatHistoryStep,
        finding: BasicAdjHistoryDecodeFinding,
    },
}

/// Decodes one Basic Adjustments row's core without editing exhaustive import
/// dispatch.
///
/// The owning import integrator must retain `source` as authoritative until
/// blend/mask and multi-instance behavior are proven.
#[must_use]
pub fn decode_basicadj_history_step(step: &CompatHistoryStep) -> BasicAdjHistoryStepDecode {
    let Some(raw_version) = step.module else {
        return preserved(
            step,
            BasicAdjHistoryDecodeFindingCode::MissingModuleVersion,
            "Darktable basicadj history row has no module parameter version".to_owned(),
        );
    };
    let Ok(source_version) = u16::try_from(raw_version) else {
        return preserved(
            step,
            BasicAdjHistoryDecodeFindingCode::InvalidModuleVersion,
            format!(
                "Darktable Basic Adjustments module version {raw_version} is outside 0..=65535"
            ),
        );
    };
    if !matches!(source_version, BASICADJ_V1_VERSION | BASICADJ_V2_VERSION) {
        return preserved(
            step,
            BasicAdjHistoryDecodeFindingCode::UnsupportedParameterVersion,
            format!(
                "Darktable Basic Adjustments v{source_version} parameters remain opaque: only native v1 and v2 are typed"
            ),
        );
    }

    let enabled = match step.enabled {
        EnabledState::Enabled => true,
        EnabledState::Disabled => false,
        state => {
            return preserved(
                step,
                BasicAdjHistoryDecodeFindingCode::InvalidEnabledState,
                format!("Darktable Basic Adjustments enabled state {state:?} is not executable"),
            );
        }
    };

    let (parameters, migrated) = match source_version {
        BASICADJ_V1_VERSION => {
            let parameters =
                match BasicAdjParametersV1::from_native_le(&step.operation_params.bytes) {
                    Ok(parameters) => parameters,
                    Err(error) => return invalid_parameters(step, source_version, &error),
                };
            (migrate_v1_to_v2(parameters), true)
        }
        BASICADJ_V2_VERSION => {
            let parameters =
                match BasicAdjParametersV2::from_native_le(&step.operation_params.bytes) {
                    Ok(parameters) => parameters,
                    Err(error) => return invalid_parameters(step, source_version, &error),
                };
            (parameters, false)
        }
        _ => unreachable!("known Basic Adjustments versions were checked above"),
    };

    BasicAdjHistoryStepDecode::BasicAdjPendingBlend(DecodedBasicAdjHistoryStep {
        source: step.clone(),
        parameters,
        source_version,
        canonical_parameters: parameters.to_native_le(),
        migrated,
        enabled,
        execution_blocker: BasicAdjHistoryDecodeFinding {
            code: BasicAdjHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Basic Adjustments core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque; no executable imported operation is emitted",
                step.blend_version
            ),
        },
    })
}

/// Alias using the operation's display-name spelling.
#[must_use]
pub fn decode_basic_adjustments_history_step(
    step: &CompatHistoryStep,
) -> BasicAdjHistoryStepDecode {
    decode_basicadj_history_step(step)
}

fn invalid_parameters(
    step: &CompatHistoryStep,
    source_version: u16,
    error: &BasicAdjCodecError,
) -> BasicAdjHistoryStepDecode {
    preserved(
        step,
        BasicAdjHistoryDecodeFindingCode::InvalidOperationParameters,
        format!(
            "Darktable Basic Adjustments v{source_version} parameters could not be decoded: {error}"
        ),
    )
}

fn preserved(
    step: &CompatHistoryStep,
    code: BasicAdjHistoryDecodeFindingCode,
    detail: String,
) -> BasicAdjHistoryStepDecode {
    BasicAdjHistoryStepDecode::Preserved {
        source: step.clone(),
        finding: BasicAdjHistoryDecodeFinding { code, detail },
    }
}

fn write_f32<const N: usize>(bytes: &mut [u8; N], offset: usize, value: f32) {
    bytes[offset..offset + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    let mut raw = [0_u8; size_of::<f32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<f32>()]);
    f32::from_le_bytes(raw)
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    let mut raw = [0_u8; size_of::<i32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<i32>()]);
    i32::from_le_bytes(raw)
}
