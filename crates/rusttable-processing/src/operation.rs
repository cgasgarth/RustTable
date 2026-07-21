use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use std::fmt;

use crate::operations::{
    basicadj::BasicAdjConfig,
    bloom::BloomConfig,
    colorcorrection::ColorCorrectionConfig,
    colorin::ColorInConfig,
    colorout::ColorOutConfig,
    colorreconstruction::ColorReconstructionConfig,
    crop::CropConfig,
    dither::DitherConfig,
    enlargecanvas::EnlargeCanvasConfig,
    finalscale::FinalScaleConfig,
    flip::{FlipConfig, FlipMode, OrientationBits},
    graduatednd::GraduatedNdConfig,
    highlights::HighlightsConfig,
    invert::InvertConfig,
    lenscorrection::LensCorrectionConfig,
    perspective::PerspectiveConfig,
    primaries::PrimariesConfig,
    relight::RelightConfig,
    rotatepixels::{RotatePixelsConfig, RotatePixelsParametersV1},
    scalepixels::ScalePixelsConfig,
    shadhi::ShadhiConfig,
    soften::SoftenConfig,
    temperature::{TemperatureConfig, WhiteBalanceSource},
    vignette::VignetteConfig,
};
use crate::{FiniteF32, ScalarNarrowingError};

#[path = "operation_basicadj.rs"]
mod operation_basicadj;
#[path = "operation_compat.rs"]
mod operation_compat;
#[path = "operation_error.rs"]
mod operation_error;
#[path = "operation_geometry.rs"]
mod operation_geometry;
#[path = "operation_parameters.rs"]
mod operation_parameters;
pub(crate) use operation_geometry::{
    compile_enlargecanvas, compile_finalscale, compile_lenscorrection, compile_perspective,
    compile_scalepixels,
};
#[path = "operation_censorize.rs"]
mod operation_censorize;
#[path = "operation_clahe.rs"]
mod operation_clahe;
#[path = "operation_defringe.rs"]
mod operation_defringe;
#[path = "operation_effects.rs"]
mod operation_effects;
#[path = "operation_grain.rs"]
mod operation_grain;
#[path = "operation_legacy.rs"]
mod operation_legacy;
#[path = "operation_spatial.rs"]
mod operation_spatial;
pub(crate) use operation_basicadj::compile_basicadj;
pub(crate) use operation_censorize::compile_censorize;
pub(crate) use operation_clahe::compile_clahe;
pub(crate) use operation_compat::{compile_dither, compile_invert};
pub(crate) use operation_defringe::compile_defringe;
pub(crate) use operation_effects::{compile_bloom, compile_soften};
pub(crate) use operation_error::compile_opacity;
pub(crate) use operation_grain::compile_grain;
pub(crate) use operation_legacy::{compile_relight, compile_shadhi};
pub(crate) use operation_parameters::{
    compile_scalar, compile_scalar_parameter, parameter_f64, parameter_integer, parameter_u32,
};
pub(crate) use operation_spatial::{compile_graduatednd, compile_vignette};
const EXPOSURE_PARAMETER: &str = "stops";
const EXPOSURE_BLACK_PARAMETER: &str = "black";
const LINEAR_OFFSET_PARAMETER: &str = "value";
const RGB_GAIN_PARAMETERS: [&str; 3] = ["red", "green", "blue"];
const CROP_PARAMETERS: [&str; 6] = ["cx", "cy", "cw", "ch", "ratio_n", "ratio_d"];
const FLIP_PARAMETERS: [&str; 2] = ["mode", "orientation"];
const ROTATEPIXELS_PARAMETERS: [&str; 3] = ["rx", "ry", "angle"];
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingOperation {
    pub(crate) operation_id: OperationId,
    pub(crate) enabled: bool,
    pub(crate) opacity: FiniteF32,
    pub(crate) kind: ProcessingOperationKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProcessingOperationKind {
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
    LensCorrection {
        config: LensCorrectionConfig,
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
    Shadhi {
        config: ShadhiConfig,
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
        Ok(ProcessingOperation {
            operation_id: operation.id(),
            enabled: operation.is_enabled(),
            opacity,
            kind: ProcessingOperationKind::Exposure { stops, black },
        })
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

    pub(crate) fn compile_relight(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_relight(operation)
    }

    pub(crate) fn compile_shadhi(operation: &Operation) -> Result<Self, OperationCompileError> {
        compile_shadhi(operation)
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
    pub const fn opacity(&self) -> FiniteF32 {
        self.opacity
    }

    #[must_use]
    pub const fn kind(&self) -> &ProcessingOperationKind {
        &self.kind
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
const COLORCORRECTION_PARAMETERS: [&str; 10] = [
    "shadow_l",
    "shadow_a",
    "shadow_b",
    "highlight_l",
    "highlight_a",
    "highlight_b",
    "saturation",
    "tonal_range",
    "balance",
    "mode",
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

fn compile_colorcorrection(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &COLORCORRECTION_PARAMETERS)?;
    let config = crate::operations::colorcorrection::migrate(
        5,
        &crate::operations::colorcorrection::ColorCorrectionLegacyParameters {
            shadow: [
                parameter_f32(operation, "shadow_l", 0.0)?,
                parameter_f32(operation, "shadow_a", 0.0)?,
                parameter_f32(operation, "shadow_b", 0.0)?,
            ],
            highlight: [
                parameter_f32(operation, "highlight_l", 0.0)?,
                parameter_f32(operation, "highlight_a", 0.0)?,
                parameter_f32(operation, "highlight_b", 0.0)?,
            ],
            saturation: parameter_f32(operation, "saturation", 1.0)?,
            tonal_range: parameter_f32(operation, "tonal_range", 0.5)?,
            balance: parameter_f32(operation, "balance", 0.0)?,
            mode: i64::from(parameter_integer(operation, "mode", 0.0)?),
        },
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::ColorCorrection { config },
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

pub(crate) fn compile_crop(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
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

pub(crate) fn compile_flip(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
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

pub(crate) fn compile_rotatepixels(
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

pub(crate) fn reject_unexpected(
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

pub(crate) fn parameter_f32(
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

fn parameter_text(
    operation: &Operation,
    name: &'static str,
) -> Result<String, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        Some(ParameterValue::Text(value)) => Ok(value.as_str().to_owned()),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
        None => Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

fn optional_parameter_text(
    operation: &Operation,
    name: &'static str,
) -> Result<Option<String>, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        None => Ok(None),
        Some(ParameterValue::Text(value)) => Ok(Some(value.as_str().to_owned())),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

fn parameter_bool(
    operation: &Operation,
    name: &'static str,
) -> Result<bool, OperationCompileError> {
    let parameter = ParameterName::new(name).expect("static processing parameter");
    match operation.parameter(&parameter) {
        Some(ParameterValue::Bool(value)) => Ok(*value),
        Some(_) => Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
        None => Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        }),
    }
}

pub(crate) fn invalid_parameters<E: fmt::Display>(
    operation: &Operation,
    error: E,
) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: error.to_string(),
    }
}
