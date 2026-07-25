//! Darktable history rows decoded without overclaiming executable parity.
//!
//! `rusttable-compat` intentionally preserves operation payloads without
//! depending on the processing crate. This import-layer bridge is where the
//! pinned module version and opaque bytes become typed parameters. It does not
//! emit an executable operation until Darktable's blend/mask and multi-instance
//! payloads are also understood.

use std::fmt;

use rusttable_compat::{CompatHistoryStep, EnabledState};
use rusttable_processing::operations::colorcontrast::{
    COLOR_CONTRAST_V2_PARAMETER_BYTES, ColorContrastConfig, ColorContrastHistory,
};
use rusttable_processing::operations::colorcorrection::{
    COLORCORRECTION_V1_PARAMETER_BYTES, ColorCorrectionConfig, ColorCorrectionHistory,
};
use rusttable_processing::operations::velvia::{
    VELVIA_V2_PARAMETER_BYTES, VelviaConfig, VelviaHistory,
};
use rusttable_processing::operations::vibrance::{
    VIBRANCE_V2_PARAMETER_BYTES, VibranceConfig, VibranceHistory,
};

const COLOR_CONTRAST_COMPATIBILITY_NAME: &str = "colorcontrast";
const COLORCORRECTION_COMPATIBILITY_NAME: &str = "colorcorrection";
const VELVIA_COMPATIBILITY_NAME: &str = "velvia";
const VIBRANCE_COMPATIBILITY_NAME: &str = "vibrance";

/// Stable reason an imported history row remains opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarktableHistoryDecodeFindingCode {
    /// This bridge does not yet own the named Darktable operation.
    UnsupportedOperation,
    /// Darktable did not persist the module parameter version.
    MissingModuleVersion,
    /// The persisted version does not fit `RustTable`'s version domain.
    InvalidModuleVersion,
    /// The codec intentionally preserved an unknown future version.
    UnsupportedParameterVersion,
    /// A known-version payload was malformed or non-finite.
    InvalidOperationParameters,
    /// The enabled column was missing or outside Darktable's boolean domain.
    InvalidEnabledState,
    /// Core parameters are decoded, but native blend/mask execution is not.
    OpaqueBlendSemantics,
}

/// Actionable evidence explaining why a row was retained verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarktableHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: DarktableHistoryDecodeFindingCode,
    /// Bounded human-readable diagnostic.
    pub detail: String,
}

/// One decoded Velvia core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVelviaHistoryStep {
    /// The original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current core parameters.
    pub config: VelviaConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical current-version parameter bytes.
    pub canonical_parameters: [u8; VELVIA_V2_PARAMETER_BYTES],
    /// Whether the source payload required the pinned v1-to-v2 migration.
    pub migrated: bool,
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Vibrance core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedVibranceHistoryStep {
    /// The original row, including opaque blend and retained instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current core parameters.
    pub config: VibranceConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical current-version parameter bytes.
    pub canonical_parameters: [u8; VIBRANCE_V2_PARAMETER_BYTES],
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Color Contrast core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorContrastHistoryStep {
    /// The original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current core parameters, including hidden offsets and unbound mode.
    pub config: ColorContrastConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical current-version parameter bytes.
    pub canonical_parameters: [u8; COLOR_CONTRAST_V2_PARAMETER_BYTES],
    /// Whether the source payload required the pinned v1-to-v2 migration.
    pub migrated: bool,
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Color Correction v1 core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorCorrectionHistoryStep {
    /// The original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked native v1 core parameters.
    pub config: ColorCorrectionConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v1 parameter bytes.
    pub canonical_parameters: [u8; COLORCORRECTION_V1_PARAMETER_BYTES],
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// Typed-but-pending result or a byte-preserving unsupported row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarktableHistoryStepDecode {
    /// Color Contrast core parameters are decoded, while blend/mask semantics
    /// remain explicitly non-executable.
    ColorContrastPendingBlend(DecodedColorContrastHistoryStep),
    /// Color Correction v1 core parameters are decoded, while blend/mask
    /// semantics remain explicitly non-executable.
    ColorCorrectionPendingBlend(DecodedColorCorrectionHistoryStep),
    /// Velvia core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    VelviaPendingBlend(DecodedVelviaHistoryStep),
    /// Vibrance core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    VibrancePendingBlend(DecodedVibranceHistoryStep),
    /// The complete compatibility row remains available for a future port.
    Preserved {
        /// Exact original compatibility record.
        source: CompatHistoryStep,
        /// Why execution was not claimed.
        finding: DarktableHistoryDecodeFinding,
    },
}

/// Decodes one row's operation-specific payload without making any whole-
/// history execution decision.
///
/// Callers must retain the owning [`rusttable_compat::CompatHistory`] because
/// its `executable`, `order_proven`, selection, findings, and operation order
/// remain authoritative. Unsupported and invalid inputs are returned verbatim.
#[must_use]
pub fn decode_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    match step.operation.name.as_deref() {
        Some(COLOR_CONTRAST_COMPATIBILITY_NAME) => decode_colorcontrast_history_step(step),
        Some(COLORCORRECTION_COMPATIBILITY_NAME) => decode_colorcorrection_history_step(step),
        Some(VELVIA_COMPATIBILITY_NAME) => decode_velvia_history_step(step),
        Some(VIBRANCE_COMPATIBILITY_NAME) => decode_vibrance_history_step(step),
        _ => preserved(
            step,
            DarktableHistoryDecodeFindingCode::UnsupportedOperation,
            format!(
                "Darktable operation {:?} has no typed import materializer",
                step.operation.raw_name
            ),
        ),
    }
}

fn decode_colorcorrection_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Correction") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match ColorCorrectionHistory::decode(source_version, &step.operation_params.bytes)
    {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Correction v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let current = match history.current() {
        Ok(current) => current,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Color Correction v{source_version} parameters remain opaque: {error}"
                ),
            );
        }
    };
    let canonical_parameters = current.to_bytes();
    let config = match ColorCorrectionConfig::try_from(current) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Correction v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::ColorCorrectionPendingBlend(DecodedColorCorrectionHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Color Correction core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decode_colorcontrast_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Contrast") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match ColorContrastHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Contrast v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let current = match history.current() {
        Ok(current) => current,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Color Contrast v{source_version} parameters remain opaque: {error}"
                ),
            );
        }
    };
    let canonical_parameters = current.to_bytes();
    let config = match ColorContrastConfig::try_from(current) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Contrast v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::ColorContrastPendingBlend(DecodedColorContrastHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        migrated: source_version != 2,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Color Contrast core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decode_velvia_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Velvia") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };

    let history = match VelviaHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Velvia v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let current = match history.current() {
        Ok(current) => current,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable Velvia v{source_version} parameters remain opaque: {error}"),
            );
        }
    };
    let canonical_parameters = current.to_bytes();
    let config = match VelviaConfig::try_from(current) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Velvia v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::VelviaPendingBlend(DecodedVelviaHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        migrated: source_version != 2,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Velvia core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decode_vibrance_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Vibrance") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match VibranceHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Vibrance v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let current = match history.current() {
        Ok(current) => current,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable Vibrance v{source_version} parameters remain opaque: {error}"),
            );
        }
    };
    let canonical_parameters = current.to_bytes();
    let config = match VibranceConfig::try_from(current) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Vibrance v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::VibrancePendingBlend(DecodedVibranceHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Vibrance core parameters are decoded, but blend version {:?}, blend/mask bytes, and native multi-instance metadata remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decoded_row_header(
    step: &CompatHistoryStep,
    operation: &str,
) -> Result<(u16, bool), DarktableHistoryDecodeFinding> {
    let Some(raw_version) = step.module else {
        return Err(DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::MissingModuleVersion,
            detail: format!("Darktable {operation} history row has no module parameter version"),
        });
    };
    let Ok(source_version) = u16::try_from(raw_version) else {
        return Err(DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::InvalidModuleVersion,
            detail: format!(
                "Darktable {operation} module version {raw_version} is outside 0..=65535"
            ),
        });
    };
    let enabled = match step.enabled {
        EnabledState::Enabled => true,
        EnabledState::Disabled => false,
        state => {
            return Err(DarktableHistoryDecodeFinding {
                code: DarktableHistoryDecodeFindingCode::InvalidEnabledState,
                detail: format!("Darktable {operation} enabled state {state:?} is not executable"),
            });
        }
    };
    Ok((source_version, enabled))
}

fn preserved(
    step: &CompatHistoryStep,
    code: DarktableHistoryDecodeFindingCode,
    detail: String,
) -> DarktableHistoryStepDecode {
    DarktableHistoryStepDecode::Preserved {
        source: step.clone(),
        finding: DarktableHistoryDecodeFinding { code, detail },
    }
}

impl fmt::Display for DarktableHistoryDecodeFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.code, self.detail)
    }
}
