use rusttable_core::{Operation, ParameterName, ParameterValue};

use crate::operations::colortransfer::{
    COLORTRANSFER_NATIVE_PARAMETER_BYTES, ColorTransferParameters,
};
use crate::operations::common::{decode_native_payload_chunks, encode_native_payload_chunks};

use super::{OperationCompileError, ProcessingOperation, ProcessingOperationKind, compile_opacity};

const PAYLOAD_CHUNKS: usize = COLORTRANSFER_NATIVE_PARAMETER_BYTES.div_ceil(2_048);

pub(crate) fn compile_colortransfer(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    let defaults = encode_native_payload_chunks(&ColorTransferParameters::default().to_bytes());
    let chunks = payload_chunks(operation, &defaults)?;
    let chunk_refs = chunks.iter().map(String::as_str).collect::<Vec<_>>();
    let bytes = decode_native_payload_chunks(&chunk_refs, COLORTRANSFER_NATIVE_PARAMETER_BYTES)
        .map_err(|error| invalid(operation, &error))?;
    let parameters =
        ColorTransferParameters::from_bytes(&bytes).map_err(|error| invalid(operation, &error))?;
    parameters
        .plan(crate::RasterDimensions::new(1, 1).expect("unit dimensions"))
        .map_err(|error| invalid(operation, &error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::ColorTransfer {
            parameters: Box::new(parameters),
        },
    })
}

fn payload_chunks(
    operation: &Operation,
    defaults: &[String],
) -> Result<Vec<String>, OperationCompileError> {
    debug_assert_eq!(defaults.len(), PAYLOAD_CHUNKS);
    let expected = (0..defaults.len())
        .map(|index| format!("payload_{index}"))
        .collect::<Vec<_>>();
    if let Some((name, _)) = operation
        .parameters()
        .find(|(name, _)| !expected.iter().any(|expected| name.as_str() == expected))
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: name.clone(),
        });
    }
    expected
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let parameter = ParameterName::new(name).expect("generated payload parameter name");
            match operation.parameter(&parameter) {
                Some(ParameterValue::Text(value)) => Ok(value.as_str().to_owned()),
                Some(_) => Err(OperationCompileError::WrongParameterType {
                    operation_id: operation.id(),
                    key: operation.key().clone(),
                    parameter,
                }),
                None => Ok(defaults[index].clone()),
            }
        })
        .collect()
}

fn invalid(operation: &Operation, error: &impl ToString) -> OperationCompileError {
    OperationCompileError::InvalidParameters {
        operation_id: operation.id(),
        key: operation.key().clone(),
        reason: error.to_string(),
    }
}
