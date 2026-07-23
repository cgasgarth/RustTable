mod support;

use rusttable_catalog::{
    ImportRepository, SourceAssetIdentity, VirtualCopy, VirtualCopyCommand, VirtualCopyId,
    VirtualCopyRepository, VirtualCopyRepositoryError,
};
use rusttable_catalog_store::{RedbImportRepository, RedbVirtualCopyRepository};
use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, ParameterName, ParameterValue, Revision,
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
        SourceAssetIdentity::new(
            rusttable_core::PhotoId::new(10).unwrap(),
            rusttable_core::AssetId::new(asset).unwrap(),
        ),
        order,
        edit(id, edit_id, 0, 1),
    )
    .unwrap()
}

#[test]
fn shared_source_and_independent_history_survive_restart() {
    let path = support::temp_path("virtual-copy-restart");
    let source = support::record("source/one.raw", 10, 50, 9);
    {
        let mut imports = RedbImportRepository::open(&path).unwrap();
        imports.commit(&source).unwrap();
        let mut repository = RedbVirtualCopyRepository::open(&path).unwrap();
        repository
            .apply(
                Revision::ZERO,
                VirtualCopyCommand::Create(copy(1, 50, 1, 101)),
            )
            .unwrap();
        repository
            .apply(
                Revision::from_u64(1),
                VirtualCopyCommand::Create(copy(2, 50, 2, 201)),
            )
            .unwrap();
        let id = VirtualCopyId::new(1).unwrap();
        repository
            .apply(
                Revision::from_u64(2),
                VirtualCopyCommand::ReplaceEdit {
                    id,
                    expected_revision: Revision::ZERO,
                    replacement: edit(id, 101, 1, 8),
                },
            )
            .unwrap();
        repository
            .apply(
                Revision::from_u64(3),
                VirtualCopyCommand::Reorder {
                    id: VirtualCopyId::new(2).unwrap(),
                    before: Some(id),
                },
            )
            .unwrap();
        repository
            .apply(Revision::from_u64(4), VirtualCopyCommand::Delete { id })
            .unwrap();
    }

    let repository = RedbVirtualCopyRepository::open(&path).unwrap();
    let active = repository.projections().unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].id().get(), 2);
    assert_eq!(active[0].source().asset_id().get(), 50);
    assert_eq!(repository.all_projections().unwrap().len(), 2);
    assert_eq!(
        repository
            .copy(VirtualCopyId::new(1).unwrap())
            .unwrap()
            .unwrap()
            .history()
            .count(),
        2
    );
    assert_eq!(repository.load().unwrap().revision(), Revision::from_u64(5));
    support::remove(&path);
}

#[test]
fn invalid_source_and_precommit_failure_leave_state_unchanged() {
    let path = support::temp_path("virtual-copy-rollback");
    let mut repository = RedbVirtualCopyRepository::open(&path).unwrap();
    let error = repository
        .apply(
            Revision::ZERO,
            VirtualCopyCommand::Create(copy(1, 99, 0, 101)),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        VirtualCopyRepositoryError::SourceAssetNotFound { .. }
    ));

    let source = support::record("source/rollback.raw", 10, 50, 9);
    {
        let mut imports = RedbImportRepository::open(&path).unwrap();
        imports.commit(&source).unwrap();
    }
    let mut repository = RedbVirtualCopyRepository::open_with_before_commit_hook(&path, || {
        Err(VirtualCopyRepositoryError::CommitFailed)
    })
    .unwrap();
    assert_eq!(
        repository.apply(
            Revision::ZERO,
            VirtualCopyCommand::Create(copy(1, 50, 0, 101))
        ),
        Err(VirtualCopyRepositoryError::CommitFailed)
    );
    drop(repository);
    let repository = RedbVirtualCopyRepository::open(&path).unwrap();
    assert!(repository.projections().unwrap().is_empty());
    support::remove(&path);
}
