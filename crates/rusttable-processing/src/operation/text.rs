use rusttable_core::{Operation, ParameterName, ParameterValue};
use std::fmt;

use super::OperationCompileError;

pub(super) fn parameter_text(
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

pub(super) fn optional_parameter_text(
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

pub(super) fn parameter_bool(
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

pub(super) fn invalid_parameters<E: fmt::Display>(
    operation: &Operation,
    error: E,
) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: error.to_string(),
    }
}
