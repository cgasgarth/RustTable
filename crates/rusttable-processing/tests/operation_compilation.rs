use std::str::FromStr;

use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterText, ParameterValue,
};
use rusttable_processing::{
    FiniteF32, OperationCompileError, ProcessingOperation, ProcessingOperationKind,
};

fn operation(
    id: u128,
    key: &str,
    enabled: bool,
    parameters: Vec<(&str, ParameterValue)>,
) -> Operation {
    Operation::new(
        OperationId::new(id).expect("nonzero operation ID"),
        OperationKey::from_str(key).expect("valid operation key"),
        enabled,
        parameters.into_iter().map(|(name, value)| {
            (
                ParameterName::from_str(name).expect("valid parameter name"),
                value,
            )
        }),
    )
    .expect("unique parameters")
}

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite parameter"))
}

fn name(name: &str) -> ParameterName {
    ParameterName::from_str(name).expect("valid parameter name")
}

#[test]
fn compiles_exposure() {
    let compiled = ProcessingOperation::compile(&operation(
        1,
        "rusttable.exposure",
        true,
        vec![("stops", scalar(1.5))],
    ))
    .expect("exposure schema is valid");

    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::Exposure {
            stops: FiniteF32::new(1.5).expect("finite"),
            black: FiniteF32::new(0.0).expect("finite"),
        }
    );
}

#[test]
fn compiles_exposure_black_level_and_preserves_legacy_default() {
    let compiled = ProcessingOperation::compile(&operation(
        12,
        "rusttable.exposure",
        true,
        vec![("stops", scalar(1.0)), ("black", scalar(0.125))],
    ))
    .expect("exposure black-level schema is valid");

    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::Exposure {
            stops: FiniteF32::new(1.0).expect("finite"),
            black: FiniteF32::new(0.125).expect("finite"),
        }
    );

    let legacy = ProcessingOperation::compile(&operation(
        13,
        "rusttable.exposure",
        true,
        vec![("stops", scalar(1.0))],
    ))
    .expect("legacy exposure schema is valid");
    assert!(matches!(
        legacy.kind(),
        ProcessingOperationKind::Exposure { black, .. }
            if black.get().to_bits() == 0.0_f32.to_bits()
    ));
}

#[test]
fn compiles_linear_offset() {
    let compiled = ProcessingOperation::compile(&operation(
        2,
        "rusttable.linear_offset",
        false,
        vec![("value", scalar(-0.25))],
    ))
    .expect("linear offset schema is valid");

    assert_eq!(
        compiled.kind(),
        &ProcessingOperationKind::LinearOffset {
            value: FiniteF32::new(-0.25).expect("finite")
        }
    );
}

#[test]
fn preserves_identity_and_enabled_state() {
    let source = operation(7, "rusttable.exposure", false, vec![("stops", scalar(0.5))]);

    let compiled = ProcessingOperation::compile(&source).expect("valid operation");

    assert_eq!(compiled.operation_id(), source.id());
    assert!(!compiled.is_enabled());
}

#[test]
fn rejects_unsupported_key() {
    let source = operation(3, "rusttable.curves", true, vec![]);

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::UnsupportedOperationKey { operation_id, key })
            if operation_id == source.id() && key == *source.key()
    ));
}

#[test]
fn rejects_missing_parameter() {
    let source = operation(4, "rusttable.exposure", true, vec![]);

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::MissingParameter { operation_id, key, parameter })
            if operation_id == source.id()
                && key == *source.key()
                && parameter == name("stops")
    ));
}

#[test]
fn rejects_unexpected_parameter() {
    let source = operation(
        5,
        "rusttable.exposure",
        true,
        vec![
            ("stops", scalar(1.0)),
            ("zeta", scalar(2.0)),
            ("alpha", scalar(3.0)),
        ],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::UnexpectedParameter { operation_id, key, parameter })
            if operation_id == source.id()
                && key == *source.key()
                && parameter == name("alpha")
    ));
}

#[test]
fn rejects_wrong_parameter_type() {
    let source = operation(
        6,
        "rusttable.exposure",
        true,
        vec![("stops", ParameterValue::Bool(true))],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::WrongParameterType { operation_id, key, parameter })
            if operation_id == source.id()
                && key == *source.key()
                && parameter == name("stops")
    ));
}

#[test]
fn rejects_scalar_overflow() {
    let source = operation(
        8,
        "rusttable.exposure",
        true,
        vec![("stops", scalar(f64::MAX))],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::ScalarNarrowingOverflow { operation_id, key, parameter })
            if operation_id == source.id()
                && key == *source.key()
                && parameter == name("stops")
    ));
}

#[test]
fn rejects_scalar_underflow() {
    let source = operation(
        9,
        "rusttable.exposure",
        true,
        vec![("stops", scalar(f64::MIN_POSITIVE * 0.5))],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::ScalarNarrowingUnderflow { operation_id, key, parameter })
            if operation_id == source.id()
                && key == *source.key()
                && parameter == name("stops")
    ));
}

#[test]
fn selects_schema_errors_deterministically() {
    let source = operation(
        10,
        "rusttable.linear_offset",
        true,
        vec![
            (
                "zeta",
                ParameterValue::Text(ParameterText::new("unexpected").expect("text")),
            ),
            ("alpha", ParameterValue::Bool(true)),
        ],
    );

    assert!(matches!(
        ProcessingOperation::compile(&source),
        Err(OperationCompileError::MissingParameter { parameter, .. })
            if parameter == name("value")
    ));
}

#[test]
fn equal_operations_compile_equally() {
    let first = operation(11, "rusttable.exposure", true, vec![("stops", scalar(2.0))]);
    let second = operation(11, "rusttable.exposure", true, vec![("stops", scalar(2.0))]);

    assert_eq!(
        ProcessingOperation::compile(&first),
        ProcessingOperation::compile(&second)
    );
}
