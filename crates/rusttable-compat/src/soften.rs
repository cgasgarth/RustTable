//! Source-faithful Soften v1 history compatibility.
//!
//! This is a direct compatibility projection of `src/iop/soften.c`. The native
//! `dt_iop_soften_params_t` declaration stores four contiguous `float` fields
//! in this order: `size`, `saturation`, `brightness`, and `amount` (source
//! lines 41-49). The module introspection version is 1 (source line 41).
//! This module decodes only that persisted core payload; blend/mask and
//! multi-instance execution remain explicitly pending.

use std::{fmt, mem::size_of};

use crate::{CompatHistoryStep, EnabledState};

/// Darktable's persisted operation name for this module.
pub const SOFTEN_COMPATIBILITY_NAME: &str = "soften";
/// The only Soften parameter version covered by this compatibility slice.
pub const SOFTEN_V1_VERSION: u16 = 1;
/// Native v1 payload size: four little-endian `f32` values.
pub const SOFTEN_V1_PARAMETER_BYTES: usize = 4 * size_of::<f32>();

/// Native v1 defaults declared by `dt_iop_soften_params_t` in `src/iop/soften.c`.
///
/// The source has no `init_presets()` registration for Soften, so this is a
/// fixture for the declared parameter defaults rather than a built-in preset.
pub const SOFTEN_V1_DEFAULT_NATIVE_LE: [u8; SOFTEN_V1_PARAMETER_BYTES] = [
    0x00, 0x00, 0x48, 0x42, // size = 50.0
    0x00, 0x00, 0xc8, 0x42, // saturation = 100.0
    0xc3, 0xf5, 0xa8, 0x3e, // brightness = 0.33
    0x00, 0x00, 0x48, 0x42, // amount = 50.0
];

/// Checked Soften v1 core fields in native declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoftenParametersV1 {
    /// Native blur size percentage.
    pub size: f32,
    /// Native saturation multiplier percentage.
    pub saturation: f32,
    /// Native brightness exposure value.
    pub brightness: f32,
    /// Native effect mix percentage.
    pub amount: f32,
}

impl SoftenParametersV1 {
    /// Decodes exactly four little-endian finite `f32` values.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the payload length differs from the native
    /// v1 struct or any decoded field is non-finite. Finite values are not
    /// clamped to UI ranges because native `commit_params()` copies them
    /// without validation.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, SoftenParameterDecodeError> {
        if bytes.len() != SOFTEN_V1_PARAMETER_BYTES {
            return Err(SoftenParameterDecodeError::WrongLength {
                expected: SOFTEN_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let parameters = Self {
            size: read_f32_le(bytes, 0),
            saturation: read_f32_le(bytes, 4),
            brightness: read_f32_le(bytes, 8),
            amount: read_f32_le(bytes, 12),
        };
        for (field, value) in [
            ("size", parameters.size),
            ("saturation", parameters.saturation),
            ("brightness", parameters.brightness),
            ("amount", parameters.amount),
        ] {
            if !value.is_finite() {
                return Err(SoftenParameterDecodeError::NonFinite { field });
            }
        }
        Ok(parameters)
    }

    /// Encodes the checked fields in the exact native little-endian order.
    #[must_use]
    pub fn to_native_le(self) -> [u8; SOFTEN_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; SOFTEN_V1_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.saturation.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.brightness.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.amount.to_le_bytes());
        bytes
    }
}

/// Why a Soften payload could not become the typed pending-core projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SoftenParameterDecodeError {
    /// The payload is not exactly the native v1 struct size.
    WrongLength { expected: usize, actual: usize },
    /// A native float is NaN or infinite; retain its bytes instead.
    NonFinite { field: &'static str },
}

impl fmt::Display for SoftenParameterDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(formatter, "expected {expected} bytes, found {actual}")
            }
            Self::NonFinite { field } => write!(formatter, "{field} is not finite"),
        }
    }
}

impl std::error::Error for SoftenParameterDecodeError {}

/// Stable reason a Soften history row remains preserved or pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoftenHistoryDecodeFindingCode {
    /// Darktable did not persist a module parameter version.
    MissingModuleVersion,
    /// The persisted version does not fit the compatibility version domain.
    InvalidModuleVersion,
    /// The persisted version is not the typed native v1 layout.
    UnsupportedParameterVersion,
    /// The v1 payload is malformed or contains a non-finite value.
    InvalidOperationParameters,
    /// The enabled column is missing or outside Darktable's boolean domain.
    InvalidEnabledState,
    /// Core fields are typed, but blend/mask and instance semantics are not.
    OpaqueBlendSemantics,
}

/// Actionable evidence for a preserved or pending Soften row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoftenHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: SoftenHistoryDecodeFindingCode,
    /// Bounded human-readable source compatibility evidence.
    pub detail: String,
}

/// Typed Soften v1 core whose complete history row remains non-executable.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSoftenHistoryStep {
    /// The exact original row, including blend/mask and multi-instance bytes.
    pub source: CompatHistoryStep,
    /// Checked fields in native declaration order.
    pub parameters: SoftenParametersV1,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v1 parameter bytes.
    pub canonical_parameters: [u8; SOFTEN_V1_PARAMETER_BYTES],
    /// Native enabled state, without creating an executable operation.
    pub enabled: bool,
    /// Explicit blocker for the unported remainder of the row.
    pub execution_blocker: SoftenHistoryDecodeFinding,
}

/// Result of decoding one Soften history row without whole-history execution.
#[derive(Debug, Clone, PartialEq)]
pub enum SoftenHistoryStepDecode {
    /// Soften v1 core fields are typed; blend/mask and multi-instance behavior
    /// remain pending, so no executable imported operation is emitted.
    SoftenPendingBlend(DecodedSoftenHistoryStep),
    /// The exact source row remains available for an unknown or invalid case.
    Preserved {
        /// Exact original compatibility record.
        source: CompatHistoryStep,
        /// Why typed projection was not claimed.
        finding: SoftenHistoryDecodeFinding,
    },
}

/// Decodes one Soften history row's native v1 core without editing the
/// exhaustive operation dispatch.
///
/// The canonical import integrator can call this helper from its Soften-specific
/// branch and retain `source` as the authoritative row until blend/mask and
/// multi-instance support is ported.
#[must_use]
pub fn decode_soften_history_step(step: &CompatHistoryStep) -> SoftenHistoryStepDecode {
    let Some(raw_version) = step.module else {
        return preserved(
            step,
            SoftenHistoryDecodeFindingCode::MissingModuleVersion,
            format!(
                "Darktable {SOFTEN_COMPATIBILITY_NAME} history row has no module parameter version"
            ),
        );
    };
    let Ok(source_version) = u16::try_from(raw_version) else {
        return preserved(
            step,
            SoftenHistoryDecodeFindingCode::InvalidModuleVersion,
            format!("Darktable Soften module version {raw_version} is outside 0..=65535"),
        );
    };
    if source_version != SOFTEN_V1_VERSION {
        return preserved(
            step,
            SoftenHistoryDecodeFindingCode::UnsupportedParameterVersion,
            format!(
                "Darktable Soften v{source_version} parameters remain opaque: only native v1 is typed"
            ),
        );
    }
    let enabled = match step.enabled {
        EnabledState::Enabled => true,
        EnabledState::Disabled => false,
        state => {
            return preserved(
                step,
                SoftenHistoryDecodeFindingCode::InvalidEnabledState,
                format!("Darktable Soften enabled state {state:?} is not executable"),
            );
        }
    };
    let parameters = match SoftenParametersV1::from_native_le(&step.operation_params.bytes) {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                SoftenHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable Soften v1 parameters could not be decoded: {error}"),
            );
        }
    };

    SoftenHistoryStepDecode::SoftenPendingBlend(DecodedSoftenHistoryStep {
        source: step.clone(),
        parameters,
        source_version,
        canonical_parameters: parameters.to_native_le(),
        enabled,
        execution_blocker: SoftenHistoryDecodeFinding {
            code: SoftenHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Soften core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque; no executable imported operation is emitted",
                step.blend_version
            ),
        },
    })
}

fn preserved(
    step: &CompatHistoryStep,
    code: SoftenHistoryDecodeFindingCode,
    detail: String,
) -> SoftenHistoryStepDecode {
    SoftenHistoryStepDecode::Preserved {
        source: step.clone(),
        finding: SoftenHistoryDecodeFinding { code, detail },
    }
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    let mut raw = [0_u8; size_of::<f32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<f32>()]);
    f32::from_le_bytes(raw)
}
