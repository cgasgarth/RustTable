//! Source-faithful Channel Mixer history compatibility from `src/iop/channelmixer.c`.
//!
//! The native v1 payload is three contiguous `float[7]` rows (84 bytes). The
//! native v2 payload keeps those rows and appends a four-byte algorithm enum
//! (88 bytes on the supported little-endian ABIs). This leaf decodes only the
//! operation core; blend/mask and multi-instance execution remain explicitly
//! pending.

use std::{fmt, mem::size_of};

use crate::{CompatHistoryStep, EnabledState};

/// Darktable's persisted operation name for this module.
pub const CHANNELMIXER_COMPATIBILITY_NAME: &str = "channelmixer";
/// Alias using the word boundary from the native module name.
pub const CHANNEL_MIXER_COMPATIBILITY_NAME: &str = CHANNELMIXER_COMPATIBILITY_NAME;
/// The legacy Channel Mixer parameter version.
pub const CHANNELMIXER_V1_VERSION: u16 = 1;
/// The current Channel Mixer parameter version.
pub const CHANNELMIXER_V2_VERSION: u16 = 2;
/// Number of persisted destination channels: hue, saturation, lightness, red,
/// green, blue, and gray.
pub const CHANNELMIXER_OUTPUT_CHANNELS: usize = 7;
/// Number of persisted matrix values across the three native rows.
pub const CHANNELMIXER_MATRIX_VALUES: usize = 3 * CHANNELMIXER_OUTPUT_CHANNELS;
/// Native v1 payload size: three contiguous little-endian `float[7]` rows.
pub const CHANNELMIXER_V1_PARAMETER_BYTES: usize = CHANNELMIXER_MATRIX_VALUES * size_of::<f32>();
/// Native v2 payload size: the v1 rows followed by a four-byte native enum.
pub const CHANNELMIXER_V2_PARAMETER_BYTES: usize =
    CHANNELMIXER_V1_PARAMETER_BYTES + size_of::<i32>();

/// Alias for callers that spell the operation name with a word separator.
pub const CHANNEL_MIXER_V1_PARAMETER_BYTES: usize = CHANNELMIXER_V1_PARAMETER_BYTES;
/// Alias for callers that spell the operation name with a word separator.
pub const CHANNEL_MIXER_V2_PARAMETER_BYTES: usize = CHANNELMIXER_V2_PARAMETER_BYTES;

/// Persisted destination-row index from `src/iop/channelmixer.c`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerDestination {
    Hue = 0,
    Saturation = 1,
    Lightness = 2,
    Red = 3,
    Green = 4,
    Blue = 5,
    Gray = 6,
}

/// Persisted native algorithm enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerAlgorithm {
    /// Legacy HSL v1 processing semantics.
    V1 = 0,
    /// Current HSL v2/RGB/gray processing semantics.
    V2 = 1,
}

impl ChannelMixerAlgorithm {
    fn from_native(raw: i32) -> Result<Self, ChannelMixerCodecError> {
        match raw {
            0 => Ok(Self::V1),
            1 => Ok(Self::V2),
            raw => Err(ChannelMixerCodecError::InvalidAlgorithm { raw }),
        }
    }

    const fn native(self) -> i32 {
        self as i32
    }
}

/// One native v1 Channel Mixer payload, in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerParametersV1 {
    /// Native red input row: hue through gray destinations.
    pub red: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
    /// Native green input row: hue through gray destinations.
    pub green: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
    /// Native blue input row: hue through gray destinations.
    pub blue: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
}

impl ChannelMixerParametersV1 {
    /// Decodes exactly the three native little-endian `float[7]` rows.
    ///
    /// Finite values are retained without annotation or UI-range clamping;
    /// native `commit_params()` consumes the persisted values directly.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong native length or a non-finite matrix value.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        if bytes.len() != CHANNELMIXER_V1_PARAMETER_BYTES {
            return Err(ChannelMixerCodecError::WrongLength {
                expected: CHANNELMIXER_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self {
            red: read_row(bytes, 0, ChannelMixerRow::Red)?,
            green: read_row(
                bytes,
                CHANNELMIXER_OUTPUT_CHANNELS * size_of::<f32>(),
                ChannelMixerRow::Green,
            )?,
            blue: read_row(
                bytes,
                2 * CHANNELMIXER_OUTPUT_CHANNELS * size_of::<f32>(),
                ChannelMixerRow::Blue,
            )?,
        })
    }

    /// Encodes the rows in the exact native little-endian declaration order.
    #[must_use]
    pub fn to_native_le(self) -> [u8; CHANNELMIXER_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; CHANNELMIXER_V1_PARAMETER_BYTES];
        write_row(&mut bytes, 0, self.red);
        write_row(
            &mut bytes,
            CHANNELMIXER_OUTPUT_CHANNELS * size_of::<f32>(),
            self.green,
        );
        write_row(
            &mut bytes,
            2 * CHANNELMIXER_OUTPUT_CHANNELS * size_of::<f32>(),
            self.blue,
        );
        bytes
    }

    /// The v1 matrix equivalent of the native v2 identity defaults.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            red: [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            green: [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            blue: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
        }
    }
}

/// The native v2 Channel Mixer payload, including the persisted algorithm.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChannelMixerParametersV2 {
    /// Native red input row: hue through gray destinations.
    pub red: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
    /// Native green input row: hue through gray destinations.
    pub green: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
    /// Native blue input row: hue through gray destinations.
    pub blue: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
    /// Native algorithm enum, not a UI-only or inferred mode.
    pub algorithm: ChannelMixerAlgorithm,
}

impl ChannelMixerParametersV2 {
    /// Decodes all 21 finite values and the exact native four-byte enum.
    ///
    /// # Errors
    ///
    /// Returns an error for a wrong native length, a non-finite matrix value,
    /// or an algorithm value outside the two native enum variants.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        if bytes.len() != CHANNELMIXER_V2_PARAMETER_BYTES {
            return Err(ChannelMixerCodecError::WrongLength {
                expected: CHANNELMIXER_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let legacy =
            ChannelMixerParametersV1::from_native_le(&bytes[..CHANNELMIXER_V1_PARAMETER_BYTES])?;
        let algorithm = ChannelMixerAlgorithm::from_native(read_i32_le(
            bytes,
            CHANNELMIXER_V1_PARAMETER_BYTES,
        ))?;
        Ok(Self {
            red: legacy.red,
            green: legacy.green,
            blue: legacy.blue,
            algorithm,
        })
    }

    /// Encodes all 21 row values followed by the native algorithm enum.
    #[must_use]
    pub fn to_native_le(self) -> [u8; CHANNELMIXER_V2_PARAMETER_BYTES] {
        let mut bytes = [0_u8; CHANNELMIXER_V2_PARAMETER_BYTES];
        let rows = ChannelMixerParametersV1 {
            red: self.red,
            green: self.green,
            blue: self.blue,
        }
        .to_native_le();
        bytes[..CHANNELMIXER_V1_PARAMETER_BYTES].copy_from_slice(&rows);
        bytes[CHANNELMIXER_V1_PARAMETER_BYTES..]
            .copy_from_slice(&self.algorithm.native().to_le_bytes());
        bytes
    }

    /// The native v2 identity defaults from `init()`.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            red: [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0],
            green: [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            blue: [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            algorithm: ChannelMixerAlgorithm::V2,
        }
    }
}

/// Migrates the native v1 rows exactly as `legacy_params()` does.
///
/// The destination is zero-initialized and marked algorithm v1. HSL indices
/// and gray are always copied. The three RGB destinations are copied only when
/// all three legacy gray coefficients compare equal to `0.0f`; this preserves
/// the native suppression of RGB mixing when gray mixing is enabled.
#[must_use]
pub fn migrate_v1_to_v2(value: ChannelMixerParametersV1) -> ChannelMixerParametersV2 {
    let mut migrated = ChannelMixerParametersV2 {
        red: [0.0; CHANNELMIXER_OUTPUT_CHANNELS],
        green: [0.0; CHANNELMIXER_OUTPUT_CHANNELS],
        blue: [0.0; CHANNELMIXER_OUTPUT_CHANNELS],
        algorithm: ChannelMixerAlgorithm::V1,
    };

    for index in 0..3 {
        migrated.red[index] = value.red[index];
        migrated.green[index] = value.green[index];
        migrated.blue[index] = value.blue[index];
    }

    migrated.red[ChannelMixerDestination::Gray as usize] =
        value.red[ChannelMixerDestination::Gray as usize];
    migrated.green[ChannelMixerDestination::Gray as usize] =
        value.green[ChannelMixerDestination::Gray as usize];
    migrated.blue[ChannelMixerDestination::Gray as usize] =
        value.blue[ChannelMixerDestination::Gray as usize];

    let gray_is_zero = migrated.red[ChannelMixerDestination::Gray as usize] == 0.0
        && migrated.green[ChannelMixerDestination::Gray as usize] == 0.0
        && migrated.blue[ChannelMixerDestination::Gray as usize] == 0.0;
    if gray_is_zero {
        for index in 3..=5 {
            migrated.red[index] = value.red[index];
            migrated.green[index] = value.green[index];
            migrated.blue[index] = value.blue[index];
        }
    }

    migrated
}

/// Alias matching the native migration terminology.
#[must_use]
pub fn migrate_v1(value: ChannelMixerParametersV1) -> ChannelMixerParametersV2 {
    migrate_v1_to_v2(value)
}

/// Why a Channel Mixer payload could not become a typed core projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelMixerCodecError {
    /// The payload does not match the exact native struct size.
    WrongLength { expected: usize, actual: usize },
    /// A native matrix value is NaN or infinite.
    NonFinite { row: ChannelMixerRow, index: usize },
    /// The native enum contains a value not declared by Darktable.
    InvalidAlgorithm { raw: i32 },
}

impl fmt::Display for ChannelMixerCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(formatter, "expected {expected} bytes, found {actual}")
            }
            Self::NonFinite { row, index } => {
                write!(formatter, "{row:?}[{index}] is not finite")
            }
            Self::InvalidAlgorithm { raw } => {
                write!(formatter, "algorithm enum value {raw} is not declared")
            }
        }
    }
}

impl std::error::Error for ChannelMixerCodecError {}

/// Identifies the native matrix row containing a non-finite value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerRow {
    Red,
    Green,
    Blue,
}

/// Version-aware payload, retaining unknown versions and bytes verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelMixerHistory {
    /// Native v1 rows.
    V1(ChannelMixerParametersV1),
    /// Native v2 rows and algorithm.
    V2(ChannelMixerParametersV2),
    /// Unknown/future version retained without interpretation.
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ChannelMixerHistory {
    /// Decodes a known version or returns an opaque future-version payload.
    ///
    /// # Errors
    ///
    /// Returns an error when a known version has a malformed length, a
    /// non-finite matrix value, or an invalid algorithm enum.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ChannelMixerCodecError> {
        match version {
            CHANNELMIXER_V1_VERSION => {
                Ok(Self::V1(ChannelMixerParametersV1::from_native_le(bytes)?))
            }
            CHANNELMIXER_V2_VERSION => {
                Ok(Self::V2(ChannelMixerParametersV2::from_native_le(bytes)?))
            }
            version => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    /// Returns the exact source payload for known or opaque history.
    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_native_le().to_vec(),
            Self::V2(parameters) => parameters.to_native_le().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// Returns the original parameter version.
    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => CHANNELMIXER_V1_VERSION,
            Self::V2(_) => CHANNELMIXER_V2_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    /// Migrates v1 to the canonical v2 core without changing opaque history.
    #[must_use]
    pub fn migrate_v1(&self) -> Option<ChannelMixerParametersV2> {
        match self {
            Self::V1(parameters) => Some(migrate_v1_to_v2(*parameters)),
            Self::V2(parameters) => Some(*parameters),
            Self::Opaque { .. } => None,
        }
    }
}

/// Stable reason a Channel Mixer history row remains preserved or pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelMixerHistoryDecodeFindingCode {
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
pub struct ChannelMixerHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: ChannelMixerHistoryDecodeFindingCode,
    /// Bounded human-readable source compatibility evidence.
    pub detail: String,
}

/// Typed Channel Mixer core whose complete history row remains non-executable.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedChannelMixerHistoryStep {
    /// Exact original row, including blend/mask and multi-instance bytes.
    pub source: CompatHistoryStep,
    /// Checked canonical v2 core parameters.
    pub parameters: ChannelMixerParametersV2,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v2 parameter bytes.
    pub canonical_parameters: [u8; CHANNELMIXER_V2_PARAMETER_BYTES],
    /// Whether v1 migration produced the canonical parameters.
    pub migrated: bool,
    /// Native enabled state without creating an executable operation.
    pub enabled: bool,
    /// Explicit blocker for the unported remainder of the row.
    pub execution_blocker: ChannelMixerHistoryDecodeFinding,
}

/// Result of decoding one Channel Mixer history row without shared dispatch.
#[derive(Debug, Clone, PartialEq)]
pub enum ChannelMixerHistoryStepDecode {
    /// Core fields are typed; blend/mask and multi-instance behavior remain
    /// pending, so no executable imported operation is emitted.
    ChannelMixerPendingBlend(DecodedChannelMixerHistoryStep),
    /// Exact original row for unknown, malformed, or invalid inputs.
    Preserved {
        /// Exact original compatibility record.
        source: CompatHistoryStep,
        /// Why typed projection was not claimed.
        finding: ChannelMixerHistoryDecodeFinding,
    },
}

/// Decodes one Channel Mixer row's core without editing exhaustive import
/// dispatch. The owning import integrator must retain `source` as authoritative
/// until blend/mask and multi-instance behavior is proven.
#[must_use]
pub fn decode_channelmixer_history_step(step: &CompatHistoryStep) -> ChannelMixerHistoryStepDecode {
    let Some(raw_version) = step.module else {
        return preserved(
            step,
            ChannelMixerHistoryDecodeFindingCode::MissingModuleVersion,
            format!(
                "Darktable {CHANNELMIXER_COMPATIBILITY_NAME} history row has no module parameter version"
            ),
        );
    };
    let Ok(source_version) = u16::try_from(raw_version) else {
        return preserved(
            step,
            ChannelMixerHistoryDecodeFindingCode::InvalidModuleVersion,
            format!("Darktable Channel Mixer module version {raw_version} is outside 0..=65535"),
        );
    };
    if !matches!(
        source_version,
        CHANNELMIXER_V1_VERSION | CHANNELMIXER_V2_VERSION
    ) {
        return preserved(
            step,
            ChannelMixerHistoryDecodeFindingCode::UnsupportedParameterVersion,
            format!(
                "Darktable Channel Mixer v{source_version} parameters remain opaque: only native v1 and v2 are typed"
            ),
        );
    }

    let enabled = match step.enabled {
        EnabledState::Enabled => true,
        EnabledState::Disabled => false,
        state => {
            return preserved(
                step,
                ChannelMixerHistoryDecodeFindingCode::InvalidEnabledState,
                format!("Darktable Channel Mixer enabled state {state:?} is not executable"),
            );
        }
    };

    let (parameters, migrated) = match source_version {
        CHANNELMIXER_V1_VERSION => {
            let parameters =
                match ChannelMixerParametersV1::from_native_le(&step.operation_params.bytes) {
                    Ok(parameters) => parameters,
                    Err(error) => return invalid_parameters(step, source_version, &error),
                };
            (migrate_v1_to_v2(parameters), true)
        }
        CHANNELMIXER_V2_VERSION => {
            let parameters =
                match ChannelMixerParametersV2::from_native_le(&step.operation_params.bytes) {
                    Ok(parameters) => parameters,
                    Err(error) => return invalid_parameters(step, source_version, &error),
                };
            (parameters, false)
        }
        _ => unreachable!("known Channel Mixer versions were checked above"),
    };

    ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(DecodedChannelMixerHistoryStep {
        source: step.clone(),
        parameters,
        source_version,
        canonical_parameters: parameters.to_native_le(),
        migrated,
        enabled,
        execution_blocker: ChannelMixerHistoryDecodeFinding {
            code: ChannelMixerHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Channel Mixer core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque; no executable imported operation is emitted",
                step.blend_version
            ),
        },
    })
}

/// Alias using the word-separated operation name.
#[must_use]
pub fn decode_channel_mixer_history_step(
    step: &CompatHistoryStep,
) -> ChannelMixerHistoryStepDecode {
    decode_channelmixer_history_step(step)
}

fn invalid_parameters(
    step: &CompatHistoryStep,
    source_version: u16,
    error: &ChannelMixerCodecError,
) -> ChannelMixerHistoryStepDecode {
    preserved(
        step,
        ChannelMixerHistoryDecodeFindingCode::InvalidOperationParameters,
        format!(
            "Darktable Channel Mixer v{source_version} parameters could not be decoded: {error}"
        ),
    )
}

fn preserved(
    step: &CompatHistoryStep,
    code: ChannelMixerHistoryDecodeFindingCode,
    detail: String,
) -> ChannelMixerHistoryStepDecode {
    ChannelMixerHistoryStepDecode::Preserved {
        source: step.clone(),
        finding: ChannelMixerHistoryDecodeFinding { code, detail },
    }
}

fn read_row(
    bytes: &[u8],
    offset: usize,
    row: ChannelMixerRow,
) -> Result<[f32; CHANNELMIXER_OUTPUT_CHANNELS], ChannelMixerCodecError> {
    let mut values = [0.0; CHANNELMIXER_OUTPUT_CHANNELS];
    for (index, value) in values.iter_mut().enumerate() {
        *value = read_f32_le(bytes, offset + index * size_of::<f32>());
        if !value.is_finite() {
            return Err(ChannelMixerCodecError::NonFinite { row, index });
        }
    }
    Ok(values)
}

fn write_row(
    bytes: &mut [u8; CHANNELMIXER_V1_PARAMETER_BYTES],
    offset: usize,
    values: [f32; CHANNELMIXER_OUTPUT_CHANNELS],
) {
    for (index, value) in values.into_iter().enumerate() {
        let start = offset + index * size_of::<f32>();
        bytes[start..start + size_of::<f32>()].copy_from_slice(&value.to_le_bytes());
    }
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
