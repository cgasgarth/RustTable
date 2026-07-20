use std::fmt;

use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};

use crate::operations::{
    colorin::ColorInConfig, colorreconstruction::ColorReconstructionConfig,
    highlights::HighlightsConfig, primaries::PrimariesConfig,
};
use crate::{FiniteF32, ScalarNarrowingError};

const EXPOSURE_PARAMETER: &str = "stops";
const LINEAR_OFFSET_PARAMETER: &str = "value";
const RGB_GAIN_PARAMETERS: [&str; 3] = ["red", "green", "blue"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingOperation {
    operation_id: OperationId,
    enabled: bool,
    opacity: FiniteF32,
    kind: ProcessingOperationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProcessingOperationKind {
    Exposure {
        stops: FiniteF32,
    },
    LinearOffset {
        value: FiniteF32,
    },
    RgbGain {
        red: FiniteF32,
        green: FiniteF32,
        blue: FiniteF32,
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
        compile_scalar(operation, EXPOSURE_PARAMETER, |stops| {
            ProcessingOperationKind::Exposure { stops }
        })
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

fn compile_scalar<F>(
    operation: &Operation,
    required_name: &str,
    build: F,
) -> Result<ProcessingOperation, OperationCompileError>
where
    F: FnOnce(FiniteF32) -> ProcessingOperationKind,
{
    let required = ParameterName::new(required_name).expect("processing schema names are valid");
    if operation.parameter(&required).is_none() {
        return Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: required,
        });
    }
    if let Some((unexpected, _)) = operation
        .parameters()
        .find(|(name, _)| name.as_str() != required_name)
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: unexpected.clone(),
        });
    }
    let value = match operation.parameter(&required) {
        Some(ParameterValue::Scalar(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
        None => unreachable!("required parameter was checked above"),
    };
    let value = match FiniteF32::try_from(value) {
        Ok(value) => value,
        Err(ScalarNarrowingError::Overflow) => {
            return Err(OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
        Err(ScalarNarrowingError::Underflow) => {
            return Err(OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
    };
    let opacity = compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: build(value),
    })
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

fn reject_unexpected(operation: &Operation, allowed: &[&str]) -> Result<(), OperationCompileError> {
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

fn parameter_f32(
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

fn parameter_integer(
    operation: &Operation,
    name: &'static str,
    default: f64,
) -> Result<i32, OperationCompileError> {
    let value = parameter_f32(operation, name, default)?;
    if !value.is_finite()
        || value.fract() != 0.0
        || value < f32::from(i16::MIN)
        || value > f32::from(i16::MAX)
    {
        return Err(invalid_parameters(
            operation,
            format!("{name} must be an exact small integer"),
        ));
    }
    #[allow(clippy::cast_possible_truncation, reason = "range checked above")]
    Ok(value as i32)
}

fn invalid_parameters<E: fmt::Display>(operation: &Operation, error: E) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: error.to_string(),
    }
}

fn compile_opacity(operation: &Operation) -> Result<FiniteF32, OperationCompileError> {
    match FiniteF32::try_from(
        FiniteF64::new(operation.opacity().get()).expect("core opacity is finite"),
    ) {
        Ok(value) => Ok(value),
        Err(ScalarNarrowingError::Underflow) => {
            Err(OperationCompileError::OpacityNarrowingUnderflow {
                operation_id: operation.id(),
            })
        }
        Err(ScalarNarrowingError::Overflow) => unreachable!("checked opacity cannot overflow f32"),
    }
}

impl fmt::Display for OperationCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOperationKey { operation_id, key } => {
                write!(
                    formatter,
                    "operation {operation_id} has unsupported key {key}"
                )
            }
            Self::MissingParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} is missing parameter {parameter}"
            ),
            Self::UnexpectedParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has unexpected parameter {parameter}"
            ),
            Self::WrongParameterType {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has wrong type for parameter {parameter}"
            ),
            Self::ScalarNarrowingOverflow {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} overflows f32 parameter {parameter}"
            ),
            Self::ScalarNarrowingUnderflow {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} underflows f32 parameter {parameter}"
            ),
            Self::OpacityNarrowingUnderflow { operation_id } => {
                write!(
                    formatter,
                    "operation {operation_id} has opacity that underflows f32"
                )
            }
            Self::NegativeParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has negative parameter {parameter}"
            ),
            Self::InvalidParameters {
                operation_id,
                key,
                reason,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has invalid parameters: {reason}"
            ),
        }
    }
}

impl std::error::Error for OperationCompileError {}
