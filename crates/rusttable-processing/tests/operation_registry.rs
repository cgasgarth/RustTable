use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::{
    FactoryError, OperationDefinition, ProcessingOperationKind, RegistryValidationError,
    builtin_registry,
};

fn operation(id: u128, key: &str, parameters: &[(&str, f64)]) -> Operation {
    Operation::new(
        OperationId::new(id).expect("operation ID"),
        OperationKey::new(key).expect("operation key"),
        true,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("operation")
}

fn missing_cpu_definition() -> OperationDefinition {
    let builtin = &builtin_registry().definitions()[0];
    OperationDefinition::new(
        builtin.descriptor().clone(),
        None,
        builtin.gpu().cloned(),
        builtin.migrations().to_vec(),
        builtin.identity().clone(),
        builtin.evidence_ids().to_vec(),
    )
}

fn migration_gap_definition() -> OperationDefinition {
    let builtin = &builtin_registry().definitions()[0];
    let mut descriptor = builtin.descriptor().clone();
    descriptor.migration.source_versions = vec![1, 2];
    descriptor.migration.target_version = 2;
    OperationDefinition::new(
        descriptor,
        builtin.cpu(),
        builtin.gpu().cloned(),
        Vec::new(),
        builtin.identity().clone(),
        builtin.evidence_ids().to_vec(),
    )
}

#[test]
fn operation_registry_executes_all_first_party_operations_through_factories() {
    let registry = builtin_registry();
    let cases = [
        operation(1, "rusttable.exposure", &[("stops", 0.5)]),
        operation(2, "rusttable.linear_offset", &[("value", 0.25)]),
        operation(
            3,
            "rusttable.rgb_gain",
            &[("red", 1.0), ("green", 0.75), ("blue", 0.5)],
        ),
    ];

    let prepared = cases
        .iter()
        .map(|operation| registry.prepare_cpu(operation).expect("factory"))
        .collect::<Vec<_>>();
    assert!(matches!(
        prepared[0].operation().kind(),
        ProcessingOperationKind::Exposure { .. }
    ));
    assert!(matches!(
        prepared[1].operation().kind(),
        ProcessingOperationKind::LinearOffset { .. }
    ));
    assert!(matches!(
        prepared[2].operation().kind(),
        ProcessingOperationKind::RgbGain { .. }
    ));
}

#[test]
fn operation_registry_keeps_unknown_imported_identity_opaque() {
    let error = builtin_registry()
        .prepare_cpu(&operation(7, "rusttable.unknown", &[]))
        .expect_err("unknown operation must not be constructed");
    assert!(matches!(
        error,
        rusttable_processing::RegistryLookupError::UnknownOperation(_)
    ));
}

#[test]
fn operation_registry_rejects_definition_without_cpu_factory() {
    let error = rusttable_processing::RegistrySnapshot::try_new(&[missing_cpu_definition])
        .expect_err("missing CPU must be rejected");
    assert!(
        error
            .findings()
            .iter()
            .any(|finding| matches!(finding, RegistryValidationError::MissingCpu(_)))
    );
}

#[test]
fn operation_registry_rejects_migration_gap() {
    let error = rusttable_processing::RegistrySnapshot::try_new(&[migration_gap_definition])
        .expect_err("migration gap must be rejected");
    assert!(
        error
            .findings()
            .iter()
            .any(|finding| matches!(finding, RegistryValidationError::MigrationGap(_)))
    );
}

#[test]
fn operation_registry_reports_factory_errors_with_operation_context() {
    let error = builtin_registry()
        .prepare_cpu(&operation(8, "rusttable.exposure", &[]))
        .expect_err("missing parameter");
    assert!(matches!(
        error,
        rusttable_processing::RegistryLookupError::Factory {
            source,
            ..
        } if matches!(source.as_ref(), FactoryError::Operation(_))
    ));
}
