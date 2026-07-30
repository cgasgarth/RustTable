use rusttable_core::{
    EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
    OperationOpacityError, ParameterName, ParameterValue, PhotoId, Revision,
};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn operation(opacity: OperationOpacity) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(1).expect("nonzero operation ID"),
        OperationKey::new("rusttable.test").expect("valid operation key"),
        true,
        opacity,
        [(
            ParameterName::new("value").expect("valid parameter name"),
            ParameterValue::Scalar(FiniteF64::new(1.0).expect("finite")),
        )],
    )
    .expect("valid operation")
}

#[test]
fn opacity_validates_endpoints_and_errors() {
    assert_eq!(
        OperationOpacity::new(f64::NAN),
        Err(OperationOpacityError::NonFinite)
    );
    assert_eq!(
        OperationOpacity::new(-0.01),
        Err(OperationOpacityError::BelowZero)
    );
    assert_eq!(
        OperationOpacity::new(1.01),
        Err(OperationOpacityError::AboveOne)
    );
    assert_eq!(OperationOpacity::ZERO.get().to_bits(), 0.0f64.to_bits());
    assert_eq!(OperationOpacity::ONE.get().to_bits(), 1.0f64.to_bits());
    assert_eq!(OperationOpacity::new(0.25).unwrap().to_string(), "0.25");
    assert_eq!(
        OperationOpacityError::AboveOne.to_string(),
        "operation opacity must not be above one"
    );
    let error: &dyn std::error::Error = &OperationOpacityError::AboveOne;
    assert!(error.source().is_none());
}

#[test]
fn opacity_orders_and_hashes_as_its_canonical_value() {
    let low = OperationOpacity::new(0.25).unwrap();
    let high = OperationOpacity::new(0.75).unwrap();
    let mut low_hasher = DefaultHasher::new();
    let mut high_hasher = DefaultHasher::new();
    low.hash(&mut low_hasher);
    high.hash(&mut high_hasher);

    assert!(low < high);
    assert_ne!(low_hasher.finish(), high_hasher.finish());
}

#[test]
fn operation_new_defaults_to_one_and_explicit_opacity_participates_in_equality() {
    let default = Operation::new(
        OperationId::new(1).unwrap(),
        OperationKey::new("rusttable.test").unwrap(),
        true,
        [],
    )
    .unwrap();
    let explicit = operation(OperationOpacity::new(0.5).unwrap());
    let same_explicit = Operation::new_with_opacity(
        explicit.id(),
        explicit.key().clone(),
        explicit.is_enabled(),
        explicit.opacity(),
        explicit
            .parameters()
            .map(|(name, value)| (name.clone(), value.clone())),
    )
    .unwrap();

    assert_eq!(default.opacity(), OperationOpacity::ONE);
    assert_eq!(explicit, same_explicit);
    assert_ne!(default, explicit);
}

#[test]
fn opacity_is_preserved_in_reconstructed_edits() {
    let edit = rusttable_core::Edit::from_parts(
        EditId::new(1).unwrap(),
        PhotoId::new(2).unwrap(),
        Revision::ZERO,
        Revision::ZERO,
        [operation(OperationOpacity::new(0.25).unwrap())],
    )
    .unwrap();

    assert_eq!(
        edit.operations().next().unwrap().opacity().get().to_bits(),
        0.25f64.to_bits()
    );
}
