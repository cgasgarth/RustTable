use std::str::FromStr;

use rusttable_core::{
    FiniteF64, Operation, OperationBuildError, OperationId, OperationKey, OperationKeyError,
    ParameterName, ParameterText, ParameterTextError, ParameterValue,
};

fn operation_key(value: &str) -> OperationKey {
    OperationKey::from_str(value).expect("valid operation key")
}

fn parameter_name(value: &str) -> ParameterName {
    ParameterName::from_str(value).expect("valid parameter name")
}

fn operation(parameters: Vec<(ParameterName, ParameterValue)>) -> Operation {
    Operation::new(
        OperationId::new(1).expect("nonzero operation ID"),
        operation_key("rusttable.exposure"),
        true,
        parameters,
    )
    .expect("valid operation")
}

#[test]
fn operation_keys_validate_grammar_and_boundaries() {
    assert_eq!(
        operation_key("rusttable.exposure").as_str(),
        "rusttable.exposure"
    );
    assert_eq!(
        operation_key("rusttable.linear_offset").to_string(),
        "rusttable.linear_offset"
    );
    assert!(matches!(
        OperationKey::from_str(""),
        Err(OperationKeyError::EmptySegment { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("single"),
        Err(OperationKeyError::SegmentCount { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("a.b.c.d.e"),
        Err(OperationKeyError::SegmentCount { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("a..b"),
        Err(OperationKeyError::EmptySegment { segment: 1 })
    ));
    assert!(matches!(
        OperationKey::from_str("1.exposure"),
        Err(OperationKeyError::InvalidInitialByte { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("rusttable.eXposure"),
        Err(OperationKeyError::InvalidSubsequentByte { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("rusttable.ex-posure"),
        Err(OperationKeyError::InvalidSubsequentByte { .. })
    ));
    assert!(matches!(
        OperationKey::from_str("rüst.exposure"),
        Err(OperationKeyError::NonAscii { .. })
    ));
    assert!(matches!(
        OperationKey::from_str(&format!("{}.exposure", "a".repeat(33))),
        Err(OperationKeyError::SegmentLength { .. })
    ));
    assert!(matches!(
        OperationKey::from_str(&"a.".repeat(65)),
        Err(OperationKeyError::Length { .. } | OperationKeyError::SegmentCount { .. })
    ));
}

#[test]
fn parameter_names_and_text_preserve_explicit_validation() {
    assert_eq!(parameter_name("stops").as_str(), "stops");
    assert_eq!(
        parameter_name("linear_offset_2").to_string(),
        "linear_offset_2"
    );
    assert!(ParameterName::from_str("").is_err());
    assert!(ParameterName::from_str("1stops").is_err());
    assert!(ParameterName::from_str("linear.offset").is_err());
    assert!(ParameterName::from_str("linear-offset").is_err());
    assert!(ParameterName::from_str("Stoops").is_err());
    assert!(ParameterName::from_str("støps").is_err());
    assert!(ParameterName::from_str(&"a".repeat(65)).is_err());

    let unicode = ParameterText::from_str("café 🎞️").expect("valid UTF-8 text");
    assert_eq!(unicode.as_str(), "café 🎞️");
    assert_eq!(
        ParameterText::from_str("").expect("empty text").as_str(),
        ""
    );
    assert!(matches!(
        ParameterText::from_str("a\0b"),
        Err(ParameterTextError::EmbeddedNul { .. })
    ));
    assert!(ParameterText::from_str(&"a".repeat(4_096)).is_ok());
    assert!(matches!(
        ParameterText::from_str(&"a".repeat(4_097)),
        Err(ParameterTextError::Length { .. })
    ));
}

#[test]
fn operations_are_immutable_and_canonical() {
    let name = parameter_name("stops");
    let text = ParameterText::from_str("note").expect("valid text");
    let first = operation(vec![
        (name.clone(), ParameterValue::Text(text.clone())),
        (parameter_name("enabled"), ParameterValue::Bool(true)),
    ]);
    let second = operation(vec![
        (parameter_name("enabled"), ParameterValue::Bool(true)),
        (name.clone(), ParameterValue::Text(text.clone())),
    ]);

    assert_eq!(first, second);
    assert_eq!(first.id(), second.id());
    assert!(first.is_enabled());
    assert_eq!(first.key().as_str(), "rusttable.exposure");
    assert_eq!(first.parameter(&name), Some(&ParameterValue::Text(text)));
    assert_eq!(
        first
            .parameters()
            .map(|(key, _)| key.as_str())
            .collect::<Vec<_>>(),
        vec!["enabled", "stops"]
    );
}

#[test]
fn operations_accept_scalars_and_empty_parameters() {
    let empty = operation(Vec::new());
    assert_eq!(empty.parameters().count(), 0);
    let scalar = ParameterValue::Scalar(FiniteF64::new(1.5).expect("finite"));
    let operation = operation(vec![(parameter_name("stops"), scalar)]);
    assert!(matches!(
        operation.parameter(&parameter_name("stops")),
        Some(ParameterValue::Scalar(_))
    ));
}

#[test]
fn duplicate_parameter_names_are_rejected_without_partial_state() {
    let name = parameter_name("stops");
    let error = Operation::new(
        OperationId::new(1).expect("nonzero operation ID"),
        operation_key("rusttable.exposure"),
        false,
        vec![
            (name.clone(), ParameterValue::Integer(1)),
            (name.clone(), ParameterValue::Integer(2)),
        ],
    )
    .expect_err("duplicate parameter names are invalid");

    assert_eq!(error, OperationBuildError::DuplicateParameterName { name });
}
