use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterText, ParameterValue, PhotoId, Revision,
};

use super::command::{BasicEditCommand, BasicEditCommandError};
use super::{
    BasicEditDraft, BasicEditDraftError, BasicEditOperation, BasicEditParameter, BasicEditValue,
    BasicEditValueError, ParameterValueType,
};

fn id(value: u128) -> OperationId {
    OperationId::new(value).expect("nonzero operation ID")
}

fn parameter(name: &str) -> ParameterName {
    ParameterName::new(name).expect("valid parameter name")
}

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite test scalar"))
}

fn operation(
    operation_id: u128,
    key: &str,
    enabled: bool,
    opacity: f64,
    parameters: impl IntoIterator<Item = (&'static str, ParameterValue)>,
) -> Operation {
    Operation::new_with_opacity(
        id(operation_id),
        OperationKey::new(key).expect("valid operation key"),
        enabled,
        OperationOpacity::new(opacity).expect("valid opacity"),
        parameters
            .into_iter()
            .map(|(name, value)| (parameter(name), value)),
    )
    .expect("valid operation")
}

fn valid_edit(exposure: f64, red: f64, green: f64, blue: f64) -> Edit {
    Edit::from_parts(
        EditId::new(41).expect("edit ID"),
        PhotoId::new(42).expect("photo ID"),
        Revision::from_u64(7),
        Revision::from_u64(3),
        [
            operation(
                10,
                "rusttable.exposure",
                false,
                1.0,
                [
                    ("stops", scalar(exposure)),
                    (
                        "note",
                        ParameterValue::Text(ParameterText::new("keep").expect("text")),
                    ),
                ],
            ),
            operation(
                20,
                "rusttable.rgb_gain",
                true,
                1.0,
                [
                    ("red", scalar(red)),
                    ("green", scalar(green)),
                    ("blue", scalar(blue)),
                ],
            ),
            operation(
                30,
                "rusttable.tone",
                true,
                0.75,
                [
                    ("strength", scalar(0.25)),
                    ("label", ParameterValue::Bool(true)),
                ],
            ),
        ],
    )
    .expect("valid edit")
}

#[test]
fn parses_defaults_and_non_default_values_with_identity_and_state() {
    let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
    let draft = BasicEditDraft::from_edit(&edit).expect("valid draft");

    assert_eq!(draft.edit_id(), edit.id());
    assert_eq!(draft.photo_id(), edit.photo_id());
    assert_eq!(draft.base_photo_revision(), Revision::from_u64(7));
    assert_eq!(draft.edit_revision(), Revision::from_u64(3));
    assert_eq!(draft.exposure_operation_id(), id(10));
    assert_eq!(draft.rgb_gain_operation_id(), id(20));
    assert!(!draft.exposure_enabled());
    assert!(draft.rgb_gain_enabled());
    assert_eq!(draft.exposure_opacity(), OperationOpacity::ONE);
    assert_eq!(draft.rgb_gain_opacity(), OperationOpacity::ONE);
    assert_float_eq(draft.exposure_stops(), 0.0);
    assert_float_eq(draft.rgb_red(), 1.0);
    assert_float_eq(draft.rgb_green(), 1.0);
    assert_float_eq(draft.rgb_blue(), 1.0);

    let edit = valid_edit(-2.5, 0.25, 1.5, 2.0);
    let draft = BasicEditDraft::from_edit(&edit).expect("valid non-default draft");
    assert_float_eq(draft.exposure_stops(), -2.5);
    assert_float_eq(draft.rgb_red(), 0.25);
    assert_float_eq(draft.rgb_green(), 1.5);
    assert_float_eq(draft.rgb_blue(), 2.0);
}

#[test]
fn setters_accept_inclusive_boundaries_and_replace_values() {
    let mut draft = BasicEditDraft::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).unwrap();
    draft.set_exposure_stops(-5.0).unwrap();
    draft.set_rgb_red(0.0).unwrap();
    draft.set_rgb_green(2.0).unwrap();
    draft.set_rgb_blue(0.0).unwrap();

    assert_float_eq(draft.exposure_stops(), -5.0);
    assert_float_eq(draft.rgb_red(), 0.0);
    assert_float_eq(draft.rgb_green(), 2.0);
    assert_float_eq(draft.rgb_blue(), 0.0);
}

#[test]
fn setters_reject_non_finite_and_out_of_range_values() {
    let mut draft = BasicEditDraft::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).unwrap();
    assert_eq!(
        draft.set_exposure_stops(f64::NAN),
        Err(BasicEditValueError::NonFinite {
            value: BasicEditValue::ExposureStops
        })
    );
    assert!(matches!(
        draft.set_exposure_stops(-5.1),
        Err(BasicEditValueError::OutOfRange { .. })
    ));
    assert!(matches!(
        draft.set_rgb_red(2.1),
        Err(BasicEditValueError::OutOfRange {
            value: BasicEditValue::RgbRed,
            ..
        })
    ));
    assert_float_eq(draft.exposure_stops(), 0.0);
}

#[test]
fn rejects_missing_and_duplicate_instances() {
    let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
    let without_exposure = Edit::from_parts(
        edit.id(),
        edit.photo_id(),
        edit.base_photo_revision(),
        edit.revision(),
        edit.operations().skip(1).cloned().collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        BasicEditDraft::from_edit(&without_exposure),
        Err(BasicEditDraftError::MissingOperation {
            operation: BasicEditOperation::Exposure
        })
    );

    let duplicate = Edit::from_parts(
        edit.id(),
        edit.photo_id(),
        edit.base_photo_revision(),
        edit.revision(),
        edit.operations()
            .cloned()
            .chain(std::iter::once(operation(
                40,
                "rusttable.exposure",
                true,
                1.0,
                [("stops", scalar(1.0))],
            )))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    assert_eq!(
        BasicEditDraft::from_edit(&duplicate),
        Err(BasicEditDraftError::DuplicateOperation {
            operation: BasicEditOperation::Exposure
        })
    );
}

#[test]
fn rejects_missing_parameters_wrong_types_and_non_unit_opacity() {
    let edit = valid_edit(0.0, 1.0, 1.0, 1.0);
    let missing = operation(10, "rusttable.exposure", true, 1.0, []);
    let edit = replace_first(&edit, &missing);
    assert_eq!(
        BasicEditDraft::from_edit(&edit),
        Err(BasicEditDraftError::MissingParameter {
            operation: BasicEditOperation::Exposure,
            parameter: BasicEditParameter::ExposureStops
        })
    );

    let wrong_type = operation(
        10,
        "rusttable.exposure",
        true,
        1.0,
        [("stops", ParameterValue::Bool(true))],
    );
    let source = valid_edit(0.0, 1.0, 1.0, 1.0);
    let edit = replace_first(&source, &wrong_type);
    assert_eq!(
        BasicEditDraft::from_edit(&edit),
        Err(BasicEditDraftError::WrongParameterType {
            operation: BasicEditOperation::Exposure,
            parameter: BasicEditParameter::ExposureStops,
            actual: ParameterValueType::Boolean
        })
    );

    let non_unit = operation(
        10,
        "rusttable.exposure",
        true,
        0.5,
        [("stops", scalar(0.0))],
    );
    let source = valid_edit(0.0, 1.0, 1.0, 1.0);
    let edit = replace_first(&source, &non_unit);
    assert!(matches!(
        BasicEditDraft::from_edit(&edit),
        Err(BasicEditDraftError::NonUnitOpacity {
            operation: BasicEditOperation::Exposure,
            ..
        })
    ));
}

#[test]
fn rejects_source_values_outside_the_typed_ranges() {
    let source = valid_edit(0.0, 1.0, 1.0, 1.0);
    let replacement = operation(
        10,
        "rusttable.exposure",
        true,
        1.0,
        [("stops", scalar(5.01))],
    );
    let edit = replace_first(&source, &replacement);
    assert!(matches!(
        BasicEditDraft::from_edit(&edit),
        Err(BasicEditDraftError::OutOfRange {
            operation: BasicEditOperation::Exposure,
            parameter: BasicEditParameter::ExposureStops,
            ..
        })
    ));

    let source = valid_edit(0.0, 1.0, 1.0, 1.0);
    let replacement = operation(
        20,
        "rusttable.rgb_gain",
        true,
        1.0,
        [
            ("red", scalar(1.0)),
            ("green", scalar(-0.01)),
            ("blue", scalar(1.0)),
        ],
    );
    let edit = replace_second(&source, &replacement);
    assert!(matches!(
        BasicEditDraft::from_edit(&edit),
        Err(BasicEditDraftError::OutOfRange {
            operation: BasicEditOperation::RgbGain,
            parameter: BasicEditParameter::RgbGreen,
            ..
        })
    ));
}

#[test]
fn replacement_preserves_unrelated_operations_order_and_parameters() {
    let original = valid_edit(0.0, 1.0, 1.0, 1.0);
    let unrelated = original.operations().nth(2).unwrap().clone();
    let mut draft = BasicEditDraft::from_edit(&original).unwrap();
    draft.set_exposure_stops(2.0).unwrap();
    draft.set_rgb_red(0.5).unwrap();
    draft.set_rgb_green(1.25).unwrap();
    draft.set_rgb_blue(1.75).unwrap();
    let revised = draft.replacement_edit().expect("revision succeeds");

    assert_eq!(
        revised.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![id(10), id(20), id(30)]
    );
    assert_eq!(revised.operations().nth(2), Some(&unrelated));
    let exposure = revised.operations().next().unwrap();
    assert_eq!(
        exposure.parameter(&parameter("note")),
        Some(&ParameterValue::Text(ParameterText::new("keep").unwrap()))
    );
    assert!(!exposure.is_enabled());
    assert_eq!(exposure.opacity(), OperationOpacity::ONE);
    assert_eq!(exposure.parameter(&parameter("stops")), Some(&scalar(2.0)));
    let rgb = revised.operations().nth(1).unwrap();
    assert!(rgb.is_enabled());
    assert_eq!(rgb.opacity(), OperationOpacity::ONE);
    assert_eq!(rgb.parameter(&parameter("red")), Some(&scalar(0.5)));
    assert_eq!(rgb.parameter(&parameter("green")), Some(&scalar(1.25)));
    assert_eq!(rgb.parameter(&parameter("blue")), Some(&scalar(1.75)));
}

#[test]
fn replacement_preserves_identity_and_advances_only_edit_revision() {
    let original = valid_edit(0.0, 1.0, 1.0, 1.0);
    let draft = BasicEditDraft::from_edit(&original).unwrap();
    let revised = draft.replacement_edit().expect("revision succeeds");

    assert_eq!(revised.id(), original.id());
    assert_eq!(revised.photo_id(), original.photo_id());
    assert_eq!(
        revised.base_photo_revision(),
        original.base_photo_revision()
    );
    assert_eq!(revised.revision(), Revision::from_u64(4));
    assert_eq!(original.revision(), Revision::from_u64(3));
}

#[test]
fn command_reads_default_and_non_default_typed_values() {
    let defaults =
        BasicEditCommand::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).expect("default command");
    let default_values = defaults.values();
    assert_float_eq(default_values.exposure_stops(), 0.0);
    assert_float_eq(default_values.rgb_red(), 1.0);
    assert_float_eq(default_values.rgb_green(), 1.0);
    assert_float_eq(default_values.rgb_blue(), 1.0);

    let nondefaults = BasicEditCommand::from_edit(&valid_edit(-2.5, 0.25, 1.5, 2.0))
        .expect("non-default command");
    let values = nondefaults.values();
    assert_float_eq(values.exposure_stops(), -2.5);
    assert_float_eq(values.rgb_red(), 0.25);
    assert_float_eq(values.rgb_green(), 1.5);
    assert_float_eq(values.rgb_blue(), 2.0);
}

#[test]
fn command_rejects_out_of_range_values_without_mutating_the_draft() {
    let command = BasicEditCommand::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).unwrap();
    let error = command
        .build_replacement(0.0, 2.1, 1.0, 1.0)
        .expect_err("red gain is outside its range");
    assert!(matches!(
        error,
        BasicEditCommandError::InvalidValue(BasicEditValueError::OutOfRange {
            value: BasicEditValue::RgbRed,
            ..
        })
    ));
    let values = command.values();
    assert_float_eq(values.exposure_stops(), 0.0);
    assert_float_eq(values.rgb_red(), 1.0);
    assert_float_eq(values.rgb_green(), 1.0);
    assert_float_eq(values.rgb_blue(), 1.0);
}

#[test]
fn command_replacement_preserves_operation_order() {
    let command = BasicEditCommand::from_edit(&valid_edit(0.0, 1.0, 1.0, 1.0)).unwrap();
    let revised = command
        .build_replacement(2.0, 0.5, 1.25, 1.75)
        .expect("replacement");
    assert_eq!(
        revised.operations().map(Operation::id).collect::<Vec<_>>(),
        vec![id(10), id(20), id(30)]
    );
}

#[test]
fn command_builds_one_atomic_replacement_with_all_values() {
    let original = valid_edit(0.0, 1.0, 1.0, 1.0);
    let command = BasicEditCommand::from_edit(&original).unwrap();
    let revised = command
        .build_replacement(2.0, 0.5, 1.25, 1.75)
        .expect("replacement");

    assert_eq!(revised.revision(), Revision::from_u64(4));
    assert_eq!(revised.operations().count(), original.operations().count());
    let exposure = revised.operations().next().unwrap();
    let rgb = revised.operations().nth(1).unwrap();
    assert_eq!(exposure.parameter(&parameter("stops")), Some(&scalar(2.0)));
    assert_eq!(rgb.parameter(&parameter("red")), Some(&scalar(0.5)));
    assert_eq!(rgb.parameter(&parameter("green")), Some(&scalar(1.25)));
    assert_eq!(rgb.parameter(&parameter("blue")), Some(&scalar(1.75)));
}

fn assert_float_eq(actual: f64, expected: f64) {
    assert_eq!(actual.to_bits(), expected.to_bits());
}

fn replace_first(edit: &Edit, operation: &Operation) -> Edit {
    replace_at(edit, 0, operation)
}

fn replace_second(edit: &Edit, operation: &Operation) -> Edit {
    replace_at(edit, 1, operation)
}

fn replace_at(edit: &Edit, index: usize, replacement: &Operation) -> Edit {
    Edit::from_parts(
        edit.id(),
        edit.photo_id(),
        edit.base_photo_revision(),
        edit.revision(),
        edit.operations()
            .enumerate()
            .map(|(current, operation)| {
                if current == index {
                    replacement.clone()
                } else {
                    operation.clone()
                }
            })
            .collect::<Vec<_>>(),
    )
    .expect("valid replacement edit")
}
