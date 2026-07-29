//! Darktable history rows decoded without overclaiming executable parity.
//!
//! `rusttable-compat` intentionally preserves operation payloads without
//! depending on the processing crate. This import-layer bridge is where the
//! pinned module version and opaque bytes become typed parameters. It does not
//! emit an executable operation until Darktable's blend/mask and multi-instance
//! payloads are also understood.
//!
//! The Color Reconstruction payload decoder mirrors `legacy_params` and the
//! v1/v2/v3 declarations in `src/iop/colorreconstruction.c`; its byte order is
//! the little-endian native history format used by the supported Darktable
//! catalog fixtures.
//!
//! The Enlarge Canvas payload decoder mirrors the v1 declaration in
//! `src/iop/enlargecanvas.c`: four little-endian `f32` percentages followed by
//! the native five-value canvas-color enum.

use std::{fmt, mem::size_of};

use rusttable_compat::{CompatHistoryStep, EnabledState};
use rusttable_processing::operations::bloom::{BLOOM_PARAMETER_BYTES, BloomConfig, BloomHistory};
use rusttable_processing::operations::colorcontrast::{
    COLOR_CONTRAST_V2_PARAMETER_BYTES, ColorContrastConfig, ColorContrastHistory,
};
use rusttable_processing::operations::colorcorrection::{
    COLORCORRECTION_V1_PARAMETER_BYTES, ColorCorrectionConfig, ColorCorrectionHistory,
};
use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionConfig, ColorReconstructionV1, ColorReconstructionV2, ColorReconstructionV3,
    migrate_v1 as migrate_colorreconstruction_v1, migrate_v2 as migrate_colorreconstruction_v2,
};
use rusttable_processing::operations::crop::{
    CROP_PARAMETER_BYTES, CropCodecError, CropConfig, CropParametersV3,
};
use rusttable_processing::operations::enlargecanvas::{
    ENLARGECANVAS_PARAMETER_BYTES, EnlargeCanvasConfig, EnlargeCanvasHistoryParameters,
    decode_history as decode_enlargecanvas_history,
};
use rusttable_processing::operations::velvia::{
    VELVIA_V2_PARAMETER_BYTES, VelviaConfig, VelviaHistory,
};
use rusttable_processing::operations::vibrance::{
    VIBRANCE_V2_PARAMETER_BYTES, VibranceConfig, VibranceHistory,
};
use rusttable_processing::{COLORZONES_V5_PARAMETER_BYTES, ColorZonesConfig, ColorZonesHistory};

const BLOOM_COMPATIBILITY_NAME: &str = "bloom";
const COLOR_CONTRAST_COMPATIBILITY_NAME: &str = "colorcontrast";
const COLORCORRECTION_COMPATIBILITY_NAME: &str = "colorcorrection";
const COLORRECONSTRUCTION_COMPATIBILITY_NAME: &str = "colorreconstruct";
const CROP_COMPATIBILITY_NAME: &str = "crop";
const COLORZONES_COMPATIBILITY_NAME: &str = "colorzones";
const VELVIA_COMPATIBILITY_NAME: &str = "velvia";

const COLORRECONSTRUCTION_V1_PARAMETER_BYTES: usize = 3 * size_of::<f32>();
const COLORRECONSTRUCTION_V2_PARAMETER_BYTES: usize =
    COLORRECONSTRUCTION_V1_PARAMETER_BYTES + size_of::<i32>();
const COLORRECONSTRUCTION_V3_PARAMETER_BYTES: usize = 4 * size_of::<f32>() + size_of::<i32>();
const ENLARGECANVAS_COMPATIBILITY_NAME: &str = "enlargecanvas";
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

/// One decoded Bloom v1 core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBloomHistoryStep {
    /// The exact original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked native v1 core parameters.
    pub config: BloomConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v1 parameter bytes.
    pub canonical_parameters: [u8; BLOOM_PARAMETER_BYTES],
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
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

/// One decoded Color Reconstruction core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorReconstructionHistoryStep {
    /// The exact original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current v3 core parameters.
    pub config: ColorReconstructionConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical current-version parameter bytes in native field order.
    pub canonical_parameters: [u8; COLORRECONSTRUCTION_V3_PARAMETER_BYTES],
    /// Whether the source payload required the pinned v1/v2-to-v3 migration.
    pub migrated: bool,
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Crop v3 core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedCropHistoryStep {
    /// The exact original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current v3 core parameters.
    pub config: CropConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical 24-byte current-version parameters in native field order.
    pub canonical_parameters: [u8; CROP_PARAMETER_BYTES],
    /// Whether the source payload required a migration to v3.
    pub migrated: bool,
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Enlarge Canvas v1 core whose complete Darktable row is not
/// executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedEnlargeCanvasHistoryStep {
    /// The exact original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked native v1 core parameters.
    pub config: EnlargeCanvasConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical native v1 parameter bytes.
    pub canonical_parameters: [u8; ENLARGECANVAS_PARAMETER_BYTES],
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Color Zones core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorZonesHistoryStep {
    /// The exact original row, including opaque blend and multi-instance metadata.
    pub source: CompatHistoryStep,
    /// Checked current v5 core parameters.
    pub config: ColorZonesConfig,
    /// Native enabled state retained without creating an executable operation.
    pub enabled: bool,
    /// Original Darktable parameter version.
    pub source_version: u16,
    /// Canonical current-version parameter bytes.
    pub canonical_parameters: [u8; COLORZONES_V5_PARAMETER_BYTES],
    /// Whether the source payload required a pinned v1-v4 migration.
    pub migrated: bool,
    /// Explicit reason no executable imported operation was emitted.
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// Typed-but-pending result or a byte-preserving unsupported row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarktableHistoryStepDecode {
    /// Bloom v1 core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    BloomPendingBlend(DecodedBloomHistoryStep),
    /// Color Contrast core parameters are decoded, while blend/mask semantics
    /// remain explicitly non-executable.
    ColorContrastPendingBlend(DecodedColorContrastHistoryStep),
    /// Color Correction v1 core parameters are decoded, while blend/mask
    /// semantics remain explicitly non-executable.
    ColorCorrectionPendingBlend(DecodedColorCorrectionHistoryStep),
    /// Color Reconstruction core parameters are decoded and migrated to canonical v3,
    /// while blend/mask semantics remain explicitly non-executable.
    ColorReconstructionPendingBlend(DecodedColorReconstructionHistoryStep),
    /// Crop v3 core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    CropPendingBlend(DecodedCropHistoryStep),
    /// Enlarge Canvas v1 core parameters are decoded, while blend/mask
    /// semantics remain explicitly non-executable.
    EnlargeCanvasPendingBlend(DecodedEnlargeCanvasHistoryStep),
    /// Color Zones core parameters are decoded and migrated to canonical v5,
    /// while blend/mask semantics remain explicitly non-executable.
    ColorZonesPendingBlend(Box<DecodedColorZonesHistoryStep>),
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
        Some(BLOOM_COMPATIBILITY_NAME) => decode_bloom_history_step(step),
        Some(COLOR_CONTRAST_COMPATIBILITY_NAME) => decode_colorcontrast_history_step(step),
        Some(COLORCORRECTION_COMPATIBILITY_NAME) => decode_colorcorrection_history_step(step),
        Some(COLORRECONSTRUCTION_COMPATIBILITY_NAME) => {
            decode_colorreconstruction_history_step(step)
        }
        Some(CROP_COMPATIBILITY_NAME) => decode_crop_history_step(step),
        Some(ENLARGECANVAS_COMPATIBILITY_NAME) => decode_enlargecanvas_history_step(step),
        Some(COLORZONES_COMPATIBILITY_NAME) => decode_colorzones_history_step(step),
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

fn decode_bloom_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Bloom") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match BloomHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Bloom v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = match history {
        BloomHistory::V1(parameters) => parameters,
        BloomHistory::Opaque { .. } => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Bloom v{source_version} parameters remain opaque: only native v1 is typed"
                ),
            );
        }
    };
    let canonical_parameters = parameters.to_bytes();
    let config = match BloomConfig::try_from(parameters) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable Bloom v{source_version} parameters are not executable: {error}"),
            );
        }
    };

    DarktableHistoryStepDecode::BloomPendingBlend(DecodedBloomHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Bloom core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
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

fn decode_colorreconstruction_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Reconstruction") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let (current, migrated) = match decode_colorreconstruction_parameters(
        source_version,
        &step.operation_params.bytes,
    ) {
        Ok(Some(decoded)) => decoded,
        Ok(None) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Color Reconstruction v{source_version} parameters remain opaque: only native v1, v2, and v3 are typed"
                ),
            );
        }
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Reconstruction v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let config = match current.config() {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Reconstruction v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::ColorReconstructionPendingBlend(
        DecodedColorReconstructionHistoryStep {
            source: step.clone(),
            config,
            enabled,
            source_version,
            canonical_parameters: encode_colorreconstruction_v3(current),
            migrated,
            execution_blocker: DarktableHistoryDecodeFinding {
                code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
                detail: format!(
                    "Darktable Color Reconstruction core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                    step.blend_version
                ),
            },
        },
    )
}

fn decode_colorreconstruction_parameters(
    source_version: u16,
    bytes: &[u8],
) -> Result<Option<(ColorReconstructionV3, bool)>, String> {
    match source_version {
        1 => {
            require_parameter_size(bytes, COLORRECONSTRUCTION_V1_PARAMETER_BYTES)?;
            let parameters = ColorReconstructionV1 {
                threshold: read_f32_le(bytes, 0),
                spatial: read_f32_le(bytes, 4),
                range: read_f32_le(bytes, 8),
            };
            Ok(Some((migrate_colorreconstruction_v1(parameters), true)))
        }
        2 => {
            require_parameter_size(bytes, COLORRECONSTRUCTION_V2_PARAMETER_BYTES)?;
            let parameters = ColorReconstructionV2 {
                threshold: read_f32_le(bytes, 0),
                spatial: read_f32_le(bytes, 4),
                range: read_f32_le(bytes, 8),
                precedence: read_i32_le(bytes, 12),
            };
            Ok(Some((migrate_colorreconstruction_v2(parameters), true)))
        }
        3 => {
            require_parameter_size(bytes, COLORRECONSTRUCTION_V3_PARAMETER_BYTES)?;
            Ok(Some((
                ColorReconstructionV3 {
                    threshold: read_f32_le(bytes, 0),
                    spatial: read_f32_le(bytes, 4),
                    range: read_f32_le(bytes, 8),
                    hue: read_f32_le(bytes, 12),
                    precedence: read_i32_le(bytes, 16),
                },
                false,
            )))
        }
        _ => Ok(None),
    }
}

fn require_parameter_size(bytes: &[u8], expected: usize) -> Result<(), String> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(format!(
            "expected {expected} bytes in the pinned native layout, found {}",
            bytes.len()
        ))
    }
}

fn read_f32_le(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + size_of::<f32>()]
            .try_into()
            .expect("payload size was checked before decoding"),
    )
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + size_of::<i32>()]
            .try_into()
            .expect("payload size was checked before decoding"),
    )
}

fn encode_colorreconstruction_v3(
    parameters: ColorReconstructionV3,
) -> [u8; COLORRECONSTRUCTION_V3_PARAMETER_BYTES] {
    let mut bytes = [0_u8; COLORRECONSTRUCTION_V3_PARAMETER_BYTES];
    bytes[0..4].copy_from_slice(&parameters.threshold.to_le_bytes());
    bytes[4..8].copy_from_slice(&parameters.spatial.to_le_bytes());
    bytes[8..12].copy_from_slice(&parameters.range.to_le_bytes());
    bytes[12..16].copy_from_slice(&parameters.hue.to_le_bytes());
    bytes[16..20].copy_from_slice(&parameters.precedence.to_le_bytes());
    bytes
}

fn decode_crop_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Crop") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    if source_version != 3 {
        let result = rusttable_processing::operations::crop::decode_legacy(
            source_version,
            &step.operation_params.bytes,
        );
        let (code, detail) = match result {
            Err(CropCodecError::LegacyPayloadOpaque { expected, .. }) => (
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Crop v{source_version} payload remains opaque: the native {expected}-byte ABI layout requires source-context migration"
                ),
            ),
            Err(error) => (
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Crop v{source_version} parameters could not be decoded: {error}"
                ),
            ),
            Ok(()) => unreachable!("Crop legacy decoder never materializes a payload"),
        };
        return preserved(step, code, detail);
    }

    let parameters = match CropParametersV3::from_bytes(&step.operation_params.bytes) {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable Crop v3 parameters could not be decoded: {error}"),
            );
        }
    };
    let config = parameters.config();
    DarktableHistoryStepDecode::CropPendingBlend(DecodedCropHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        migrated: false,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Crop core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decode_enlargecanvas_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Enlarge Canvas") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match decode_enlargecanvas_history(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Enlarge Canvas v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = match history {
        EnlargeCanvasHistoryParameters::V1(parameters) => parameters,
        EnlargeCanvasHistoryParameters::Opaque { .. } => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Enlarge Canvas v{source_version} parameters remain opaque: only native v1 is typed"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::EnlargeCanvasPendingBlend(DecodedEnlargeCanvasHistoryStep {
        source: step.clone(),
        config: parameters.config(),
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Enlarge Canvas core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    })
}

fn decode_colorzones_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Zones") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match ColorZonesHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Zones v{source_version} parameters could not be decoded: {error}"
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
                    "Darktable Color Zones v{source_version} parameters remain opaque: {error}"
                ),
            );
        }
    };
    let canonical_parameters = current.to_bytes();
    let config = match ColorZonesConfig::try_from(&current) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Zones v{source_version} parameters have invalid active semantics: {error}"
                ),
            );
        }
    };

    DarktableHistoryStepDecode::ColorZonesPendingBlend(Box::new(DecodedColorZonesHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters,
        migrated: source_version != 5,
        execution_blocker: DarktableHistoryDecodeFinding {
            code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
            detail: format!(
                "Darktable Color Zones core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
                step.blend_version
            ),
        },
    }))
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
