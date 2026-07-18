use std::fmt;

use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};

use crate::{FiniteF32, ScalarNarrowingError};

const EXPOSURE_KEY: &str = "rusttable.exposure";
const EXPOSURE_PARAMETER: &str = "stops";
const LINEAR_OFFSET_KEY: &str = "rusttable.linear_offset";
const LINEAR_OFFSET_PARAMETER: &str = "value";
const RGB_GAIN_KEY: &str = "rusttable.rgb_gain";
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
}

impl ProcessingOperation {
    /// Compiles one validated core operation into closed processing data.
    ///
    /// # Errors
    ///
    /// Returns a typed [`OperationCompileError`] when the operation key or its
    /// exact schema is not supported by the processing boundary.
    pub fn compile(operation: &Operation) -> Result<Self, OperationCompileError> {
        match operation.key().as_str() {
            EXPOSURE_KEY => compile_scalar(operation, EXPOSURE_PARAMETER, |stops| {
                ProcessingOperationKind::Exposure { stops }
            }),
            LINEAR_OFFSET_KEY => compile_scalar(operation, LINEAR_OFFSET_PARAMETER, |value| {
                ProcessingOperationKind::LinearOffset { value }
            }),
            RGB_GAIN_KEY => compile_rgb_gain(operation),
            _ => Err(OperationCompileError::UnsupportedOperationKey {
                operation_id: operation.id(),
                key: operation.key().clone(),
            }),
        }
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
        }
    }
}

impl std::error::Error for OperationCompileError {}
