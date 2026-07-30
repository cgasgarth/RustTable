mod support;

use redb::{Database, ReadableTable, TableDefinition};
use rusttable_catalog::{
    HistoryApplyOutcome, HistoryBranchId, HistoryCommand, HistoryOperationKind,
    HistoryOperationSummary, HistoryPayload, HistoryRepository, HistoryRepositoryError,
    HistoryRevisionId, HistorySnapshotDocument, HistorySnapshotId, HistorySnapshotOperation,
    HistorySnapshotRepository, HistoryState, HistoryVersion,
};
use rusttable_catalog_store::RedbHistoryRepository;
use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, ParameterName, ParameterValue, PhotoId,
    Revision,
};

const HISTORY_SNAPSHOTS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_snapshots");

fn payload(edit_id: u128, operation_id: u128, key: &str, enabled: bool) -> HistoryPayload {
    let operation = Operation::new(
        OperationId::new(operation_id).expect("operation ID"),
        OperationKey::new(key).expect("operation key"),
        enabled,
        [(
            ParameterName::new("amount").expect("parameter name"),
            ParameterValue::Integer(i64::try_from(edit_id).expect("test integer")),
        )],
    )
    .expect("operation");
    let edit = Edit::new(
        EditId::new(edit_id).expect("edit ID"),
        PhotoId::new(7).expect("photo ID"),
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    HistoryPayload::new(
        edit,
        [0x12, 0x34],
        [u8::try_from(edit_id).expect("pipeline byte")],
        HistoryOperationSummary::new(
            HistoryOperationKind::Parameter,
            None,
            None,
            format!("operation {edit_id}"),
        )
        .expect("summary"),
    )
}

fn snapshot_with_undo() -> (HistoryState, HistorySnapshotDocument) {
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    for (edit_id, operation_id, key, enabled) in [
        (1, 11, "exposure.base", false),
        (2, 12, "color.balance", true),
        (3, 13, "tone.curve", true),
    ] {
        let outcome = state
            .apply(
                state.version(),
                HistoryCommand::Append {
                    payload: payload(edit_id, operation_id, key, enabled),
                },
            )
            .expect("append");
        assert!(matches!(outcome, HistoryApplyOutcome::Appended { .. }));
    }
    state
        .apply(state.version(), HistoryCommand::Undo)
        .expect("undo");
    let branch = state
        .branch(state.active_branch_id())
        .expect("active branch");
    let revisions = state.revisions().cloned().collect::<Vec<_>>();
    let snapshot = HistorySnapshotDocument::capture(
        photo,
        HistorySnapshotId::new(5).expect("snapshot ID"),
        branch,
        &revisions,
    )
    .expect("snapshot");
    (state, snapshot)
}

fn snapshot_key(photo: PhotoId, snapshot: HistorySnapshotId) -> [u8; 24] {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&photo.get().to_be_bytes());
    key[16..].copy_from_slice(&snapshot.get().to_be_bytes());
    key
}

#[test]
fn persisted_snapshot_preserves_cursor_order_filter_and_hash_after_restart() {
    let path = support::temp_path("history-snapshot-restart");
    let photo = PhotoId::new(7).expect("photo ID");
    let (state, original) = snapshot_with_undo();
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        repository
            .commit(HistoryVersion::ZERO, &state)
            .expect("history state");
        repository.store_snapshot(&original).expect("store");
        let stored = repository
            .load_snapshot(original.snapshot_id())
            .expect("load")
            .expect("snapshot");
        assert_eq!(stored, original);
        assert_eq!(stored.history_end(), 2);
        assert_eq!(
            stored.current_revision().map(HistoryRevisionId::get),
            Some(2)
        );
        assert_eq!(stored.current_history().count(), 2);
        assert_eq!(
            stored
                .enabled_history()
                .map(HistorySnapshotOperation::key)
                .collect::<Vec<_>>(),
            ["color.balance"]
        );
        assert_eq!(stored.stable_hash(), original.stable_hash());
    }
    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    let recovered = repository
        .load_snapshot(original.snapshot_id())
        .expect("load after restart")
        .expect("snapshot after restart");
    assert_eq!(recovered, original);
    assert_eq!(recovered.stable_hash(), original.stable_hash());
    support::remove(&path);
}

#[test]
fn pruning_history_commit_invalidates_durable_snapshots_before_restart() {
    let path = support::temp_path("history-snapshot-prune-restart");
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    state
        .apply(
            state.version(),
            HistoryCommand::Append {
                payload: payload(1, 11, "exposure.base", true),
            },
        )
        .expect("base revision");
    let base = state.active_cursor();
    let branch = match state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "temporary".to_owned(),
                from: Some(base),
            },
        )
        .expect("branch")
    {
        HistoryApplyOutcome::BranchCreated { branch } => branch,
        outcome => panic!("unexpected branch outcome: {outcome:?}"),
    };
    let removed_revision = match state
        .apply(
            state.version(),
            HistoryCommand::Append {
                payload: payload(2, 12, "color.balance", true),
            },
        )
        .expect("branch revision")
    {
        HistoryApplyOutcome::Appended { revision } => revision,
        outcome => panic!("unexpected append outcome: {outcome:?}"),
    };
    let branch_state = state.clone();
    let branch_ref = branch_state
        .branch(branch_state.active_branch_id())
        .expect("active branch");
    let snapshot = HistorySnapshotDocument::capture(
        photo,
        HistorySnapshotId::new(17).expect("snapshot ID"),
        branch_ref,
        &branch_state.revisions().cloned().collect::<Vec<_>>(),
    )
    .expect("snapshot");

    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        repository
            .commit(HistoryVersion::ZERO, &state)
            .expect("history state");
        repository
            .store_snapshot(&snapshot)
            .expect("store snapshot");
        assert_eq!(
            repository
                .load_snapshot(snapshot.snapshot_id())
                .expect("load snapshot"),
            Some(snapshot.clone())
        );

        state
            .apply(
                state.version(),
                HistoryCommand::SwitchBranch {
                    branch: HistoryBranchId::new(1).expect("main branch"),
                },
            )
            .expect("switch branch");
        state
            .apply(state.version(), HistoryCommand::DeleteBranch { branch })
            .expect("delete branch");
        state
            .apply(state.version(), HistoryCommand::PruneOrphans)
            .expect("prune");
        assert!(state.revision(removed_revision).is_none());
        repository
            .commit(branch_state.version(), &state)
            .expect("pruned history state");
    }

    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert!(repository.load().expect("load history").is_some());
    assert_eq!(
        repository
            .load_snapshot(snapshot.snapshot_id())
            .expect("load snapshot after restart"),
        None
    );
    support::remove(&path);
}

#[test]
fn malformed_persisted_snapshot_is_rejected_without_partial_decode() {
    let path = support::temp_path("history-snapshot-corrupt");
    let photo = PhotoId::new(7).expect("photo ID");
    let (state, snapshot) = snapshot_with_undo();
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        repository
            .commit(HistoryVersion::ZERO, &state)
            .expect("history state");
        repository.store_snapshot(&snapshot).expect("store");
    }
    let database = Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut table = transaction.open_table(HISTORY_SNAPSHOTS).expect("table");
    let key = snapshot_key(photo, snapshot.snapshot_id());
    table
        .insert(key.as_slice(), &b"malformed"[..])
        .expect("corrupt value");
    drop(table);
    transaction.commit().expect("commit corrupt fixture");
    drop(database);

    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert_eq!(
        repository.load_snapshot(snapshot.snapshot_id()),
        Err(HistoryRepositoryError::CorruptPersistedData)
    );
    support::remove(&path);
}

#[test]
fn snapshot_revision_reference_must_survive_restart_in_the_current_graph() {
    let path = support::temp_path("history-snapshot-graph-reference");
    let photo = PhotoId::new(7).expect("photo ID");
    let (state, snapshot) = snapshot_with_undo();
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        repository
            .commit(HistoryVersion::ZERO, &state)
            .expect("history state");
        repository.store_snapshot(&snapshot).expect("store");
    }
    let database = Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut table = transaction.open_table(HISTORY_SNAPSHOTS).expect("table");
    let key = snapshot_key(photo, snapshot.snapshot_id());
    let mut bytes = table
        .get(key.as_slice())
        .expect("snapshot row")
        .expect("snapshot value")
        .value()
        .to_vec();
    let first_revision_id_offset = 8 + 2 + 16 + 8 + 8 + 4;
    bytes[first_revision_id_offset..first_revision_id_offset + 8]
        .copy_from_slice(&99_u64.to_le_bytes());
    table
        .insert(key.as_slice(), bytes.as_slice())
        .expect("corrupt reference");
    drop(table);
    transaction.commit().expect("commit corrupt fixture");
    drop(database);

    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert_eq!(
        repository.load_snapshot(snapshot.snapshot_id()),
        Err(HistoryRepositoryError::CorruptPersistedData)
    );
    support::remove(&path);
}

#[test]
fn failed_snapshot_replace_and_clear_leave_the_committed_value_untouched() {
    let path = support::temp_path("history-snapshot-transaction");
    let photo = PhotoId::new(7).expect("photo ID");
    let (state, original) = snapshot_with_undo();
    let mut replacement_state = state.clone();
    replacement_state
        .apply(replacement_state.version(), HistoryCommand::Undo)
        .expect("second undo");
    let replacement_branch = replacement_state
        .branch(replacement_state.active_branch_id())
        .expect("replacement branch");
    let replacement_revisions = replacement_state.revisions().cloned().collect::<Vec<_>>();
    let replacement = HistorySnapshotDocument::capture(
        photo,
        original.snapshot_id(),
        replacement_branch,
        &replacement_revisions,
    )
    .expect("replacement snapshot");
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        repository
            .commit(HistoryVersion::ZERO, &state)
            .expect("history state");
        repository
            .store_snapshot(&original)
            .expect("store original");
    }
    let mut failing = RedbHistoryRepository::open_with_before_commit_hook(&path, photo, || {
        Err(HistoryRepositoryError::CommitFailure)
    })
    .expect("failing repository");

    assert_eq!(
        failing.store_snapshot(&replacement),
        Err(HistoryRepositoryError::CommitFailure)
    );
    assert_eq!(
        failing
            .load_snapshot(original.snapshot_id())
            .expect("load after failed replace"),
        Some(original.clone())
    );
    assert_eq!(
        failing.clear_snapshot(original.snapshot_id()),
        Err(HistoryRepositoryError::CommitFailure)
    );
    assert_eq!(
        failing
            .load_snapshot(original.snapshot_id())
            .expect("load after failed clear"),
        Some(original)
    );
    support::remove(&path);
}

#[test]
fn snapshot_for_another_photo_is_rejected_before_persistence() {
    let path = support::temp_path("history-snapshot-photo-boundary");
    let photo = PhotoId::new(8).expect("photo ID");
    let (_, snapshot) = snapshot_with_undo();
    let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
    assert_eq!(
        repository.store_snapshot(&snapshot),
        Err(HistoryRepositoryError::CorruptPersistedData)
    );
    assert_eq!(
        repository
            .load_snapshot(snapshot.snapshot_id())
            .expect("load"),
        None
    );
    support::remove(&path);
}
