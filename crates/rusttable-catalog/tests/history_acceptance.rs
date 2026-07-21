use rusttable_catalog::{
    BranchTransferPolicy, CanonicalHistoryCommand, HistoryApplyOutcome, HistoryCommand,
    HistoryEvidenceKind, HistoryImport, HistoryImportEntry, HistoryImportSource,
    HistoryOperationKind, HistoryOperationSummary, HistoryPayload, HistoryState,
};
use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, ParameterName, ParameterValue, PhotoId,
    Revision,
};

fn edit(id: u128, value: i64) -> Edit {
    Edit::new(
        EditId::new(id).expect("edit ID"),
        PhotoId::new(7).expect("photo ID"),
        Revision::ZERO,
        [Operation::new(
            OperationId::new(1).expect("operation ID"),
            OperationKey::new("rusttable.exposure").expect("operation key"),
            true,
            [(
                ParameterName::new("stops").expect("parameter name"),
                ParameterValue::Integer(value),
            )],
        )
        .expect("operation")],
    )
    .expect("edit")
}

fn payload(id: u128, value: i64) -> HistoryPayload {
    HistoryPayload::new(
        edit(id, value),
        [1, 2, 3],
        [4, 5, 6],
        HistoryOperationSummary::new(HistoryOperationKind::Parameter, None, None, "parameter")
            .expect("summary"),
    )
}

fn append(state: &mut HistoryState, payload: HistoryPayload) {
    state
        .apply(state.version(), HistoryCommand::Append { payload })
        .expect("append");
}

#[test]
fn canonical_commands_are_stable_and_journal_sequences_are_monotonic() {
    let first = CanonicalHistoryCommand::Enable {
        operation_id: OperationId::new(1).expect("operation ID"),
        enabled: true,
    }
    .canonical_bytes()
    .expect("canonical bytes");
    let second = CanonicalHistoryCommand::Enable {
        operation_id: OperationId::new(1).expect("operation ID"),
        enabled: true,
    }
    .canonical_bytes()
    .expect("canonical bytes");
    assert_eq!(first, second);

    let mut state = HistoryState::new(PhotoId::new(7).expect("photo ID"));
    append(&mut state, payload(1, 1));
    append(&mut state, payload(2, 2));
    state
        .apply(state.version(), HistoryCommand::Undo)
        .expect("undo");
    assert_eq!(state.commit_sequence(), 3);
    assert_eq!(state.current_pointer(), state.active_cursor());
    assert!(
        state
            .evidence()
            .any(|value| value.kind() == HistoryEvidenceKind::Restore)
    );
    assert_eq!(
        state
            .journal()
            .map(rusttable_catalog::HistoryJournalEntry::sequence)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

#[test]
fn restore_is_a_new_immutable_revision_and_copy_paste_is_explicit() {
    let mut state = HistoryState::new(PhotoId::new(7).expect("photo ID"));
    append(&mut state, payload(1, 1));
    let source = state.active_cursor();
    let copied = state
        .apply(state.version(), HistoryCommand::Copy { source })
        .expect("copy");
    assert_eq!(copied, HistoryApplyOutcome::Copied { source });
    let pasted = state
        .apply(state.version(), HistoryCommand::Paste { source })
        .expect("paste");
    let HistoryApplyOutcome::Appended { revision } = pasted else {
        panic!("paste did not append")
    };
    assert_eq!(
        state
            .revision(revision)
            .expect("revision")
            .payload()
            .summary()
            .kind(),
        HistoryOperationKind::Copy
    );
    let restored = state
        .apply(
            state.version(),
            HistoryCommand::Restore {
                source: source.revision().expect("source"),
            },
        )
        .expect("restore");
    let HistoryApplyOutcome::Appended {
        revision: restored_id,
    } = restored
    else {
        panic!("restore did not append")
    };
    assert_ne!(restored_id, source.revision().expect("source"));
    assert!(state.evidence().any(
        |value| value.revision() == source.revision().expect("source")
            && value.kind() == HistoryEvidenceKind::Restore
    ));
}

#[test]
fn merge_rejects_conflicting_operation_instances() {
    let mut state = HistoryState::new(PhotoId::new(7).expect("photo ID"));
    append(&mut state, payload(1, 1));
    let base = state.active_cursor();
    let HistoryApplyOutcome::BranchCreated { branch } = state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "source".to_owned(),
                from: Some(base),
            },
        )
        .expect("branch")
    else {
        panic!("branch did not create")
    };
    append(&mut state, payload(2, 2));
    let source = state.active_cursor();
    state
        .apply(
            state.version(),
            HistoryCommand::SwitchBranch {
                branch: rusttable_catalog::HistoryBranchId::new(1).expect("main"),
            },
        )
        .expect("switch");
    append(&mut state, payload(3, 3));
    let target = state.active_cursor();
    assert!(matches!(
        state.apply(state.version(), HistoryCommand::Merge { source, target }),
        Err(rusttable_catalog::HistoryError::MergeConflict { .. })
    ));
    assert!(state.branch(branch).is_some());
}

#[test]
fn import_reconstructs_current_and_redo_with_source_evidence() {
    let first = HistoryImportEntry::new(
        payload(1, 1),
        HistoryImportSource::Darktable {
            schema: 42,
            source_id: "history-row-1".to_owned(),
        },
        false,
        None,
    );
    let second = HistoryImportEntry::new(
        payload(2, 2),
        HistoryImportSource::RustTable {
            schema: 7,
            source_id: "revision-2".to_owned(),
        },
        true,
        Some(0),
    );
    let state = HistoryImport::new(
        PhotoId::new(7).expect("photo ID"),
        vec![first, second],
        Some(0),
    )
    .expect("import")
    .reconstruct()
    .expect("reconstruct");
    assert_eq!(
        state
            .current_revision()
            .expect("current")
            .payload()
            .edit()
            .id()
            .get(),
        1
    );
    assert_eq!(state.active_cursor().revision().expect("current").get(), 1);
    assert!(
        state
            .evidence()
            .any(|value| value.kind() == HistoryEvidenceKind::Redo)
    );
    assert!(
        state
            .evidence()
            .any(|value| value.kind() == HistoryEvidenceKind::Restore)
    );
    assert!(matches!(
        state
            .provenance(
                state
                    .revision(state.active_cursor().revision().expect("revision"))
                    .expect("revision")
                    .id()
            )
            .expect("provenance"),
        rusttable_catalog::HistoryProvenance::Darktable { .. }
    ));
}

#[test]
fn transfer_copy_policy_remains_compatible_with_the_explicit_commands() {
    let mut state = HistoryState::new(PhotoId::new(7).expect("photo ID"));
    append(&mut state, payload(1, 1));
    let source = state.active_cursor();
    let outcome = state
        .apply(
            state.version(),
            HistoryCommand::Transfer {
                source,
                policy: BranchTransferPolicy::Copy,
            },
        )
        .expect("copy transfer");
    assert!(matches!(outcome, HistoryApplyOutcome::Appended { .. }));
}
