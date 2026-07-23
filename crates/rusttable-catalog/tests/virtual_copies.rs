use rusttable_catalog::{
    SourceAssetIdentity, VirtualCopy, VirtualCopyCatalog, VirtualCopyCommand, VirtualCopyId,
};
use rusttable_core::{
    AssetId, Edit, EditId, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};

fn edit(copy_id: VirtualCopyId, edit_id: u128, revision: u64, value: i64) -> Edit {
    Edit::from_parts(
        EditId::new(edit_id).unwrap(),
        copy_id.photo_id(),
        Revision::ZERO,
        Revision::from_u64(revision),
        [Operation::new(
            OperationId::new(edit_id + 1000).unwrap(),
            OperationKey::new("rusttable.exposure").unwrap(),
            true,
            [(
                ParameterName::new("stops").unwrap(),
                ParameterValue::Integer(value),
            )],
        )
        .unwrap()],
    )
    .unwrap()
}

fn copy(id: u128, asset: u128, order: u64, edit_id: u128) -> VirtualCopy {
    let id = VirtualCopyId::new(id).unwrap();
    VirtualCopy::new(
        id,
        SourceAssetIdentity::new(PhotoId::new(10).unwrap(), AssetId::new(asset).unwrap()),
        order,
        edit(id, edit_id, 0, 1),
    )
    .unwrap()
}

#[test]
fn copies_share_one_explicit_source_but_edit_history_is_independent() {
    let first = copy(1, 50, 0, 100);
    let second = copy(2, 50, 1, 200);
    let mut catalog = VirtualCopyCatalog::new();
    catalog
        .apply(Revision::ZERO, VirtualCopyCommand::Create(first))
        .unwrap();
    catalog
        .apply(Revision::from_u64(1), VirtualCopyCommand::Create(second))
        .unwrap();

    let first_id = VirtualCopyId::new(1).unwrap();
    let second_id = VirtualCopyId::new(2).unwrap();
    catalog
        .apply(
            Revision::from_u64(2),
            VirtualCopyCommand::ReplaceEdit {
                id: first_id,
                expected_revision: Revision::ZERO,
                replacement: edit(first_id, 100, 1, 9),
            },
        )
        .unwrap();

    assert_eq!(
        catalog.copy(first_id).unwrap().source().asset_id(),
        AssetId::new(50).unwrap()
    );
    assert_eq!(
        catalog.copy(second_id).unwrap().source().asset_id(),
        AssetId::new(50).unwrap()
    );
    assert_eq!(catalog.copy(first_id).unwrap().history().count(), 2);
    assert_eq!(catalog.copy(second_id).unwrap().history().count(), 1);
    assert_eq!(
        catalog.copy(second_id).unwrap().current_edit().revision(),
        Revision::ZERO
    );
}

#[test]
fn ordering_and_deletion_are_deterministic_and_tombstoned() {
    let mut catalog = VirtualCopyCatalog::new();
    for (id, order) in [(3, 4), (1, 4), (2, 1)] {
        let revision = catalog.revision();
        catalog
            .apply(
                revision,
                VirtualCopyCommand::Create(copy(id, 50, order, 100 + id)),
            )
            .unwrap();
    }
    let ids = catalog
        .projections()
        .into_iter()
        .map(|value| value.id().get())
        .collect::<Vec<_>>();
    assert_eq!(ids, [2, 1, 3]);

    catalog
        .apply(
            catalog.revision(),
            VirtualCopyCommand::Reorder {
                id: VirtualCopyId::new(3).unwrap(),
                before: Some(VirtualCopyId::new(2).unwrap()),
            },
        )
        .unwrap();
    assert_eq!(
        catalog
            .projections()
            .into_iter()
            .map(|value| value.id().get())
            .collect::<Vec<_>>(),
        [3, 2, 1]
    );

    catalog
        .apply(
            catalog.revision(),
            VirtualCopyCommand::Delete {
                id: VirtualCopyId::new(2).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        catalog
            .projections()
            .into_iter()
            .map(|value| value.id().get())
            .collect::<Vec<_>>(),
        [3, 1]
    );
    assert!(
        catalog
            .all_projections()
            .iter()
            .any(|value| value.id().get() == 2 && value.is_deleted())
    );
}

#[test]
fn invalid_revision_is_rejected_without_mutating_state() {
    let mut catalog = VirtualCopyCatalog::new();
    let id = VirtualCopyId::new(1).unwrap();
    catalog
        .apply(
            Revision::ZERO,
            VirtualCopyCommand::Create(copy(1, 50, 0, 100)),
        )
        .unwrap();
    let before = catalog.clone();
    let error = catalog
        .apply(
            catalog.revision(),
            VirtualCopyCommand::ReplaceEdit {
                id,
                expected_revision: Revision::from_u64(9),
                replacement: edit(id, 100, 1, 2),
            },
        )
        .unwrap_err();
    assert!(matches!(
        error,
        rusttable_catalog::VirtualCopyError::EditRevisionConflict { .. }
    ));
    assert_eq!(catalog, before);
}
