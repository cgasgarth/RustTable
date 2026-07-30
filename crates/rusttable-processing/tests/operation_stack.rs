use rusttable_processing::descriptor::exposure_descriptor;
use rusttable_processing::operation_stack::{
    InsertPosition, MigrationFinding, MigrationOutcome, OperationInstance, OperationStackSnapshot,
    OperationStackTemplate, StackCommand, StackStage,
};

fn operation(id: u128, multi_instance: bool) -> OperationInstance {
    OperationInstance::new(
        id,
        exposure_descriptor().id,
        vec![0, 1],
        StackStage::SceneLinear,
        false,
        multi_instance,
    )
    .expect("valid operation")
}

#[test]
fn operation_stack_templates_are_explicit_and_empty_until_descriptors_are_registered() {
    let raster = OperationStackSnapshot::new(OperationStackTemplate::raster_basic());
    let raw = OperationStackSnapshot::new(OperationStackTemplate::raw_basic());

    assert_eq!(raster.template().name(), "RasterBasic");
    assert_eq!(raw.template().name(), "RawBasic");
    raster.validate().expect("raster template");
    raw.validate().expect("raw template");
}

#[test]
fn operation_stack_every_mutation_returns_a_new_snapshot_and_receipt() {
    let empty = OperationStackSnapshot::new(OperationStackTemplate::raster_basic());
    let inserted = empty
        .apply(StackCommand::Insert {
            operation: operation(1, true),
            position: InsertPosition::End,
        })
        .expect("insert");
    let renamed = inserted
        .snapshot
        .apply(StackCommand::Rename {
            id: 1,
            name: Some("Exposure".to_owned()),
        })
        .expect("rename");

    assert!(empty.operations().is_empty());
    assert!(inserted.snapshot.operations()[0].name().is_none());
    assert_eq!(renamed.snapshot.operations()[0].name(), Some("Exposure"));
    assert_eq!(inserted.receipt.new_hash, renamed.receipt.new_hash);
}

#[test]
fn operation_stack_failed_commands_preserve_the_original_snapshot() {
    let snapshot = OperationStackSnapshot::new(OperationStackTemplate::raster_basic())
        .apply(StackCommand::Insert {
            operation: operation(1, false),
            position: InsertPosition::End,
        })
        .expect("insert")
        .snapshot;
    let before = snapshot.clone();

    assert!(
        snapshot
            .apply(StackCommand::Duplicate { id: 1, new_id: 2 })
            .is_err()
    );
    assert_eq!(snapshot, before);
}

#[test]
fn operation_stack_migration_findings_have_an_opaque_non_executable_path() {
    let opaque = MigrationOutcome::Opaque(rusttable_processing::OpaqueOperation {
        source_version: 99,
        raw: vec![1, 2, 3],
        original_index: 4,
    });
    assert!(matches!(opaque, MigrationOutcome::Opaque(_)));
    assert_eq!(
        MigrationFinding::UnknownVersion,
        MigrationFinding::UnknownVersion
    );
}
