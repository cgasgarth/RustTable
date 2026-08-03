use crate::operations::{
    agx::AgxConfig,
    basicadj::BasicAdjConfig,
    bloom::BloomConfig,
    channelmixer::ChannelMixerConfig,
    clipping::ClippingConfig,
    colorcontrast::ColorContrastConfig,
    colorcorrection::ColorCorrectionConfig,
    colorin::ColorInConfig,
    colormapping::ColorMappingConfig,
    colorout::ColorOutConfig,
    colorreconstruction::ColorReconstructionConfig,
    colortransfer::ColorTransferParameters,
    colorzones::ColorZonesPlan,
    crop::CropConfig,
    dither::DitherConfig,
    enlargecanvas::EnlargeCanvasConfig,
    finalscale::FinalScaleConfig,
    flip::{FlipConfig, FlipMode, OrientationBits},
    graduatednd::GraduatedNdConfig,
    highlights::HighlightsConfig,
    invert::InvertConfig,
    lenscorrection::LensCorrectionConfig,
    levels::LevelsConfig,
    perspective::PerspectiveConfig,
    primaries::PrimariesConfig,
    rasterfile::RasterFileParametersV1,
    relight::RelightConfig,
    rgblevels::RgbLevelsConfig,
    rotatepixels::{RotatePixelsConfig, RotatePixelsParametersV1},
    scalepixels::ScalePixelsConfig,
    shadhi::ShadhiConfig,
    sharpen::SharpenConfig,
    soften::SoftenConfig,
    spots::SpotsParametersV2,
    temperature::{TemperatureConfig, WhiteBalanceSource},
    velvia::VelviaConfig,
    vibrance::VibranceConfig,
    vignette::VignetteConfig,
};
use crate::{FiniteF32, RasterDimensions, ScalarNarrowingError};
use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};

mod agx;
mod basicadj;
mod channelmixer;
mod compat;
mod error;
mod geometry;
mod parameters;
pub use geometry::{
    compile_clipping, compile_enlargecanvas, compile_finalscale, compile_lenscorrection,
    compile_perspective, compile_rasterfile, compile_scalepixels,
};
mod censorize;
mod clahe;
mod colorcontrast;
mod colorcorrection;
mod colormapping;
mod colortransfer;
pub mod colorzones;
mod defringe;
mod effects;
mod grain;
mod legacy;
mod levels;
mod masks;
mod retouch;
mod rgblevels;
mod spatial;
mod spots;
mod text;
mod velvia;
mod vibrance;
pub use agx::compile_agx;
pub use basicadj::compile_basicadj;
pub use censorize::compile_censorize;
pub use channelmixer::compile_channelmixer;
pub use clahe::compile_clahe;
pub use colorcontrast::compile_colorcontrast;
pub use colorcorrection::compile_colorcorrection;
pub use colormapping::compile_colormapping;
pub use colortransfer::compile_colortransfer;
pub use colorzones::compile_colorzones;
pub use compat::{compile_dither, compile_invert};
pub use defringe::compile_defringe;
pub use effects::{compile_bloom, compile_soften};
pub use error::compile_opacity;
pub use grain::compile_grain;
pub use legacy::{compile_relight, compile_shadhi};
pub use levels::compile_levels;
pub use parameters::{
    compile_scalar, compile_scalar_parameter, parameter_bool_default, parameter_f32_array,
    parameter_f64, parameter_integer, parameter_u32,
};
pub use rgblevels::compile_rgblevels;
pub use spatial::{compile_graduatednd, compile_vignette};
use text::{invalid_parameters, optional_parameter_text, parameter_bool, parameter_text};
pub use velvia::compile_velvia;
pub use vibrance::compile_vibrance;
const EXPOSURE_PARAMETER: &str = "stops";
const EXPOSURE_BLACK_PARAMETER: &str = "black";
const LINEAR_OFFSET_PARAMETER: &str = "value";
const RGB_GAIN_PARAMETERS: [&str; 3] = ["red", "green", "blue"];
const CROP_PARAMETERS: [&str; 6] = ["cx", "cy", "cw", "ch", "ratio_n", "ratio_d"];
const FLIP_PARAMETERS: [&str; 2] = ["mode", "orientation"];
const ROTATEPIXELS_PARAMETERS: [&str; 3] = ["rx", "ry", "angle"];
/// Typed operations that execute on decoded source data before the RGB graph.
///
/// These values are intentionally separate from [`ProcessingOperationKind`]:
/// the generic operation registry and its linear-RGB executor cannot represent
/// a single-plane RAW buffer without inventing an image predicate.
pub use crate::operations::rawprepare::RawPrepareSourceOperation as SourceProcessingOperation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingOperation {
    pub(crate) operation_id: OperationId,
    pub(crate) enabled: bool,
    pub(crate) opacity: FiniteF32,
    pub(crate) kind: ProcessingOperationKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProcessingOperationKind {
    Agx {
        config: AgxConfig,
    },
    BasicAdj {
        config: BasicAdjConfig,
    },
    Exposure {
        stops: FiniteF32,
        black: FiniteF32,
    },
    LinearOffset {
        value: FiniteF32,
    },
    RgbGain {
        red: FiniteF32,
        green: FiniteF32,
        blue: FiniteF32,
    },
    Invert {
        config: InvertConfig,
    },
    Dither {
        config: DitherConfig,
    },
    Grain {
        config: crate::operations::grain::GrainConfig,
    },
    Crop {
        config: CropConfig,
    },
    Flip {
        config: FlipConfig,
    },
    RotatePixels {
        config: RotatePixelsConfig,
    },
    ScalePixels {
        config: ScalePixelsConfig,
    },
    FinalScale {
        config: FinalScaleConfig,
    },
    EnlargeCanvas {
        config: EnlargeCanvasConfig,
    },
    Perspective {
        config: PerspectiveConfig,
    },
    Clipping {
        config: ClippingConfig,
    },
    RasterFile {
        config: Box<RasterFileParametersV1>,
    },
    LensCorrection {
        config: LensCorrectionConfig,
    },
    Levels {
        config: LevelsConfig,
    },
    RgbLevels {
        config: RgbLevelsConfig,
    },
    ColorTransfer {
        parameters: Box<ColorTransferParameters>,
    },
    ColorMapping {
        config: Box<ColorMappingConfig>,
    },
    Highlights {
        config: HighlightsConfig,
    },
    ColorReconstruction {
        config: ColorReconstructionConfig,
    },
    ColorIn {
        config: ColorInConfig,
    },
    Primaries {
        config: PrimariesConfig,
    },
    ColorOut {
        config: ColorOutConfig,
    },
    ColorCorrection {
        config: ColorCorrectionConfig,
    },
    ColorContrast {
        config: ColorContrastConfig,
    },
    ChannelMixer {
        config: ChannelMixerConfig,
    },
    ColorZones {
        plan: ColorZonesPlan,
    },
    Temperature {
        config: TemperatureConfig,
    },
    Bloom {
        config: BloomConfig,
    },
    Soften {
        config: SoftenConfig,
    },
    Relight {
        config: RelightConfig,
    },
    Velvia {
        config: VelviaConfig,
    },
    Vibrance {
        config: VibranceConfig,
    },
    Shadhi {
        config: ShadhiConfig,
    },
    Sharpen {
        config: SharpenConfig,
    },
    Vignette {
        config: VignetteConfig,
    },
    GraduatedNd {
        config: GraduatedNdConfig,
    },
    Censorize {
        config: crate::operations::censorize::CensorizeConfig,
    },
    Defringe {
        config: crate::operations::defringe::DefringeConfig,
    },
    Clahe {
        config: crate::operations::clahe::ClaheConfig,
    },
    MaskManager {
        config: crate::operations::mask_manager::MaskManagerParameters,
    },
    Retouch {
        config: crate::operations::retouch::RetouchParameters,
    },
    Spots {
        parameters: Box<SpotsParametersV2>,
    },
    Liquify {
        config: crate::operations::liquify::LiquifyConfig,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationCompileError {
    UnsupportedOperationKey {
        operation_id: OperationId,
        key: OperationKey,
    },
    MissingParameter {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    UnexpectedParameter {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    WrongParameterType {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    ScalarNarrowingOverflow {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    ScalarNarrowingUnderflow {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    OpacityNarrowingUnderflow {
        operation_id: OperationId,
    },
    NegativeParameter {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    InvalidParameters {
        operation_id: OperationId,
        key: OperationKey,
        reason: String,
    },
}

impl ProcessingOperation {
    /// Compiles one validated core operation into closed processing data.
    ///
    /// # Errors
    ///
    /// Returns a typed [`OperationCompileError`] when the operation key or its
    /// exact schema is not supported by the processing boundary.
    pub fn compile(operation: &Operation) -> Result<Self, OperationCompileError> {
        Self::prepare(operation).map(|prepared| prepared.operation().clone())
    }

    pub(crate) fn prepare(
        operation: &Operation,
    ) -> Result<crate::registry::PreparedCpuOperation, OperationCompileError> {
        crate::registry::builtin_registry()
            .prepare_cpu(operation)
            .map_err(|error| match error {
                crate::registry::RegistryLookupError::UnknownOperation(key) => {
                    OperationCompileError::UnsupportedOperationKey {
                        operation_id: operation.id(),
                        key,
                    }
                }
                crate::registry::RegistryLookupError::Factory { source, .. } => match *source {
                    crate::registry::FactoryError::Operation(source) => source,
                    crate::registry::FactoryError::DescriptorMismatch { .. } => {
                        OperationCompileError::UnsupportedOperationKey {
                            operation_id: operation.id(),
                            key: operation.key().clone(),
                        }
                    }
                },
            })
    }

    pub(crate) fn compile_exposure(operation: &Operation) -> Result<Self, OperationCompileError> {
        let stops_parameter = ParameterName::new(EXPOSURE_PARAMETER).expect("schema name");
        if operation.parameter(&stops_parameter).is_none() {
            return Err(OperationCompileError::MissingParameter {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: stops_parameter,
            });
        }
        if let Some((unexpected, _)) = operation.parameters().find(|(name, _)| {
            name.as_str() != EXPOSURE_PARAMETER && name.as_str() != EXPOSURE_BLACK_PARAMETER
        }) {
            return Err(OperationCompileError::UnexpectedParameter {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: unexpected.clone(),
            });
        }
        let stops = compile_scalar_parameter(operation, EXPOSURE_PARAMETER)?;
        let black = operation
            .parameter(&ParameterName::new(EXPOSURE_BLACK_PARAMETER).expect("schema name"))
            .map_or_else(
                || Ok(FiniteF32::new(0.0).expect("zero is finite")),
                |_| compile_scalar_parameter(operation, EXPOSURE_BLACK_PARAMETER),
            )?;
        let opacity = compile_opacity(operation)?;
        Ok(Self {
            operation_id: operation.id(),
            enabled: operation.is_enabled(),
            opacity,
            kind: ProcessingOperationKind::Exposure { stops, black },
        })
    }
    pub(crate) fn compile_agx(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_agx(operation)
    }
    pub(crate) fn compile_basicadj(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_basicadj(operation)
    }
    pub(crate) fn compile_linear_offset(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_scalar(operation, LINEAR_OFFSET_PARAMETER, |value| {
            ProcessingOperationKind::LinearOffset { value }
        })
    }
    pub(crate) fn compile_rgb_gain(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_rgb_gain(operation)
    }
    pub(crate) fn compile_invert(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_invert(operation)
    }
    pub(crate) fn compile_dither(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_dither(operation)
    }
    pub(crate) fn compile_grain(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_grain(operation)
    }
    pub(crate) fn compile_censorize(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_censorize(operation)
    }
    pub(crate) fn compile_defringe(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_defringe(operation)
    }
    pub(crate) fn compile_mask_manager(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        masks::compile_mask_manager(operation)
    }
    pub(crate) fn compile_retouch(operation: &Operation) -> Result<Self, OperationCompileError> {
        retouch::compile_retouch(operation)
    }
    pub(crate) fn compile_spots(operation: &Operation) -> Result<Self, OperationCompileError> {
        spots::compile_spots(operation)
    }
    pub(crate) fn compile_liquify(operation: &Operation) -> Result<Self, OperationCompileError> {
        let parameter = ParameterName::new("payload").expect("schema name");
        let payload = operation.parameter(&parameter).ok_or_else(|| {
            OperationCompileError::MissingParameter {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: parameter.clone(),
            }
        })?;
        let rusttable_core::ParameterValue::Text(payload) = payload else {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        };
        let config = crate::operations::liquify::LiquifyConfig::from_hex(payload.as_str())
            .map_err(|error| OperationCompileError::InvalidParameters {
                operation_id: operation.id(),
                key: operation.key().clone(),
                reason: error.to_string(),
            })?;
        let opacity = compile_opacity(operation)?;
        Ok(Self {
            operation_id: operation.id(),
            enabled: operation.is_enabled(),
            opacity,
            kind: ProcessingOperationKind::Liquify { config },
        })
    }
    pub(crate) fn compile_levels(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_levels(operation)
    }
    pub(crate) fn compile_rgblevels(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_rgblevels(operation)
    }
    pub(crate) fn compile_colortransfer(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_colortransfer(operation)
    }
    pub(crate) fn compile_colormapping(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_colormapping(operation)
    }
    pub(crate) fn compile_relight(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_relight(operation)
    }
    pub(crate) fn compile_velvia(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_velvia(operation)
    }
    pub(crate) fn compile_vibrance(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_vibrance(operation)
    }
    pub(crate) fn compile_shadhi(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_shadhi(operation)
    }
    pub(crate) fn compile_sharpen(operation: &Operation) -> Result<Self, OperationCompileError> {
        reject_unexpected(operation, &["radius", "amount", "threshold"])?;
        let config = SharpenConfig::new(
            parameter_f32(
                operation,
                "radius",
                f64::from(crate::operations::sharpen::SHARPEN_DEFAULT_RADIUS),
            )?,
            parameter_f32(
                operation,
                "amount",
                f64::from(crate::operations::sharpen::SHARPEN_DEFAULT_AMOUNT),
            )?,
            parameter_f32(
                operation,
                "threshold",
                f64::from(crate::operations::sharpen::SHARPEN_DEFAULT_THRESHOLD),
            )?,
        )
        .map_err(|error| invalid_parameters(operation, error))?;
        Ok(Self {
            operation_id: operation.id(),
            enabled: operation.is_enabled(),
            opacity: compile_opacity(operation)?,
            kind: ProcessingOperationKind::Sharpen { config },
        })
    }
    pub(crate) fn compile_highlights(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_highlights(operation)
    }

    pub(crate) fn compile_color_reconstruction(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_color_reconstruction(operation)
    }
    pub(crate) fn compile_colorin(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_colorin(operation)
    }

    pub(crate) fn compile_primaries(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_primaries(operation)
    }
    pub(crate) fn compile_colorout(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_colorout(operation)
    }

    pub(crate) fn compile_colorcorrection(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_colorcorrection(operation)
    }
    pub(crate) fn compile_colorcontrast(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_colorcontrast(operation)
    }
    pub(crate) fn compile_channelmixer(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_channelmixer(operation)
    }
    pub(crate) fn compile_colorzones(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_colorzones(operation)
    }
    pub(crate) fn compile_temperature(
        operation: &Operation,
    ) -> Result<Self, OperationCompileError> {
        compile_temperature(operation)
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    #[must_use]
    pub const fn opacity(&self) -> FiniteF32 {
        self.opacity
    }

    #[must_use]
    pub const fn kind(&self) -> &ProcessingOperationKind {
        &self.kind
    }

    /// Returns whether the registry contract requires full-image analysis.
    ///
    /// The executor uses this contract for tile scheduling so analysis
    /// operations observe the same full raster regardless of tile boundaries.
    #[must_use]
    pub fn requires_full_image_analysis(&self) -> bool {
        let descriptor_id = crate::registry::operation_descriptor_for(self);
        crate::registry::builtin_registry()
            .definition(descriptor_id.rust_id.as_str())
            .is_some_and(|definition| {
                definition
                    .descriptor()
                    .flags
                    .contains(crate::descriptor::OperationFlags::ANALYSIS)
            })
    }

    /// Resolves the source-derived per-edge neighborhood required at one
    /// immutable ROI/input scale pair.
    ///
    /// Point and full-frame operations return zero. Neighborhood operations
    /// quantize their support from committed parameters rather than relying on
    /// a misleading static descriptor overlap.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-capability error when finite positive scale
    /// evidence cannot resolve the operation's committed neighborhood.
    pub fn neighborhood_overlap_pixels(
        &self,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_iscale: f32,
    ) -> Result<u32, crate::operations::OperationExecutionError> {
        match self.kind() {
            ProcessingOperationKind::Sharpen { config } => {
                crate::operations::sharpen::tiling::SharpenTilingPlan::from_committed_radius(
                    config.commit().radius(),
                    roi_scale,
                    piece_iscale,
                )
                .map(crate::operations::sharpen::tiling::SharpenTilingPlan::overlap)
                .map_err(|_| {
                    crate::operations::OperationExecutionError::UnsupportedCapability(
                        "sharpen could not resolve its dynamic neighborhood overlap",
                    )
                })
            }
            ProcessingOperationKind::Soften { config } => {
                crate::operations::soften::SoftenPlan::overlap_pixels(
                    *config,
                    dimensions,
                    roi_scale,
                    piece_iscale,
                )
            }
            ProcessingOperationKind::ColorMapping { config } => {
                let scale = piece_iscale / roi_scale;
                crate::operations::colormapping::ColorMappingPlan::new_with_scale(
                    (**config).clone(),
                    dimensions,
                    scale,
                )
                .map_err(|_| {
                    crate::operations::OperationExecutionError::UnsupportedCapability(
                        "Color Mapping could not resolve its native ROI scale",
                    )
                })?
                .tiling(1)
                .map(|tiling| tiling.overlap)
                .map_err(|_| {
                    crate::operations::OperationExecutionError::UnsupportedCapability(
                        "Color Mapping could not resolve its dynamic neighborhood overlap",
                    )
                })
            }
            _ => Ok(0),
        }
    }
}

fn compile_rgb_gain(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    for required_name in RGB_GAIN_PARAMETERS {
        let required =
            ParameterName::new(required_name).expect("processing schema names are valid");
        if operation.parameter(&required).is_none() {
            return Err(OperationCompileError::MissingParameter {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
    }
    if let Some((unexpected, _)) = operation.parameters().find(|(name, _)| {
        !RGB_GAIN_PARAMETERS
            .iter()
            .any(|required| name.as_str() == *required)
    }) {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: unexpected.clone(),
        });
    }

    let red = compile_gain_parameter(operation, "red")?;
    let green = compile_gain_parameter(operation, "green")?;
    let blue = compile_gain_parameter(operation, "blue")?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::RgbGain { red, green, blue },
    })
}

fn compile_gain_parameter(
    operation: &Operation,
    parameter_name: &str,
) -> Result<FiniteF32, OperationCompileError> {
    let parameter = ParameterName::new(parameter_name).expect("processing schema names are valid");
    let value = match operation.parameter(&parameter) {
        Some(ParameterValue::Scalar(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
        None => unreachable!("required gain parameter was checked above"),
    };
    let value = match FiniteF32::try_from(value) {
        Ok(value) => value,
        Err(ScalarNarrowingError::Overflow) => {
            return Err(OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
        Err(ScalarNarrowingError::Underflow) => {
            return Err(OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
    };
    if value.get() < 0.0 {
        return Err(OperationCompileError::NegativeParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        });
    }
    Ok(value)
}

const HIGHLIGHTS_PARAMETERS: [&str; 12] = [
    "method",
    "blend_l",
    "blend_c",
    "strength",
    "clip",
    "noise_level",
    "iterations",
    "scales",
    "candidating",
    "combine",
    "recovery",
    "solid_color",
];

const COLOR_RECONSTRUCTION_PARAMETERS: [&str; 5] =
    ["threshold", "spatial", "range", "hue", "precedence"];
const COLORIN_PARAMETERS: [&str; 5] = [
    "input_profile",
    "working_profile",
    "intent",
    "normalize",
    "blue_mapping",
];
const PRIMARIES_PARAMETERS: [&str; 8] = [
    "achromatic_tint_hue",
    "achromatic_tint_purity",
    "red_hue",
    "red_purity",
    "green_hue",
    "green_purity",
    "blue_hue",
    "blue_purity",
];
const COLOROUT_PARAMETERS: [&str; 5] = [
    "profile",
    "intent",
    "black_point_compensation",
    "proof_profile",
    "gamut",
];
const TEMPERATURE_PARAMETERS: [&str; 14] = [
    "red",
    "green",
    "blue",
    "various",
    "preset",
    "source",
    "temperature",
    "tint",
    "stage",
    "camera_alias",
    "preset_id",
    "tuning",
    "source_table_revision",
    "temp_out",
];

fn compile_highlights(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &HIGHLIGHTS_PARAMETERS)?;
    let method = parameter_integer(operation, "method", 5.0)?;
    let scales = parameter_integer(operation, "scales", 6.0)?;
    let recovery = parameter_integer(operation, "recovery", 0.0)?;
    let iterations = parameter_integer(operation, "iterations", 30.0)?;
    let iterations = u16::try_from(iterations)
        .map_err(|_| invalid_parameters(operation, "iterations must be between 1 and 256"))?;
    let config = HighlightsConfig::new(
        crate::operations::highlights::HighlightsMethod::from_id(method)
            .map_err(|error| invalid_parameters(operation, error))?,
        parameter_f32(operation, "strength", 0.0)?,
        parameter_f32(operation, "clip", 1.0)?,
        parameter_f32(operation, "noise_level", 0.0)?,
        iterations,
        crate::operations::highlights::WaveletScale::new(
            u8::try_from(scales)
                .map_err(|_| invalid_parameters(operation, "scales must be between 0 and 11"))?,
        )
        .map_err(|error| invalid_parameters(operation, error))?,
        parameter_f32(operation, "candidating", 0.4)?,
        parameter_f32(operation, "combine", 2.0)?,
        crate::operations::highlights::RecoveryMode::from_id(recovery)
            .map_err(|error| invalid_parameters(operation, error))?,
        parameter_f32(operation, "solid_color", 0.0)?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::Highlights { config },
    })
}

fn compile_color_reconstruction(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &COLOR_RECONSTRUCTION_PARAMETERS)?;
    let precedence = parameter_integer(operation, "precedence", 0.0)?;
    let config = ColorReconstructionConfig::new(
        parameter_f32(operation, "threshold", 100.0)?,
        parameter_f32(operation, "spatial", 400.0)?,
        parameter_f32(operation, "range", 10.0)?,
        parameter_f32(operation, "hue", 0.66)?,
        crate::operations::colorreconstruction::ColorReconstructionPrecedence::from_id(precedence)
            .map_err(|error| invalid_parameters(operation, error))?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::ColorReconstruction { config },
    })
}

fn compile_colorin(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &COLORIN_PARAMETERS)?;
    let input_profile = parameter_text(operation, "input_profile")?;
    let working_profile = parameter_text(operation, "working_profile")?;
    let intent = parameter_integer(operation, "intent", 0.0)?;
    let normalization = parameter_integer(operation, "normalize", 0.0)?;
    let blue_mapping = parameter_bool(operation, "blue_mapping")?;
    let config = crate::operations::colorin::migrate(
        7,
        crate::operations::colorin::ColorInLegacyParameters {
            input_profile,
            working_profile: Some(working_profile),
            intent: i64::from(intent),
            normalization: i64::from(normalization),
            blue_mapping: Some(blue_mapping),
        },
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::ColorIn { config },
    })
}

fn compile_primaries(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &PRIMARIES_PARAMETERS)?;
    let config = PrimariesConfig::new(
        parameter_f32(operation, "achromatic_tint_hue", 0.0)?,
        parameter_f32(operation, "achromatic_tint_purity", 0.0)?,
        parameter_f32(operation, "red_hue", 0.0)?,
        parameter_f32(operation, "red_purity", 1.0)?,
        parameter_f32(operation, "green_hue", 0.0)?,
        parameter_f32(operation, "green_purity", 1.0)?,
        parameter_f32(operation, "blue_hue", 0.0)?,
        parameter_f32(operation, "blue_purity", 1.0)?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::Primaries { config },
    })
}

fn compile_colorout(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &COLOROUT_PARAMETERS)?;
    let config = crate::operations::colorout::migrate(
        7,
        &crate::operations::colorout::ColorOutLegacyParameters {
            output_profile: parameter_text(operation, "profile")?,
            intent: i64::from(parameter_integer(operation, "intent", 1.0)?),
            black_point_compensation: parameter_bool(operation, "black_point_compensation")?,
            proof_profile: Some(parameter_text(operation, "proof_profile")?),
            gamut: i64::from(parameter_integer(operation, "gamut", 0.0)?),
        },
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::ColorOut { config },
    })
}

fn compile_temperature(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &TEMPERATURE_PARAMETERS)?;
    let preset = parameter_integer(operation, "preset", 0.0)?;
    let source = match operation.parameter(&ParameterName::new("source").expect("static name")) {
        None => source_from_legacy_preset(operation, preset)?,
        Some(ParameterValue::Text(value)) => WhiteBalanceSource::parse(value.as_str())
            .map_err(|error| invalid_parameters(operation, error))?,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: ParameterName::new("source").expect("static name"),
            });
        }
    };
    let camera_alias = optional_parameter_text(operation, "camera_alias")?;
    let preset_id = optional_parameter_text(operation, "preset_id")?;
    let provenance = match (camera_alias, preset_id) {
        (Some(camera_alias), Some(preset_id))
            if !camera_alias.is_empty() && !preset_id.is_empty() =>
        {
            Some(
                crate::operations::temperature::PresetProvenance::new(
                    camera_alias,
                    preset_id,
                    i16::try_from(parameter_integer(operation, "tuning", 0.0)?)
                        .map_err(|_| invalid_parameters(operation, "tuning is out of range"))?,
                    u64::try_from(parameter_integer(operation, "source_table_revision", 0.0)?)
                        .map_err(|_| {
                            invalid_parameters(operation, "source table revision is negative")
                        })?,
                )
                .map_err(|error| invalid_parameters(operation, error))?,
            )
        }
        (None, None) => None,
        _ => {
            return Err(invalid_parameters(
                operation,
                "preset provenance is incomplete",
            ));
        }
    };
    let multipliers = crate::operations::temperature::ChannelMultipliers::from_coefficients([
        parameter_f32(operation, "red", 1.0)?,
        parameter_f32(operation, "green", 1.0)?,
        parameter_f32(operation, "blue", 1.0)?,
        parameter_f32(operation, "various", 1.0)?,
    ])
    .map_err(|error| invalid_parameters(operation, error))?;
    let stage = match optional_parameter_text(operation, "stage")? {
        Some(value) => crate::operations::temperature::WhiteBalanceStage::parse(&value)
            .map_err(|error| invalid_parameters(operation, error))?,
        None => crate::operations::temperature::WhiteBalanceStage::PreDemosaic,
    };
    let temperature_tint = if source == WhiteBalanceSource::TemperatureTint {
        crate::operations::temperature::TemperatureTint::new(
            parameter_f32(operation, "temperature", 4000.0)?,
            parameter_f32(operation, "tint", 1.0)?,
        )
        .ok()
    } else {
        None
    };
    let config = crate::operations::temperature::TemperatureConfig::with_details(
        multipliers,
        source,
        stage,
        temperature_tint,
        provenance,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::Temperature { config },
    })
}

fn source_from_legacy_preset(
    operation: &Operation,
    preset: i32,
) -> Result<WhiteBalanceSource, OperationCompileError> {
    match preset {
        -1 | 2 => Ok(WhiteBalanceSource::Custom),
        0 => Ok(WhiteBalanceSource::AsShot),
        1 => Ok(WhiteBalanceSource::Spot),
        3 => Ok(WhiteBalanceSource::CameraReference),
        4 => Ok(WhiteBalanceSource::DaylightReference),
        _ => Err(invalid_parameters(
            operation,
            "named white-balance presets require immutable preset provenance",
        )),
    }
}

pub fn compile_crop(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &CROP_PARAMETERS)?;
    let config = CropConfig::new(
        parameter_f32(operation, "cx", 0.0)?,
        parameter_f32(operation, "cy", 0.0)?,
        parameter_f32(operation, "cw", 1.0)?,
        parameter_f32(operation, "ch", 1.0)?,
        parameter_integer(operation, "ratio_n", -1.0)?,
        parameter_integer(operation, "ratio_d", -1.0)?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Crop { config },
    })
}

pub fn compile_flip(operation: &Operation) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &FLIP_PARAMETERS)?;
    let mode = match parameter_integer(operation, "mode", 0.0)? {
        0 => FlipMode::Automatic,
        1 => FlipMode::Explicit,
        _ => return Err(invalid_parameters(operation, "flip mode is invalid")),
    };
    let orientation_value = parameter_integer(operation, "orientation", 0.0)?;
    let orientation = OrientationBits::try_from(orientation_value)
        .map_err(|error| invalid_parameters(operation, error))?;
    let config =
        FlipConfig::new(mode, orientation).map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Flip { config },
    })
}

pub fn compile_rotatepixels(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &ROTATEPIXELS_PARAMETERS)?;
    let rx = parameter_u32(operation, "rx", 0)?;
    let ry = parameter_u32(operation, "ry", 0)?;
    let angle = parameter_f32(operation, "angle", 0.0)?;
    let config = RotatePixelsConfig::new(RotatePixelsParametersV1::new(rx, ry, angle))
        .map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::RotatePixels { config },
    })
}

pub fn reject_unexpected(
    operation: &Operation,
    allowed: &[&str],
) -> Result<(), OperationCompileError> {
    if let Some((parameter, _)) = operation
        .parameters()
        .find(|(name, _)| !allowed.iter().any(|allowed| *allowed == name.as_str()))
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: parameter.clone(),
        });
    }
    Ok(())
}

pub fn parameter_f32(
    operation: &Operation,
    name: &'static str,
    default: f64,
) -> Result<f32, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    let value = match operation.parameter(&parameter) {
        None => default,
        Some(ParameterValue::Integer(value)) => {
            let value = i32::try_from(*value).map_err(|_| {
                invalid_parameters(operation, format!("{name} must be an exact small integer"))
            })?;
            f64::from(value)
        }
        Some(ParameterValue::Scalar(value)) => value.get(),
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            });
        }
    };
    match FiniteF32::try_from(FiniteF64::new(value).expect("core scalar is finite")) {
        Ok(value) => Ok(value.get()),
        Err(ScalarNarrowingError::Overflow) => {
            Err(OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            })
        }
        Err(ScalarNarrowingError::Underflow) => {
            Err(OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter,
            })
        }
    }
}
