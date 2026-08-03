//! Darktable history rows decoded with fail-closed executable eligibility.
//!
//! `rusttable-compat` preserves operation payloads without depending on the
//! processing crate. This import-layer bridge turns supported current payloads
//! into typed parameters and emits executable target rows only after native
//! order, instance, and identity-blend evidence is proven.
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

pub use rusttable_compat::basicadj::DecodedBasicAdjHistoryStep;
use rusttable_compat::basicadj::{
    BASICADJ_COMPATIBILITY_NAME, BasicAdjHistoryDecodeFindingCode, BasicAdjHistoryStepDecode,
    decode_basicadj_history_step,
};
pub use rusttable_compat::channelmixer::DecodedChannelMixerHistoryStep;
use rusttable_compat::channelmixer::{
    ChannelMixerHistoryDecodeFindingCode, ChannelMixerHistoryStepDecode,
    decode_channelmixer_history_step,
};
pub use rusttable_compat::sharpen::DecodedSharpenHistoryStep;
use rusttable_compat::sharpen::{
    SharpenHistoryDecodeFindingCode, SharpenHistoryStepDecode, decode_sharpen_history_step,
};
pub use rusttable_compat::soften::DecodedSoftenHistoryStep;
use rusttable_compat::soften::{
    SoftenHistoryDecodeFindingCode, SoftenHistoryStepDecode, decode_soften_history_step,
};
use rusttable_compat::{CompatHistory, CompatHistoryStep, EnabledState, is_identity_blend_v14};
use rusttable_processing::operations::agx::{
    AGX_COMPATIBILITY_ID, AGX_PARAMETER_BYTES_V7, AGX_SCHEMA_VERSION, AgxConfig, AgxHistory,
};
use rusttable_processing::operations::basecurve::{
    BASECURVE_SCHEMA_VERSION, BasecurveConfig, BasecurveHistory, BasecurveParameters,
};
use rusttable_processing::operations::bloom::{BLOOM_PARAMETER_BYTES, BloomConfig, BloomHistory};
use rusttable_processing::operations::colorcontrast::{
    COLOR_CONTRAST_V2_PARAMETER_BYTES, ColorContrastConfig, ColorContrastHistory,
};
use rusttable_processing::operations::colorcorrection::{
    COLORCORRECTION_V1_PARAMETER_BYTES, ColorCorrectionConfig, ColorCorrectionHistory,
};
use rusttable_processing::operations::colormapping::{
    COLOR_MAPPING_COMPATIBILITY_ID, ColorMappingConfig, ColorMappingHistory,
};
use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionConfig, ColorReconstructionV1, ColorReconstructionV2, ColorReconstructionV3,
    migrate_v1 as migrate_colorreconstruction_v1, migrate_v2 as migrate_colorreconstruction_v2,
};
use rusttable_processing::operations::colortransfer::{
    COLORTRANSFER_COMPATIBILITY_ID, COLORTRANSFER_SCHEMA_VERSION, ColorTransferParameters,
};
use rusttable_processing::operations::crop::{
    CROP_PARAMETER_BYTES, CropCodecError, CropConfig, CropParametersV3,
};
use rusttable_processing::operations::enlargecanvas::{
    ENLARGECANVAS_PARAMETER_BYTES, EnlargeCanvasConfig, EnlargeCanvasHistoryParameters,
    decode_history as decode_enlargecanvas_history,
};
use rusttable_processing::operations::highpass::{
    HIGHPASS_SCHEMA_VERSION, HighpassConfig, HighpassHistory, HighpassParametersV1,
};
use rusttable_processing::operations::levels::{
    LEVELS_COMPATIBILITY_ID, LEVELS_PARAMETER_BYTES_V2, LEVELS_SCHEMA_VERSION, LevelsConfig,
    LevelsHistory,
};
use rusttable_processing::operations::rgblevels::{
    RGBLEVELS_COMPATIBILITY_ID, RGBLEVELS_PARAMETER_BYTES, RgbLevelsConfig, RgbLevelsHistory,
};
use rusttable_processing::operations::tonecurve::{
    PARAMETER_VERSION as TONECURVE_SCHEMA_VERSION, ToneCurveConfig, ToneCurveHistory,
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
const CHANNELMIXER_COMPATIBILITY_NAME: &str = "channelmixer";
const SHARPEN_COMPATIBILITY_NAME: &str = "sharpen";
const SOFTEN_COMPATIBILITY_NAME: &str = "soften";
const VELVIA_COMPATIBILITY_NAME: &str = "velvia";

const COLORRECONSTRUCTION_V1_PARAMETER_BYTES: usize = 3 * size_of::<f32>();
const COLORRECONSTRUCTION_V2_PARAMETER_BYTES: usize =
    COLORRECONSTRUCTION_V1_PARAMETER_BYTES + size_of::<i32>();
const COLORRECONSTRUCTION_V3_PARAMETER_BYTES: usize = 4 * size_of::<f32>() + size_of::<i32>();
const ENLARGECANVAS_COMPATIBILITY_NAME: &str = "enlargecanvas";
const VIBRANCE_COMPATIBILITY_NAME: &str = "vibrance";
const BASECURVE_COMPATIBILITY_NAME: &str = "basecurve";
const HIGHPASS_COMPATIBILITY_NAME: &str = "highpass";
const TONECURVE_COMPATIBILITY_NAME: &str = "tonecurve";

// `src/develop/blend.h` assigns Lab=2, display RGB=3, and scene RGB=4.
// Native module initialization and commit replace NONE with these operation defaults.
const DEVELOP_BLEND_CS_LAB: i32 = 2;
const DEVELOP_BLEND_CS_RGB_DISPLAY: i32 = 3;
const DEVELOP_BLEND_CS_RGB_SCENE: i32 = 4;

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
    /// Core parameters are decoded, but a native process-owned state transition is not materialized.
    DeferredRuntimeState,
}

/// Actionable evidence explaining why a row was retained verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarktableHistoryDecodeFinding {
    /// Stable machine-readable classification.
    pub code: DarktableHistoryDecodeFindingCode,
    /// Bounded human-readable diagnostic.
    pub detail: String,
}

/// One decoded `AgX` core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedAgxHistoryStep {
    pub source: CompatHistoryStep,
    pub config: AgxConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: [u8; AGX_PARAMETER_BYTES_V7],
    pub migrated: bool,
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Levels core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedLevelsHistoryStep {
    pub source: CompatHistoryStep,
    pub config: LevelsConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: [u8; LEVELS_PARAMETER_BYTES_V2],
    pub migrated: bool,
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded RGB Levels core whose complete Darktable row is not executable yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRgbLevelsHistoryStep {
    pub source: CompatHistoryStep,
    pub config: RgbLevelsConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: [u8; RGBLEVELS_PARAMETER_BYTES],
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Color Mapping v1 core whose native blend/mask row remains pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorMappingHistoryStep {
    pub source: CompatHistoryStep,
    pub config: Box<ColorMappingConfig>,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: Vec<u8>,
    pub execution_blocker: DarktableHistoryDecodeFinding,
}

/// One decoded Color Transfer v1 core whose process-owned acquisition state remains pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedColorTransferHistoryStep {
    pub source: CompatHistoryStep,
    pub parameters: Box<ColorTransferParameters>,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: Vec<u8>,
    pub execution_blocker: DarktableHistoryDecodeFinding,
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

/// One Highpass row whose parameters and identity blend are executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedHighpassHistoryStep {
    pub source: CompatHistoryStep,
    pub config: HighpassConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters:
        [u8; rusttable_processing::operations::highpass::HIGHPASS_PARAMETER_BYTES],
    pub migrated: bool,
    pub execution_blocker: Option<DarktableHistoryDecodeFinding>,
}

/// One Tone Curve row whose parameters and identity blend are executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedToneCurveHistoryStep {
    pub source: CompatHistoryStep,
    pub config: ToneCurveConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters: [u8; rusttable_processing::operations::tonecurve::PARAMETER_BYTES],
    pub migrated: bool,
    pub execution_blocker: Option<DarktableHistoryDecodeFinding>,
}

/// One Base Curve row whose parameters and identity blend are executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBasecurveHistoryStep {
    pub source: CompatHistoryStep,
    pub config: BasecurveConfig,
    pub enabled: bool,
    pub source_version: u16,
    pub canonical_parameters:
        [u8; rusttable_processing::operations::basecurve::BASECURVE_V6_PARAMETER_BYTES],
    pub migrated: bool,
    pub execution_blocker: Option<DarktableHistoryDecodeFinding>,
}

/// Typed-but-pending result or a byte-preserving unsupported row.
#[derive(Debug, Clone, PartialEq)]
pub enum DarktableHistoryStepDecode {
    /// `AgX` parameters are decoded and normalized to v7 while blend/mask semantics remain pending.
    AgxPendingBlend(DecodedAgxHistoryStep),
    /// Levels parameters are decoded and normalized to v2 while blend/mask semantics remain pending.
    LevelsPendingBlend(DecodedLevelsHistoryStep),
    /// RGB Levels v1 parameters are decoded while blend/mask semantics remain pending.
    RgbLevelsPendingBlend(DecodedRgbLevelsHistoryStep),
    /// Color Mapping v1 parameters are decoded while blend/mask semantics remain pending.
    ColorMappingPendingBlend(Box<DecodedColorMappingHistoryStep>),
    /// Color Transfer v1 parameters are decoded while preview acquisition state remains pending.
    ColorTransferPendingRuntime(Box<DecodedColorTransferHistoryStep>),
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
    /// Basic Adjust core parameters are decoded, while the complete blend/mask
    /// and multi-instance row remains byte-preserved and non-executable.
    BasicAdjPendingBlend(DecodedBasicAdjHistoryStep),
    /// Channel Mixer core parameters are decoded, while the complete blend/mask
    /// and multi-instance row remains byte-preserved and non-executable.
    ChannelMixerPendingBlend(DecodedChannelMixerHistoryStep),
    /// Sharpen v1 core parameters are decoded, while the complete blend/mask
    /// and multi-instance row remains byte-preserved and non-executable.
    SharpenPendingBlend(DecodedSharpenHistoryStep),
    /// Soften v1 core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    SoftenPendingBlend(DecodedSoftenHistoryStep),
    /// Velvia core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    VelviaPendingBlend(DecodedVelviaHistoryStep),
    /// Vibrance core parameters are decoded, while blend/mask semantics remain
    /// explicitly non-executable.
    VibrancePendingBlend(DecodedVibranceHistoryStep),
    /// Highpass core parameters are decoded, while history-level eligibility or
    /// blend identity remains pending.
    HighpassPendingBlend(DecodedHighpassHistoryStep),
    /// Highpass is executable only after whole-history order validation.
    HighpassExecutable(DecodedHighpassHistoryStep),
    /// Tone Curve core parameters are decoded, while history-level eligibility or
    /// blend identity remains pending.
    ToneCurvePendingBlend(DecodedToneCurveHistoryStep),
    /// Tone Curve is executable only after whole-history order validation.
    ToneCurveExecutable(DecodedToneCurveHistoryStep),
    /// Base Curve core parameters are decoded, while history-level eligibility or
    /// blend identity remains pending.
    BasecurvePendingBlend(DecodedBasecurveHistoryStep),
    /// Base Curve is executable only after whole-history order validation.
    BasecurveExecutable(DecodedBasecurveHistoryStep),
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
        Some(AGX_COMPATIBILITY_ID) => decode_agx_history_step(step),
        Some(LEVELS_COMPATIBILITY_ID) => decode_levels_history_step(step),
        Some(RGBLEVELS_COMPATIBILITY_ID) => decode_rgblevels_history_step(step),
        Some(COLOR_MAPPING_COMPATIBILITY_ID) => decode_colormapping_history_step(step),
        Some(COLORTRANSFER_COMPATIBILITY_ID) => decode_colortransfer_history_step(step),
        Some(BLOOM_COMPATIBILITY_NAME) => decode_bloom_history_step(step),
        Some(COLOR_CONTRAST_COMPATIBILITY_NAME) => decode_colorcontrast_history_step(step),
        Some(COLORCORRECTION_COMPATIBILITY_NAME) => decode_colorcorrection_history_step(step),
        Some(COLORRECONSTRUCTION_COMPATIBILITY_NAME) => {
            decode_colorreconstruction_history_step(step)
        }
        Some(CROP_COMPATIBILITY_NAME) => decode_crop_history_step(step),
        Some(ENLARGECANVAS_COMPATIBILITY_NAME) => decode_enlargecanvas_history_step(step),
        Some(COLORZONES_COMPATIBILITY_NAME) => decode_colorzones_history_step(step),
        Some(BASICADJ_COMPATIBILITY_NAME) => decode_basicadj_import_history_step(step),
        Some(CHANNELMIXER_COMPATIBILITY_NAME) => decode_channelmixer_import_history_step(step),
        Some(SHARPEN_COMPATIBILITY_NAME) => decode_sharpen_import_history_step(step),
        Some(SOFTEN_COMPATIBILITY_NAME) => decode_soften_import_history_step(step),
        Some(VELVIA_COMPATIBILITY_NAME) => decode_velvia_history_step(step),
        Some(VIBRANCE_COMPATIBILITY_NAME) => decode_vibrance_history_step(step),
        Some(BASECURVE_COMPATIBILITY_NAME) => decode_basecurve_history_step(step, false),
        Some(HIGHPASS_COMPATIBILITY_NAME) => decode_highpass_history_step(step, false),
        Some(TONECURVE_COMPATIBILITY_NAME) => decode_tonecurve_history_step(step, false),
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

fn decode_agx_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "AgX") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match AgxHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(rusttable_processing::operations::agx::AgxCodecError::UnsupportedVersion(_)) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable AgX v{source_version} parameters remain unsupported"),
            );
        }
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable AgX v{source_version} parameters could not be decoded: {error}"),
            );
        }
    };
    let parameters = history.current();
    let config = match AgxConfig::new(parameters) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable AgX v{source_version} parameters are not executable: {error}"),
            );
        }
    };
    DarktableHistoryStepDecode::AgxPendingBlend(DecodedAgxHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        migrated: source_version != AGX_SCHEMA_VERSION,
        execution_blocker: pending_blend_finding(step, "AgX"),
    })
}

fn decode_levels_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Levels") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match LevelsHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Levels v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = match history.current() {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable Levels v{source_version} parameters remain opaque: {error}"),
            );
        }
    };
    let config = match LevelsConfig::new(parameters) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Levels v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };
    DarktableHistoryStepDecode::LevelsPendingBlend(DecodedLevelsHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        migrated: source_version != LEVELS_SCHEMA_VERSION,
        execution_blocker: pending_blend_finding(step, "Levels"),
    })
}

fn decode_rgblevels_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "RGB Levels") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match RgbLevelsHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable RGB Levels v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = match history.current() {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable RGB Levels v{source_version} parameters remain opaque: {error}"),
            );
        }
    };
    let config = match RgbLevelsConfig::new(parameters) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable RGB Levels v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };
    DarktableHistoryStepDecode::RgbLevelsPendingBlend(DecodedRgbLevelsHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        execution_blocker: pending_blend_finding(step, "RGB Levels"),
    })
}

fn decode_colormapping_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Mapping") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match ColorMappingHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Mapping v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = match history.current() {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!(
                    "Darktable Color Mapping v{source_version} parameters remain opaque: {error}"
                ),
            );
        }
    };
    let canonical_parameters = parameters.to_bytes();
    let config = match ColorMappingConfig::new(parameters) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Mapping v{source_version} parameters are not executable: {error}"
                ),
            );
        }
    };
    DarktableHistoryStepDecode::ColorMappingPendingBlend(Box::new(DecodedColorMappingHistoryStep {
        source: step.clone(),
        config: Box::new(config),
        enabled,
        source_version,
        canonical_parameters,
        execution_blocker: pending_blend_finding(step, "Color Mapping"),
    }))
}

fn decode_colortransfer_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Color Transfer") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    if source_version != COLORTRANSFER_SCHEMA_VERSION {
        return preserved(
            step,
            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
            format!("Darktable Color Transfer v{source_version} parameters remain opaque"),
        );
    }
    let parameters = match ColorTransferParameters::from_bytes(&step.operation_params.bytes) {
        Ok(parameters) => parameters,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Color Transfer v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    if let Err(error) = parameters
        .plan(rusttable_processing::RasterDimensions::new(1, 1).expect("unit dimensions are valid"))
    {
        return preserved(
            step,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
            format!(
                "Darktable Color Transfer v{source_version} parameters are not executable: {error}"
            ),
        );
    }
    let canonical_parameters = parameters.to_bytes();
    DarktableHistoryStepDecode::ColorTransferPendingRuntime(Box::new(
        DecodedColorTransferHistoryStep {
            source: step.clone(),
            parameters: Box::new(parameters),
            enabled,
            source_version,
            canonical_parameters,
            execution_blocker: DarktableHistoryDecodeFinding {
                code: DarktableHistoryDecodeFindingCode::DeferredRuntimeState,
                detail: "Darktable Color Transfer parameters are decoded, but preview acquisition, the process-owned points stream, and deprecated UI lifecycle remain deferred"
                    .to_owned(),
            },
        },
    ))
}

fn pending_blend_finding(
    step: &CompatHistoryStep,
    operation_name: &str,
) -> DarktableHistoryDecodeFinding {
    DarktableHistoryDecodeFinding {
        code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
        detail: format!(
            "Darktable {operation_name} core parameters are decoded, but blend version {:?}, blend/mask bytes, and multi-instance semantics remain opaque",
            step.blend_version
        ),
    }
}

fn decode_basicadj_import_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    match decode_basicadj_history_step(step) {
        BasicAdjHistoryStepDecode::BasicAdjPendingBlend(decoded) => {
            DarktableHistoryStepDecode::BasicAdjPendingBlend(decoded)
        }
        BasicAdjHistoryStepDecode::Preserved { source, finding } => {
            DarktableHistoryStepDecode::Preserved {
                source,
                finding: DarktableHistoryDecodeFinding {
                    code: map_basicadj_finding_code(finding.code),
                    detail: finding.detail,
                },
            }
        }
    }
}

const fn map_basicadj_finding_code(
    code: BasicAdjHistoryDecodeFindingCode,
) -> DarktableHistoryDecodeFindingCode {
    match code {
        BasicAdjHistoryDecodeFindingCode::MissingModuleVersion => {
            DarktableHistoryDecodeFindingCode::MissingModuleVersion
        }
        BasicAdjHistoryDecodeFindingCode::InvalidModuleVersion => {
            DarktableHistoryDecodeFindingCode::InvalidModuleVersion
        }
        BasicAdjHistoryDecodeFindingCode::UnsupportedParameterVersion => {
            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
        }
        BasicAdjHistoryDecodeFindingCode::InvalidOperationParameters => {
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters
        }
        BasicAdjHistoryDecodeFindingCode::InvalidEnabledState => {
            DarktableHistoryDecodeFindingCode::InvalidEnabledState
        }
        BasicAdjHistoryDecodeFindingCode::OpaqueBlendSemantics => {
            DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
        }
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

fn decode_channelmixer_import_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    match decode_channelmixer_history_step(step) {
        ChannelMixerHistoryStepDecode::ChannelMixerPendingBlend(decoded) => {
            DarktableHistoryStepDecode::ChannelMixerPendingBlend(decoded)
        }
        ChannelMixerHistoryStepDecode::Preserved { source, finding } => {
            DarktableHistoryStepDecode::Preserved {
                source,
                finding: DarktableHistoryDecodeFinding {
                    code: match finding.code {
                        ChannelMixerHistoryDecodeFindingCode::MissingModuleVersion => {
                            DarktableHistoryDecodeFindingCode::MissingModuleVersion
                        }
                        ChannelMixerHistoryDecodeFindingCode::InvalidModuleVersion => {
                            DarktableHistoryDecodeFindingCode::InvalidModuleVersion
                        }
                        ChannelMixerHistoryDecodeFindingCode::UnsupportedParameterVersion => {
                            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
                        }
                        ChannelMixerHistoryDecodeFindingCode::InvalidOperationParameters => {
                            DarktableHistoryDecodeFindingCode::InvalidOperationParameters
                        }
                        ChannelMixerHistoryDecodeFindingCode::InvalidEnabledState => {
                            DarktableHistoryDecodeFindingCode::InvalidEnabledState
                        }
                        ChannelMixerHistoryDecodeFindingCode::OpaqueBlendSemantics => {
                            DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
                        }
                    },
                    detail: finding.detail,
                },
            }
        }
    }
}

fn decode_sharpen_import_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    match decode_sharpen_history_step(step) {
        SharpenHistoryStepDecode::SharpenPendingBlend(decoded) => {
            DarktableHistoryStepDecode::SharpenPendingBlend(decoded)
        }
        SharpenHistoryStepDecode::Preserved { source, finding } => {
            DarktableHistoryStepDecode::Preserved {
                source,
                finding: DarktableHistoryDecodeFinding {
                    code: match finding.code {
                        SharpenHistoryDecodeFindingCode::MissingModuleVersion => {
                            DarktableHistoryDecodeFindingCode::MissingModuleVersion
                        }
                        SharpenHistoryDecodeFindingCode::InvalidModuleVersion => {
                            DarktableHistoryDecodeFindingCode::InvalidModuleVersion
                        }
                        SharpenHistoryDecodeFindingCode::UnsupportedParameterVersion => {
                            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
                        }
                        SharpenHistoryDecodeFindingCode::InvalidOperationParameters => {
                            DarktableHistoryDecodeFindingCode::InvalidOperationParameters
                        }
                        SharpenHistoryDecodeFindingCode::InvalidEnabledState => {
                            DarktableHistoryDecodeFindingCode::InvalidEnabledState
                        }
                        SharpenHistoryDecodeFindingCode::OpaqueBlendSemantics => {
                            DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
                        }
                    },
                    detail: finding.detail,
                },
            }
        }
    }
}

fn decode_soften_import_history_step(step: &CompatHistoryStep) -> DarktableHistoryStepDecode {
    match decode_soften_history_step(step) {
        SoftenHistoryStepDecode::SoftenPendingBlend(decoded) => {
            DarktableHistoryStepDecode::SoftenPendingBlend(decoded)
        }
        SoftenHistoryStepDecode::Preserved { source, finding } => {
            DarktableHistoryStepDecode::Preserved {
                source,
                finding: DarktableHistoryDecodeFinding {
                    code: match finding.code {
                        SoftenHistoryDecodeFindingCode::MissingModuleVersion => {
                            DarktableHistoryDecodeFindingCode::MissingModuleVersion
                        }
                        SoftenHistoryDecodeFindingCode::InvalidModuleVersion => {
                            DarktableHistoryDecodeFindingCode::InvalidModuleVersion
                        }
                        SoftenHistoryDecodeFindingCode::UnsupportedParameterVersion => {
                            DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion
                        }
                        SoftenHistoryDecodeFindingCode::InvalidOperationParameters => {
                            DarktableHistoryDecodeFindingCode::InvalidOperationParameters
                        }
                        SoftenHistoryDecodeFindingCode::InvalidEnabledState => {
                            DarktableHistoryDecodeFindingCode::InvalidEnabledState
                        }
                        SoftenHistoryDecodeFindingCode::OpaqueBlendSemantics => {
                            DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics
                        }
                    },
                    detail: finding.detail,
                },
            }
        }
    }
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

fn decode_highpass_history_step(
    step: &CompatHistoryStep,
    allow_execute: bool,
) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Highpass") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match HighpassHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(HighpassHistory::V1(parameters)) => parameters,
        Ok(HighpassHistory::Opaque { .. }) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::UnsupportedParameterVersion,
                format!("Darktable Highpass v{source_version} parameters remain unsupported"),
            );
        }
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Highpass v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let config = match HighpassConfig::try_from(history) {
        Ok(config) => config,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!("Darktable Highpass parameters are not executable: {error}"),
            );
        }
    };
    let decoded = DecodedHighpassHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: HighpassParametersV1::new(config.sharpness(), config.contrast())
            .to_bytes(),
        migrated: source_version != HIGHPASS_SCHEMA_VERSION,
        execution_blocker: (!allow_execute
            || source_version != HIGHPASS_SCHEMA_VERSION
            || !target_row_blend_eligible(step, HIGHPASS_COMPATIBILITY_NAME))
        .then(|| pending_target_blend_finding(step, HIGHPASS_COMPATIBILITY_NAME)),
    };
    if decoded.execution_blocker.is_none() {
        DarktableHistoryStepDecode::HighpassExecutable(decoded)
    } else {
        DarktableHistoryStepDecode::HighpassPendingBlend(decoded)
    }
}

fn decode_tonecurve_history_step(
    step: &CompatHistoryStep,
    allow_execute: bool,
) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Tone Curve") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match ToneCurveHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Tone Curve v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters = history.current().clone();
    if let Err(error) = parameters.validate() {
        return preserved(
            step,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
            format!("Darktable Tone Curve parameters are not executable: {error}"),
        );
    }
    if let Err(error) =
        rusttable_processing::operations::tonecurve::compile_parameters(&parameters, None)
    {
        return preserved(
            step,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
            format!("Darktable Tone Curve parameters cannot be compiled: {error}"),
        );
    }
    let config = ToneCurveConfig::new(parameters.clone());
    let decoded = DecodedToneCurveHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        migrated: source_version != TONECURVE_SCHEMA_VERSION,
        execution_blocker: (!allow_execute
            || source_version != TONECURVE_SCHEMA_VERSION
            || !target_row_blend_eligible(step, TONECURVE_COMPATIBILITY_NAME))
        .then(|| pending_target_blend_finding(step, TONECURVE_COMPATIBILITY_NAME)),
    };
    if decoded.execution_blocker.is_none() {
        DarktableHistoryStepDecode::ToneCurveExecutable(decoded)
    } else {
        DarktableHistoryStepDecode::ToneCurvePendingBlend(decoded)
    }
}

fn decode_basecurve_history_step(
    step: &CompatHistoryStep,
    allow_execute: bool,
) -> DarktableHistoryStepDecode {
    let (source_version, enabled) = match decoded_row_header(step, "Base Curve") {
        Ok(header) => header,
        Err(finding) => return preserved(step, finding.code, finding.detail),
    };
    let history = match BasecurveHistory::decode(source_version, &step.operation_params.bytes) {
        Ok(history) => history,
        Err(error) => {
            return preserved(
                step,
                DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
                format!(
                    "Darktable Base Curve v{source_version} parameters could not be decoded: {error}"
                ),
            );
        }
    };
    let parameters: BasecurveParameters = history.current();
    let config = BasecurveConfig::new(parameters);
    if let Err(error) =
        rusttable_processing::operations::basecurve::BasecurvePlan::compile(parameters)
    {
        return preserved(
            step,
            DarktableHistoryDecodeFindingCode::InvalidOperationParameters,
            format!("Darktable Base Curve parameters are not executable: {error}"),
        );
    }
    let decoded = DecodedBasecurveHistoryStep {
        source: step.clone(),
        config,
        enabled,
        source_version,
        canonical_parameters: parameters.to_bytes(),
        migrated: source_version != BASECURVE_SCHEMA_VERSION,
        execution_blocker: (!allow_execute
            || source_version != BASECURVE_SCHEMA_VERSION
            || !target_row_blend_eligible(step, BASECURVE_COMPATIBILITY_NAME))
        .then(|| pending_target_blend_finding(step, BASECURVE_COMPATIBILITY_NAME)),
    };
    if decoded.execution_blocker.is_none() {
        DarktableHistoryStepDecode::BasecurveExecutable(decoded)
    } else {
        DarktableHistoryStepDecode::BasecurvePendingBlend(decoded)
    }
}

fn target_row_blend_eligible(step: &CompatHistoryStep, operation: &str) -> bool {
    let priority = step.multi_priority.unwrap_or(0);
    let effective_name = !step.multi_name.bytes.is_empty()
        && (priority > 0 || step.multi_name_hand_edited.unwrap_or(0) != 0);
    let identity_blend = match operation {
        BASECURVE_COMPATIBILITY_NAME => [DEVELOP_BLEND_CS_RGB_DISPLAY, DEVELOP_BLEND_CS_RGB_SCENE]
            .into_iter()
            .any(|blend_cst| {
                is_identity_blend_v14(&step.blend_params, step.blend_version, blend_cst)
            }),
        HIGHPASS_COMPATIBILITY_NAME | TONECURVE_COMPATIBILITY_NAME => {
            is_identity_blend_v14(&step.blend_params, step.blend_version, DEVELOP_BLEND_CS_LAB)
        }
        _ => return false,
    };
    matches!(step.enabled, EnabledState::Enabled)
        && step.selected
        && priority == 0
        && !effective_name
        && identity_blend
}

fn pending_target_blend_finding(
    step: &CompatHistoryStep,
    operation: &str,
) -> DarktableHistoryDecodeFinding {
    DarktableHistoryDecodeFinding {
        code: DarktableHistoryDecodeFindingCode::OpaqueBlendSemantics,
        detail: format!(
            "Darktable {operation} row is retained pending selected priority-zero single-instance metadata, native order proof, and exact v14 identity blend bytes (version {:?})",
            step.blend_version
        ),
    }
}

/// Decodes all rows while allowing the three promoted operations to become
/// executable only when the owning history proves their order and instance.
#[must_use]
pub fn decode_history_steps(history: &CompatHistory) -> Vec<DarktableHistoryStepDecode> {
    history
        .steps
        .iter()
        .map(|step| match step.operation.name.as_deref() {
            Some(BASECURVE_COMPATIBILITY_NAME) => decode_basecurve_history_step(
                step,
                target_history_is_executable(history, step, BASECURVE_COMPATIBILITY_NAME),
            ),
            Some(HIGHPASS_COMPATIBILITY_NAME) => decode_highpass_history_step(
                step,
                target_history_is_executable(history, step, HIGHPASS_COMPATIBILITY_NAME),
            ),
            Some(TONECURVE_COMPATIBILITY_NAME) => decode_tonecurve_history_step(
                step,
                target_history_is_executable(history, step, TONECURVE_COMPATIBILITY_NAME),
            ),
            _ => decode_history_step(step),
        })
        .collect()
}

fn target_history_is_executable(
    history: &CompatHistory,
    step: &CompatHistoryStep,
    operation: &str,
) -> bool {
    if !history.executable
        || !history.order_proven
        || !history.selection.active_rows.contains(&step.source)
        || !target_row_blend_eligible(step, operation)
    {
        return false;
    }
    let instances = history
        .instances
        .iter()
        .filter(|instance| instance.operation.name.as_deref() == Some(operation))
        .collect::<Vec<_>>();
    instances.len() == 1
        && instances[0].id == step.instance_id
        && history.operation_order.contains(&step.instance_id)
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
