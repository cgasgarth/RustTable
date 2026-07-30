//! Source-faithful Sharpen v1 history compatibility.
//!
//! This is a direct compatibility projection of `src/iop/sharpen.c`.
//! The native `dt_iop_sharpen_params_t` declaration stores three contiguous
//! `float` fields in this order: `radius`, `amount`, and `threshold` (source
//! lines 42-47). The built-in raw-only preset at lines 98-108 supplies
//! `(2.0, 0.5, 0.5)`. This module decodes only that persisted core payload;
//! blend/mask and multi-instance execution remain explicitly pending.

use std::{fmt, mem::size_of};

use crate::{CompatHistoryStep, EnabledState};

/// Darktable's persisted operation name for this module.
pub const SHARPEN_COMPATIBILITY_NAME: &str = "sharpen";
/// The only Sharpen parameter version covered by this compatibility slice.
pub const SHARPEN_V1_VERSION: u16 = 1;
/// Native v1 payload size: three little-endian `f32` values.
pub const SHARPEN_V1_PARAMETER_BYTES: usize = 3 * size_of::<f32>();

/// Raw native bytes for the built-in raw-only preset registered by
/// `src/iop/sharpen.c` (`radius = 2.0`, `amount = 0.5`, `threshold = 0.5`).
///
/// The source passes `dt_iop_sharpen_params_t` and its `sizeof` directly to
/// `dt_gui_presets_add_generic`; no version conversion or field reordering is
/// involved in this payload.
pub const SHARPEN_V1_BUILTIN_PRESET_NATIVE_LE: [u8; SHARPEN_V1_PARAMETER_BYTES] = [
    0x00, 0x00, 0x00, 0x40, // radius = 2.0
    0x00, 0x00, 0x00, 0x3f, // amount = 0.5
    0x00, 0x00, 0x00, 0x3f, // threshold = 0.5
];

/// The exact native format restriction applied by `init_presets()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenBuiltinPresetApplicability {
    RawOnly,
}

/// Sharpen has exactly one source-owned built-in preset, restricted to RAW input.
pub const SHARPEN_V1_BUILTIN_PRESET_APPLICABILITY: SharpenBuiltinPresetApplicability =
    SharpenBuiltinPresetApplicability::RawOnly;

/// Checked Sharpen v1 core fields in native declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenParametersV1 {
    /// Native spatial radius.
    pub radius: f32,
    /// Native unsharp-mask amount.
    pub amount: f32,
    /// Native luma threshold.
    pub threshold: f32,
}

impl SharpenParametersV1 {
    /// Decodes exactly three little-endian finite `f32` values.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the payload length differs from the native
    /// v1 struct or any decoded field is non-finite.
    pub fn from_native_le(bytes: &[u8]) -> Result<Self, SharpenParameterDecodeError> {
        if bytes.len() != SHARPEN_V1_PARAMETER_BYTES {
            return Err(SharpenParameterDecodeError::WrongLength {
                expected: SHARPEN_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let parameters = Self {
            radius: read_f32_le(bytes, 0),
            amount: read_f32_le(bytes, 4),
            threshold: read_f32_le(bytes, 8),
        };
        for (field, value) in [
            ("radius", parameters.radius),
            ("amount", parameters.amount),
            ("threshold", parameters.threshold),
        ] {
            if !value.is_finite() {
                return Err(SharpenParameterDecodeError::NonFinite { field });
            }
        }
        Ok(parameters)
    }

    /// Encodes the checked fields in the exact native little-endian order.
    #[must_use]
    pub fn to_native_le(self) -> [u8; SHARPEN_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; SHARPEN_V1_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.radius.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.amount.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.threshold.to_le_bytes());
        bytes
    }
}

/// Why a Sharpen payload could not become the typed pending-core projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharpenParameterDecodeError {
    /// The payload is not exactly the native v1 struct size.
    WrongLength { expected: usize, actual: usize },
    /// A native float is NaN or infinite; retain its bytes instead.
    NonFinite { field: &'static str },
}

impl fmt::Display for SharpenParameterDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongLength { expected, actual } => {
                write!(formatter, "expected {expected} bytes, found {actual}")
            }
            Self::NonFinite { field } => write!(formatter, "{field} is not finite"),
        }
    }
}

impl std::error::Error for SharpenParameterDecodeError {}

/// Stable reason a Sharpen history row remains preserved or pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenHistoryDecodeFindingCode {
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

/// Actionable evidence for a preserved or pending Sharpen row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharpenHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: SharpenHistoryDecodeFindingCode,
    /// Bounded human-readable source compatibility evidence.
    pub detail: String,
}

/// Typed Sharpen v1 core whose complete history row remains non-executable.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSharpenHistoryStep {
    /// The exact original row, including blend/mask and multi-instance bytes.
    pub source: CompatHistoryStep,
    /// Checked fields in native declaration order.
    pub parameters: SharpenParametersV1,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v1 parameter bytes.
    pub canonical_parameters: [u8; SHARPEN_V1_PARAMETER_BYTES],
    /// Native enabled state, without creating an executable operation.
    pub enabled: bool,
    /// Explicit blocker for the unported remainder of the row.
    pub execution_blocker: SharpenHistoryDecodeFinding,
}

/// Result of decoding one Sharpen history row without whole-history execution.
#[derive(Debug, Clone, PartialEq)]
pub enum SharpenHistoryStepDecode {
    /// Sharpen v1 core fields are typed; blend/mask and multi-instance behavior
    /// remain pending, so no executable imported operation is emitted.
    SharpenPendingBlend(DecodedSharpenHistoryStep),
    /// The exact source row remains available for an unknown or invalid case.
    Preserved {
        /// Exact original compatibility record.
        source: CompatHistoryStep,
        /// Why typed projection was not claimed.
        finding: SharpenHistoryDecodeFinding,
    },
}

/// Decodes one Sharpen history row's native v1 core without editing the
/// exhaustive operation dispatch. The canonical import integrator can call
/// this helper from its Sharpen-specific branch and retain `source` as the
/// authoritative row until blend/mask and multi-instance support is ported.
#[must_use]
pub fn decode_sharpen_history_step(step: &CompatHistoryStep) -> SharpenHistoryStepDecode {
    let Some(raw_version) = step.module else {
        return preserved(
            step,
            SharpenHistoryDecodeFindingCode::MissingModuleVersion,
            format!(
                "Darktable {SHARPEN_COMPATIBILITY_NAME} history row has no module parameter version"
            ),
        );
    };
    let Ok(source_version) = u16::try_from(raw_version) else {
        return preserved(
            step,
            SharpenHistoryDecodeFindingCode::InvalidModuleVersion,
            format!("Darktable Sharpen module version {raw_version} is outside 0..=65535"),
        );
    };
    if source_version != SHARPEN_V1_VERSION {
        return preserved(
            step,
            SharpenHistoryDecodeFindingCode::UnsupportedParameterVersion,
            format!(
                "Darktable Sharpen v{source_version} parameters remain opaque: only native v1 is typed"
            ),
        );
    }
    let enabled = match step.enabled {
        EnabledState::Enabled => true,
        EnabledState::Disabled => false,
        state => {
            return preserved(
                step,
                SharpenHistoryDecodeFindingCode::InvalidEnabledState,
                format!("Darktable Sharpen enabled state {state:?} is not executable"),
            );
        }
    };
    let parameters = match SharpenParametersV1::from_native_le(&step.operation_params.bytes) {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                SharpenHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable Sharpen v1 parameters could not be decoded: {error}"),
            );
        }
    };

    SharpenHistoryStepDecode::SharpenPendingBlend(DecodedSharpenHistoryStep {
        source: step.clone(),
        parameters,
        source_version,
        canonical_parameters: parameters.to_native_le(),
        enabled,
        execution_blocker: SharpenHistoryDecodeFinding {
            code: SharpenHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Sharpen core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque; no executable imported operation is emitted",
                step.blend_version
            ),
        },
    })
}

fn preserved(
    step: &CompatHistoryStep,
    code: SharpenHistoryDecodeFindingCode,
    detail: String,
) -> SharpenHistoryStepDecode {
    SharpenHistoryStepDecode::Preserved {
        source: step.clone(),
        finding: SharpenHistoryDecodeFinding { code, detail },
    }
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    let mut raw = [0_u8; size_of::<f32>()];
    raw.copy_from_slice(&bytes[offset..offset + size_of::<f32>()]);
    f32::from_le_bytes(raw)
}
