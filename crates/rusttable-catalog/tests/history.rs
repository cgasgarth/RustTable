use rusttable_catalog::{
    BranchTransferPolicy, HistoryApplyOutcome, HistoryCommand, HistoryComparisonPair, HistoryError,
    HistoryOperationKind, HistoryOperationSummary, HistoryPayload, HistoryState,
};
use rusttable_core::{Edit, EditId, PhotoId};

fn edit(id: u128, photo_id: u128) -> Edit {
    Edit::new(
        EditId::new(id).expect("edit ID"),
        PhotoId::new(photo_id).expect("photo ID"),
        rusttable_core::Revision::ZERO,
        [],
    )
    .expect("edit")
}

fn payload(id: u128, kind: HistoryOperationKind) -> HistoryPayload {
    HistoryPayload::new(
        edit(id, 7),
        [u8::try_from(id & 0xff).unwrap(), 0x5a],
        id.to_be_bytes().to_vec(),
        HistoryOperationSummary::new(kind, None, None, format!("{kind:?} {id}")).unwrap(),
    )
}

fn append(state: &mut HistoryState, id: u128, kind: HistoryOperationKind) -> u64 {
    let outcome = state
        .apply(
            state.version(),
            HistoryCommand::Append {
                payload: payload(id, kind),
            },
        )
        .unwrap();
    let HistoryApplyOutcome::Appended { revision } = outcome else {
        panic!("append outcome")
    };
    revision.get()
}

#[test]
fn linear_undo_redo_moves_cursor_without_rewriting_payloads() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let first = append(&mut state, 1, HistoryOperationKind::Parameter);
    let second = append(&mut state, 2, HistoryOperationKind::Order);
    let original_first = state
        .revision(rusttable_catalog::HistoryRevisionId::new(first).unwrap())
        .unwrap()
        .clone();
    let original_second = state
        .revision(rusttable_catalog::HistoryRevisionId::new(second).unwrap())
        .unwrap()
        .clone();

    state.apply(state.version(), HistoryCommand::Undo).unwrap();
    assert_eq!(state.current_revision().unwrap().id().get(), first);
    state.apply(state.version(), HistoryCommand::Redo).unwrap();
    assert_eq!(state.current_revision().unwrap().id().get(), second);
    assert_eq!(state.revision(original_first.id()), Some(&original_first));
    assert_eq!(state.revision(original_second.id()), Some(&original_second));
}

#[test]
fn edit_after_undo_forks_branch_and_preserves_redo_lineage() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let first = append(&mut state, 1, HistoryOperationKind::Parameter);
    let second = append(&mut state, 2, HistoryOperationKind::Enable);
    state.apply(state.version(), HistoryCommand::Undo).unwrap();
    append(&mut state, 3, HistoryOperationKind::Reset);

    assert_ne!(state.active_branch_id().get(), 1);
    assert_eq!(
        state
            .current_revision()
            .unwrap()
            .payload()
            .edit()
            .id()
            .get(),
        3
    );
    let main = state
        .branch(rusttable_catalog::HistoryBranchId::new(1).unwrap())
        .unwrap();
    assert_eq!(main.cursor().unwrap().get(), first);
    assert_eq!(
        main.redo(),
        &[rusttable_catalog::HistoryRevisionId::new(second).unwrap()]
    );
}

#[test]
fn snapshots_evidence_and_pruning_protect_referenced_history() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    append(&mut state, 1, HistoryOperationKind::Style);
    let created = state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "experiment".to_owned(),
                from: None,
            },
        )
        .unwrap();
    let HistoryApplyOutcome::BranchCreated { branch } = created else {
        panic!("branch outcome")
    };
    let unique = append(&mut state, 2, HistoryOperationKind::Mask);
    let snapshot = state
        .apply(
            state.version(),
            HistoryCommand::CreateSnapshot {
                name: "mask-check".to_owned(),
            },
        )
        .unwrap();
    let HistoryApplyOutcome::SnapshotCreated { snapshot } = snapshot else {
        panic!("snapshot outcome")
    };
    let main = rusttable_catalog::HistoryBranchId::new(1).unwrap();
    state
        .apply(
            state.version(),
            HistoryCommand::SwitchBranch { branch: main },
        )
        .unwrap();
    assert_eq!(
        state.apply(state.version(), HistoryCommand::DeleteBranch { branch },),
        Err(HistoryError::BranchHasSnapshot(branch))
    );
    state
        .apply(state.version(), HistoryCommand::DeleteSnapshot { snapshot })
        .unwrap();
    state
        .apply(
            state.version(),
            HistoryCommand::RetainEvidence {
                revision: rusttable_catalog::HistoryRevisionId::new(unique).unwrap(),
                kind: rusttable_catalog::HistoryEvidenceKind::Export,
            },
        )
        .unwrap();
    assert_eq!(
        state.apply(state.version(), HistoryCommand::DeleteBranch { branch },),
        Err(HistoryError::BranchHasEvidence(branch))
    );
    state
        .apply(
            state.version(),
            HistoryCommand::ReleaseEvidence {
                revision: rusttable_catalog::HistoryRevisionId::new(unique).unwrap(),
                kind: rusttable_catalog::HistoryEvidenceKind::Export,
            },
        )
        .unwrap();
    state
        .apply(state.version(), HistoryCommand::DeleteBranch { branch })
        .unwrap();
    let pruned = state
        .apply(state.version(), HistoryCommand::PruneOrphans)
        .unwrap();
    assert_eq!(pruned, HistoryApplyOutcome::Pruned { removed: 1 });
    assert!(
        state
            .revision(rusttable_catalog::HistoryRevisionId::new(unique).unwrap())
            .is_none()
    );
}

#[test]
fn branch_transfer_has_explicit_copy_and_merge_policy() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    append(&mut state, 1, HistoryOperationKind::Parameter);
    let source = state.active_cursor();
    state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "copy".to_owned(),
                from: Some(source),
            },
        )
        .unwrap();
    append(&mut state, 2, HistoryOperationKind::Blend);
    let source = state.active_cursor();
    state
        .apply(
            state.version(),
            HistoryCommand::SwitchBranch {
                branch: source.branch(),
            },
        )
        .unwrap();
    let outcome = state
        .apply(
            state.version(),
            HistoryCommand::Transfer {
                source,
                policy: BranchTransferPolicy::Copy,
            },
        )
        .unwrap();
    let HistoryApplyOutcome::Appended { revision } = outcome else {
        panic!("copy outcome")
    };
    assert_eq!(
        state.revision(revision).unwrap().payload().summary().kind(),
        HistoryOperationKind::Copy
    );
    let merge = state
        .apply(
            state.version(),
            HistoryCommand::Transfer {
                source,
                policy: BranchTransferPolicy::Merge,
            },
        )
        .unwrap();
    let HistoryApplyOutcome::Appended { revision } = merge else {
        panic!("merge outcome")
    };
    assert_eq!(
        state.revision(revision).unwrap().payload().summary().kind(),
        HistoryOperationKind::Merge
    );
}

#[test]
fn comparison_pair_keeps_before_and_after_cursors_typed() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let before = state.active_cursor();
    append(&mut state, 1, HistoryOperationKind::Parameter);
    let after = state.active_cursor();
    let pair = HistoryComparisonPair::new(before, after);
    assert_eq!(pair.before(), before);
    assert_eq!(pair.after(), after);
}

#[test]
fn stale_history_commands_are_rejected_before_mutation() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    append(&mut state, 1, HistoryOperationKind::Parameter);
    let before = state.clone();
    let result = state.apply(
        rusttable_catalog::HistoryVersion::ZERO,
        HistoryCommand::Undo,
    );
    assert!(matches!(result, Err(HistoryError::VersionConflict { .. })));
    assert_eq!(state, before);
}
