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
        operation(
            4,
            "rusttable.bloom",
            &[("size", 0.0), ("threshold", 0.0), ("strength", 25.0)],
        ),
        operation(
            5,
            "rusttable.soften",
            &[
                ("size", 0.0),
                ("saturation", 100.0),
                ("brightness", 0.33),
                ("amount", 50.0),
            ],
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
    assert!(matches!(
        prepared[3].operation().kind(),
        ProcessingOperationKind::Bloom { .. }
    ));
    assert!(matches!(
        prepared[4].operation().kind(),
        ProcessingOperationKind::Soften { .. }
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

#[test]
fn operation_registry_preserves_darktable_declaration_order_for_ui_projections() {
    let ids = builtin_registry()
        .definitions_in_declaration_order()
        .into_iter()
        .map(|definition| definition.descriptor().id.compatibility_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), builtin_registry().definitions().len());
    assert_eq!(
        &ids[..8],
        [
            "exposure",
            "basicadj",
            "linear-offset",
            "rgbgain",
            "invert",
            "defringe",
            "clahe",
            "dither"
        ]
    );
    assert_eq!(ids.last(), Some(&"colorcorrection"));
}

#[test]
fn censorize_is_registry_visible_and_cpu_qualified() {
    let definition = builtin_registry()
        .definition("rusttable.censorize")
        .expect("censorize registry seam");
    assert_eq!(definition.descriptor().parameters.len(), 4);
    assert!(definition.availability().is_available());
    assert!(
        builtin_registry()
            .capability(
                "rusttable.censorize",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("preview"),
            )
            .is_some_and(|capability| capability.available)
    );
}

#[test]
fn clahe_registry_is_descriptor_visible_but_truthfully_unavailable() {
    let registry = builtin_registry();
    let definition = registry
        .definition("rusttable.clahe")
        .expect("CLAHE registry seam");
    let descriptor = definition.descriptor();
    assert_eq!(descriptor.id.compatibility_name, "clahe");
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::DEPRECATED)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::HIDDEN)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::STYLE_ELIGIBLE)
    );
    assert!(definition.cpu().is_none());
    assert_eq!(
        definition.availability().reason(),
        Some("backend qualification is pending #473; rusttable.clahe is read-only")
    );
    let radius = descriptor
        .parameters
        .iter()
        .find(|parameter| parameter.id == "radius")
        .expect("radius descriptor");
    assert_eq!(
        radius.kind,
        rusttable_processing::descriptor::ParameterKind::Scalar {
            minimum: 0.0,
            maximum: 256.0,
        }
    );
    assert_eq!(
        radius.default,
        rusttable_processing::descriptor::ParameterDefault::Scalar(64.0)
    );
    assert_eq!(
        registry
            .capability(
                "rusttable.clahe",
                &rusttable_processing::DeviceCapabilitySnapshot::cpu_only(),
                rusttable_color::ColorEncoding::LinearSrgbD65,
                Some("full"),
            )
            .and_then(|capability| capability.reason),
        Some("backend qualification is pending #473; rusttable.clahe is read-only".to_owned())
    );
}
