use rusttable_catalog::{
    BranchTransferPolicy, HistoryApplyOutcome, HistoryBranch, HistoryCommand,
    HistoryComparisonPair, HistoryCursor, HistoryError, HistoryJournalEntry, HistoryOperationKind,
    HistoryOperationSummary, HistoryPayload, HistoryProvenance, HistoryRevisionId, HistoryState,
    HistoryStateSnapshot,
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

fn rebuild(
    state: &HistoryState,
    branches: Vec<HistoryBranch>,
    snapshots: Vec<rusttable_catalog::HistorySnapshot>,
    journal: Vec<HistoryJournalEntry>,
    provenance: std::collections::BTreeMap<HistoryRevisionId, HistoryProvenance>,
) -> HistoryStateSnapshot {
    let persisted = state.persistence_snapshot();
    HistoryStateSnapshot::from_parts_with_journal(
        persisted.photo_id(),
        persisted.version(),
        persisted.commit_sequence(),
        persisted.next_revision_id(),
        persisted.next_branch_id(),
        persisted.next_snapshot_id(),
        persisted.active_branch(),
        persisted.revisions().to_vec(),
        branches,
        snapshots,
        persisted.evidence().to_vec(),
        journal,
        provenance,
    )
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

#[test]
fn persisted_graph_rejects_empty_cursors_and_invalid_branch_origins() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let first = append(&mut state, 1, HistoryOperationKind::Parameter);
    let main = rusttable_catalog::HistoryBranchId::new(1).unwrap();
    let mut branches = state.branches().cloned().collect::<Vec<_>>();
    branches[0] = HistoryBranch::from_parts(
        main,
        "main".to_owned(),
        None,
        vec![HistoryRevisionId::new(first).unwrap()],
        None,
        Vec::new(),
    );
    assert_eq!(
        HistoryState::restore(rebuild(
            &state,
            branches,
            Vec::new(),
            state.journal().cloned().collect(),
            state.persistence_snapshot().provenance().clone(),
        )),
        Err(HistoryError::InvalidPersistedState)
    );

    let persisted = state.persistence_snapshot();
    let empty_cursor_snapshot = rusttable_catalog::HistorySnapshot::from_parts(
        rusttable_catalog::HistorySnapshotId::new(1).unwrap(),
        "empty-cursor".to_owned(),
        HistoryCursor::new(main, None),
    );
    assert_eq!(
        HistoryState::restore(rebuild(
            &state,
            persisted.branches().to_vec(),
            vec![empty_cursor_snapshot],
            persisted.journal().to_vec(),
            persisted.provenance().clone(),
        )),
        Err(HistoryError::InvalidPersistedState)
    );

    state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "experiment".to_owned(),
                from: Some(state.active_cursor()),
            },
        )
        .unwrap();
    let branch_id = state.active_branch_id();
    let mut branches = state.branches().cloned().collect::<Vec<_>>();
    let branch = branches
        .iter_mut()
        .find(|branch| branch.id() == branch_id)
        .unwrap();
    *branch = HistoryBranch::from_parts(
        branch.id(),
        branch.name().to_owned(),
        HistoryRevisionId::new(99),
        branch.lineage().to_vec(),
        branch.cursor(),
        branch.redo().to_vec(),
    );
    assert_eq!(
        HistoryState::restore(rebuild(
            &state,
            branches,
            Vec::new(),
            state.journal().cloned().collect(),
            state.persistence_snapshot().provenance().clone(),
        )),
        Err(HistoryError::InvalidPersistedState)
    );
}

#[test]
fn persisted_journal_rejects_dangling_cursor_restore_and_provenance_references() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let first = append(&mut state, 1, HistoryOperationKind::Parameter);
    let main = rusttable_catalog::HistoryBranchId::new(1).unwrap();
    let persisted = state.persistence_snapshot();
    let corrupt_journal = vec![HistoryJournalEntry::new(
        1,
        HistoryOperationKind::Parameter,
        Some(HistoryRevisionId::new(first).unwrap()),
        HistoryCursor::new(main, HistoryRevisionId::new(99)),
        HistoryCursor::new(main, HistoryRevisionId::new(first)),
        HistoryRevisionId::new(99),
        HistoryProvenance::native(),
    )];
    assert_eq!(
        HistoryState::restore(rebuild(
            &state,
            persisted.branches().to_vec(),
            persisted.snapshots().to_vec(),
            corrupt_journal,
            persisted.provenance().clone(),
        )),
        Err(HistoryError::InvalidPersistedState)
    );

    let mut provenance = persisted.provenance().clone();
    provenance.insert(
        HistoryRevisionId::new(99).unwrap(),
        HistoryProvenance::native(),
    );
    assert_eq!(
        HistoryState::restore(rebuild(
            &state,
            persisted.branches().to_vec(),
            persisted.snapshots().to_vec(),
            persisted.journal().to_vec(),
            provenance,
        )),
        Err(HistoryError::InvalidPersistedState)
    );
}

#[test]
fn persisted_journal_restore_references_must_be_prior_revisions() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    let first = append(&mut state, 1, HistoryOperationKind::Parameter);
    let second = append(&mut state, 2, HistoryOperationKind::Parameter);
    let main = main_branch();
    let persisted = state.persistence_snapshot();
    let before = HistoryCursor::new(main, None);
    let after = HistoryCursor::new(main, HistoryRevisionId::new(first));

    for restore_from in [first, second] {
        let journal = vec![HistoryJournalEntry::new(
            1,
            HistoryOperationKind::Reset,
            Some(HistoryRevisionId::new(first).unwrap()),
            before,
            after,
            Some(HistoryRevisionId::new(restore_from).unwrap()),
            HistoryProvenance::native(),
        )];
        assert_eq!(
            HistoryState::restore(rebuild(
                &state,
                persisted.branches().to_vec(),
                persisted.snapshots().to_vec(),
                journal,
                persisted.provenance().clone(),
            )),
            Err(HistoryError::InvalidPersistedState)
        );
    }

    state
        .apply(
            state.version(),
            HistoryCommand::Restore {
                source: HistoryRevisionId::new(first).unwrap(),
            },
        )
        .unwrap();
    assert_eq!(
        state
            .journal()
            .last()
            .and_then(HistoryJournalEntry::restore_from),
        Some(HistoryRevisionId::new(first).unwrap())
    );
    assert!(HistoryState::restore(state.persistence_snapshot()).is_ok());
}

#[test]
fn branch_deletion_and_pruning_leave_restartable_journal_and_provenance() {
    let mut state = HistoryState::new(PhotoId::new(7).unwrap());
    append(&mut state, 1, HistoryOperationKind::Parameter);
    let base = state.active_cursor();
    let branch = match state
        .apply(
            state.version(),
            HistoryCommand::CreateBranch {
                name: "experiment".to_owned(),
                from: Some(base),
            },
        )
        .unwrap()
    {
        HistoryApplyOutcome::BranchCreated { branch } => branch,
        outcome => panic!("unexpected branch outcome: {outcome:?}"),
    };
    let orphaned = append(&mut state, 2, HistoryOperationKind::Mask);
    state
        .apply(
            state.version(),
            HistoryCommand::SwitchBranch {
                branch: main_branch(),
            },
        )
        .unwrap();
    let persisted = state.persistence_snapshot();
    let mut provenance = persisted.provenance().clone();
    provenance.insert(
        HistoryRevisionId::new(orphaned).unwrap(),
        HistoryProvenance::darktable(1, "branch-row"),
    );
    let seeded = rebuild(
        &state,
        persisted.branches().to_vec(),
        persisted.snapshots().to_vec(),
        persisted.journal().to_vec(),
        provenance,
    );
    state = HistoryState::restore(seeded).unwrap();

    state
        .apply(state.version(), HistoryCommand::DeleteBranch { branch })
        .unwrap();
    state
        .apply(state.version(), HistoryCommand::PruneOrphans)
        .unwrap();

    assert!(
        state
            .provenance(HistoryRevisionId::new(orphaned).unwrap())
            .is_none()
    );
    assert!(
        state
            .journal()
            .all(|entry| { entry.before().branch() != branch && entry.after().branch() != branch })
    );
    assert!(HistoryState::restore(state.persistence_snapshot()).is_ok());
}

fn main_branch() -> rusttable_catalog::HistoryBranchId {
    rusttable_catalog::HistoryBranchId::new(1).unwrap()
}
