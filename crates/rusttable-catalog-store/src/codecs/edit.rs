use postcard::{from_bytes, to_allocvec};
use serde::{Deserialize, Serialize};

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterText, ParameterValue, PhotoId, Revision,
};

const EDIT_FORMAT_VERSION: u8 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StoredEdit {
    version: u8,
    id: [u8; 16],
    photo_id: [u8; 16],
    base_photo_revision: [u8; 8],
    revision: [u8; 8],
    operations: Vec<StoredOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredOperation {
    id: [u8; 16],
    key: Vec<u8>,
    enabled: bool,
    opacity: [u8; 8],
    parameters: Vec<StoredParameter>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredParameter {
    name: Vec<u8>,
    kind: u8,
    value: Vec<u8>,
}

pub(crate) fn encode(edit: &Edit) -> Result<Vec<u8>, ()> {
    let operations = edit.operations().map(encode_operation).collect();
    to_allocvec(&StoredEdit {
        version: EDIT_FORMAT_VERSION,
        id: edit.id().get().to_be_bytes(),
        photo_id: edit.photo_id().get().to_be_bytes(),
        base_photo_revision: edit.base_photo_revision().get().to_be_bytes(),
        revision: edit.revision().get().to_be_bytes(),
        operations,
    })
    .map_err(|_| ())
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Edit, ()> {
    let stored: StoredEdit = from_bytes(bytes).map_err(|_| ())?;
    if stored.version != EDIT_FORMAT_VERSION {
        return Err(());
    }
    let operations = stored
        .operations
        .iter()
        .map(decode_operation)
        .collect::<Result<Vec<_>, _>>()?;
    Edit::from_parts(
        edit_id(stored.id)?,
        photo_id(stored.photo_id)?,
        Revision::from_u64(u64::from_be_bytes(stored.base_photo_revision)),
        Revision::from_u64(u64::from_be_bytes(stored.revision)),
        operations,
    )
    .map_err(|_| ())
}

fn encode_operation(operation: &Operation) -> StoredOperation {
    let parameters = operation.parameters().map(encode_parameter).collect();
    StoredOperation {
        id: operation.id().get().to_be_bytes(),
        key: operation.key().as_str().as_bytes().to_vec(),
        enabled: operation.is_enabled(),
        opacity: operation.opacity().get().to_bits().to_be_bytes(),
        parameters,
    }
}

fn decode_operation(stored: &StoredOperation) -> Result<Operation, ()> {
    let parameters = stored
        .parameters
        .iter()
        .map(decode_parameter)
        .collect::<Result<Vec<_>, _>>()?;
    Operation::new_with_opacity(
        operation_id(stored.id)?,
        OperationKey::new(text(&stored.key)?).map_err(|_| ())?,
        stored.enabled,
        opacity(stored.opacity)?,
        parameters,
    )
    .map_err(|_| ())
}

fn encode_parameter((name, value): (&ParameterName, &ParameterValue)) -> StoredParameter {
    let (kind, value) = match value {
        ParameterValue::Bool(value) => (1, vec![u8::from(*value)]),
        ParameterValue::Integer(value) => (2, value.to_be_bytes().to_vec()),
        ParameterValue::Scalar(value) => (3, value.get().to_bits().to_be_bytes().to_vec()),
        ParameterValue::Text(value) => (4, value.as_str().as_bytes().to_vec()),
    };
    StoredParameter {
        name: name.as_str().as_bytes().to_vec(),
        kind,
        value,
    }
}

fn decode_parameter(stored: &StoredParameter) -> Result<(ParameterName, ParameterValue), ()> {
    let name = ParameterName::new(text(&stored.name)?).map_err(|_| ())?;
    let value = match stored.kind {
        1 => ParameterValue::Bool(match stored.value.as_slice() {
            [0] => false,
            [1] => true,
            _ => return Err(()),
        }),
        2 => ParameterValue::Integer(i64::from_be_bytes(array(&stored.value)?)),
        3 => ParameterValue::Scalar(
            FiniteF64::new(f64::from_bits(u64::from_be_bytes(array(&stored.value)?)))
                .map_err(|_| ())?,
        ),
        4 => ParameterValue::Text(ParameterText::new(text(&stored.value)?).map_err(|_| ())?),
        _ => return Err(()),
    };
    Ok((name, value))
}

fn opacity(bytes: [u8; 8]) -> Result<OperationOpacity, ()> {
    OperationOpacity::new(f64::from_bits(u64::from_be_bytes(bytes))).map_err(|_| ())
}

fn text(bytes: &[u8]) -> Result<String, ()> {
    String::from_utf8(bytes.to_vec()).map_err(|_| ())
}

fn array<const N: usize>(bytes: &[u8]) -> Result<[u8; N], ()> {
    bytes.try_into().map_err(|_| ())
}

fn edit_id(bytes: [u8; 16]) -> Result<EditId, ()> {
    EditId::new(u128::from_be_bytes(bytes)).ok_or(())
}

fn photo_id(bytes: [u8; 16]) -> Result<PhotoId, ()> {
    PhotoId::new(u128::from_be_bytes(bytes)).ok_or(())
}

fn operation_id(bytes: [u8; 16]) -> Result<OperationId, ()> {
    OperationId::new(u128::from_be_bytes(bytes)).ok_or(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_edit_round_trips_with_exact_scalar_bits() {
        let edit = Edit::from_parts(
            EditId::new(1).unwrap(),
            PhotoId::new(2).unwrap(),
            Revision::from_u64(3),
            Revision::from_u64(4),
            [Operation::new_with_opacity(
                OperationId::new(5).unwrap(),
                OperationKey::new("rusttable.exposure").unwrap(),
                true,
                OperationOpacity::new(0.5).unwrap(),
                [
                    (
                        ParameterName::new("enabled").unwrap(),
                        ParameterValue::Bool(true),
                    ),
                    (
                        ParameterName::new("stops").unwrap(),
                        ParameterValue::Scalar(FiniteF64::new(-0.25).unwrap()),
                    ),
                ],
            )
            .unwrap()],
        )
        .unwrap();

        assert_eq!(decode(&encode(&edit).unwrap()).unwrap(), edit);
    }
}
