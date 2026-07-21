use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::{
    BasicAdjConfig, BasicAdjParametersV2, PreserveColors, ProcessingOperationKind, builtin_registry,
};

fn operation(parameters: &[(&str, f64)]) -> Operation {
    Operation::new(
        OperationId::new(321).expect("operation ID"),
        OperationKey::new("rusttable.basicadj").expect("operation key"),
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

#[test]
fn registry_compiles_basicadj_as_one_atomic_operation() {
    let prepared = builtin_registry()
        .prepare_cpu(&operation(&[
            ("exposure", 1.0),
            ("black_point", 0.05),
            ("contrast", 0.5),
            ("preserve_colors", 1.0),
        ]))
        .expect("basicadj factory");
    assert!(matches!(
        prepared.operation().kind(),
        ProcessingOperationKind::BasicAdj { config }
            if config.preserve_colors() == PreserveColors::Luminance
    ));
}

#[test]
fn compiler_rejects_unknown_preserve_colors_mode() {
    let error = builtin_registry()
        .prepare_cpu(&operation(&[("preserve_colors", 99.0)]))
        .expect_err("unknown mode must be rejected");
    assert!(error.to_string().contains("preserve-colors"));
}

#[test]
fn config_identity_includes_auto_clip_control() {
    let first = BasicAdjParametersV2::defaults();
    let mut second = first;
    second.clip = 0.1;
    let first = BasicAdjConfig::new(first).expect("first config");
    let second = BasicAdjConfig::new(second).expect("second config");
    let first_plan = rusttable_processing::BasicAdjPlan::new(first).expect("first plan");
    let second_plan = rusttable_processing::BasicAdjPlan::new(second).expect("second plan");
    assert_ne!(first_plan.identity(), second_plan.identity());
}
