use crate::{OperationCompileError, ProcessingOperation, ProcessingOperationKind};
use rusttable_core::{Operation, ParameterName, ParameterValue};

pub(crate) fn compile_spots(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    let parameter = ParameterName::new("payload").expect("static spots parameter");
    let payload =
        operation
            .parameter(&parameter)
            .ok_or_else(|| OperationCompileError::MissingParameter {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: parameter.clone(),
            })?;
    let ParameterValue::Text(payload) = payload else {
        return Err(OperationCompileError::WrongParameterType {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter,
        });
    };
    let bytes = decode_hex(operation, payload)?;
    let parameters = match bytes.len() {
        crate::operations::spots::SPOTS_PARAMETER_BYTES_V1 => {
            let legacy = crate::operations::spots::SpotsParametersV1::from_bytes(&bytes)
                .map_err(|error| invalid(operation, error.to_string()))?;
            crate::operations::spots::migrate_v1_to_v2(&legacy)
                .map_err(|error| invalid(operation, error.to_string()))?
        }
        crate::operations::spots::SPOTS_PARAMETER_BYTES_V2 => {
            crate::operations::spots::SpotsParametersV2::from_bytes(&bytes)
                .map_err(|error| invalid(operation, error.to_string()))?
        }
        actual => {
            return Err(invalid(
                operation,
                format!("spots payload has {actual} decoded bytes"),
            ));
        }
    };
    let opacity = super::compile_opacity(operation)?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity,
        kind: ProcessingOperationKind::Spots {
            parameters: Box::new(parameters),
        },
    })
}

fn decode_hex(
    operation: &Operation,
    payload: &rusttable_core::ParameterText,
) -> Result<Vec<u8>, OperationCompileError> {
    let text = payload.as_str();
    if !text.len().is_multiple_of(2) {
        return Err(invalid(
            operation,
            "spots payload must be even-length hexadecimal",
        ));
    }
    let (pairs, remainder) = text.as_bytes().as_chunks::<2>();
    debug_assert!(
        remainder.is_empty(),
        "spots payload length was prevalidated"
    );
    pairs
        .iter()
        .map(|chunk| {
            let high = hex_digit(chunk[0]).ok_or_else(|| "invalid spots payload hex".to_owned());
            let low = hex_digit(chunk[1]).ok_or_else(|| "invalid spots payload hex".to_owned());
            high.and_then(|high| low.map(|low| (high << 4) | low))
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|reason| invalid(operation, reason))
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn invalid(operation: &Operation, reason: impl Into<String>) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: reason.into(),
    }
}
