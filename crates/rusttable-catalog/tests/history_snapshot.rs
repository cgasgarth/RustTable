//! Focused pure tests for the history snapshot leaf.

use rusttable_catalog::{
    HistoryApplyOutcome, HistoryCommand, HistoryOperationKind, HistoryOperationSummary,
    HistoryPayload, HistoryRevisionId, HistorySnapshotDecodeError, HistorySnapshotDocument,
    HistorySnapshotId, HistorySnapshotRevision, HistoryState, MAX_HISTORY_SNAPSHOT_BYTES,
};
use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, ParameterName, ParameterValue, PhotoId,
    Revision,
};

const PHOTO_ID: u128 = 7;

fn payload(
    edit_id: u128,
    operation_id: u128,
    operation_key: &str,
    enabled: bool,
) -> HistoryPayload {
    let operation = Operation::new(
        OperationId::new(operation_id).expect("operation ID"),
        OperationKey::new(operation_key).expect("operation key"),
        enabled,
        [(
            ParameterName::new("amount").expect("parameter name"),
            ParameterValue::Integer(i64::try_from(edit_id).expect("test integer")),
        )],
    )
    .expect("operation");
    let edit = Edit::new(
        EditId::new(edit_id).expect("edit ID"),
        PhotoId::new(PHOTO_ID).expect("photo ID"),
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

fn append(state: &mut HistoryState, payload: HistoryPayload) {
    let outcome = state
        .apply(state.version(), HistoryCommand::Append { payload })
        .expect("append");
    assert!(matches!(outcome, HistoryApplyOutcome::Appended { .. }));
}

fn capture(state: &HistoryState) -> HistorySnapshotDocument {
    let branch = state
        .branch(state.active_branch_id())
        .expect("active branch");
    let revisions = state.revisions().cloned().collect::<Vec<_>>();
    HistorySnapshotDocument::capture(
        state.photo_id(),
        HistorySnapshotId::new(1).expect("snapshot ID"),
        branch,
        &revisions,
    )
    .expect("snapshot")
}

#[test]
fn empty_history_has_no_current_selection() {
    let state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    let snapshot = capture(&state);

    assert_eq!(snapshot.history_end(), 0);
    assert_eq!(
        snapshot.photo_id(),
        PhotoId::new(PHOTO_ID).expect("photo ID")
    );
    assert_eq!(snapshot.snapshot_id().get(), 1);
    assert_eq!(
        snapshot,
        HistorySnapshotDocument::empty(
            PhotoId::new(PHOTO_ID).expect("photo ID"),
            HistorySnapshotId::new(1).expect("snapshot ID"),
        )
    );
    assert_eq!(snapshot.current_revision(), None);
    assert_eq!(snapshot.current_history().count(), 0);
    assert_eq!(snapshot.enabled_history().count(), 0);
}

#[test]
fn current_selection_stops_at_undo_cursor() {
    let mut state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut state, payload(1, 11, "exposure.base", true));
    append(&mut state, payload(2, 12, "color.balance", true));
    append(&mut state, payload(3, 13, "tone.curve", true));
    state
        .apply(state.version(), HistoryCommand::Undo)
        .expect("undo");

    let snapshot = capture(&state);
    assert_eq!(snapshot.history_end(), 2);
    assert_eq!(
        snapshot.current_revision().map(HistoryRevisionId::get),
        Some(2)
    );
    assert_eq!(
        snapshot
            .revisions()
            .iter()
            .map(HistorySnapshotRevision::order)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(snapshot.current_history().count(), 2);
    assert_eq!(
        snapshot.revision(HistoryRevisionId::new(3).expect("revision")),
        None
    );
}

#[test]
fn disabled_operations_remain_ordered_but_enabled_history_filters_them() {
    let mut state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut state, payload(1, 11, "exposure.base", false));
    append(&mut state, payload(2, 12, "color.balance", true));

    let snapshot = capture(&state);
    let all = snapshot
        .current_history()
        .map(|operation| {
            (
                operation.operation_id().get(),
                operation.key().to_owned(),
                operation.is_enabled(),
            )
        })
        .collect::<Vec<_>>();
    let enabled = snapshot
        .enabled_history()
        .map(|operation| operation.key().to_owned())
        .collect::<Vec<_>>();

    assert_eq!(
        all,
        [
            (11, "exposure.base".to_owned(), false),
            (12, "color.balance".to_owned(), true)
        ]
    );
    assert_eq!(enabled, ["color.balance".to_owned()]);
}

#[test]
fn multiple_revisions_preserve_native_order_and_identity() {
    let mut state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut state, payload(1, 11, "exposure.base", true));
    append(&mut state, payload(2, 12, "color.balance", true));
    append(&mut state, payload(3, 13, "tone.curve", false));

    let snapshot = capture(&state);
    assert_eq!(
        snapshot
            .revisions()
            .iter()
            .map(|revision| revision.id().get())
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(snapshot.history_end(), 3);
    assert!(
        snapshot
            .revisions()
            .windows(2)
            .all(|pair| pair[0].order() < pair[1].order())
    );
    assert!(snapshot.revisions().iter().all(|revision| {
        revision.operations().len() == 1
            && !revision.canonical_payload().is_empty()
            && revision.payload_hash() != [0; 32]
    }));
    assert_eq!(snapshot.photo_id().get(), PHOTO_ID);
    assert_eq!(snapshot.snapshot_id().get(), 1);
}

#[test]
fn stable_hash_is_repeatable_and_covers_enabled_state() {
    let mut enabled_state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut enabled_state, payload(1, 11, "exposure.base", true));
    let enabled = capture(&enabled_state);
    let enabled_again = capture(&enabled_state);

    let mut disabled_state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut disabled_state, payload(1, 11, "exposure.base", false));
    let disabled = capture(&disabled_state);

    assert_eq!(
        enabled.stable_hash().expect("hash"),
        enabled_again.stable_hash().expect("hash")
    );
    assert_ne!(
        enabled.stable_hash().expect("hash"),
        disabled.stable_hash().expect("hash")
    );
}

#[test]
fn serialization_round_trip_preserves_current_and_enabled_history() {
    let mut state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut state, payload(1, 11, "exposure.base", false));
    append(&mut state, payload(2, 12, "color.balance", true));
    let original = capture(&state);
    let bytes = original.serialize().expect("serialize");

    let restored = HistorySnapshotDocument::deserialize(&bytes).expect("deserialize");
    assert_eq!(restored, original);
    assert_eq!(
        restored.stable_hash().expect("hash"),
        original.stable_hash().expect("hash")
    );
    assert_eq!(restored.current_revision(), original.current_revision());
    assert_eq!(restored.enabled_history().count(), 1);
}

#[test]
fn malformed_payloads_fail_closed() {
    let mut state = HistoryState::new(PhotoId::new(PHOTO_ID).expect("photo ID"));
    append(&mut state, payload(1, 11, "exposure.base", true));
    let bytes = capture(&state).serialize().expect("serialize");

    assert_eq!(
        HistorySnapshotDocument::deserialize(&bytes[..bytes.len() - 1]),
        Err(HistorySnapshotDecodeError::Truncated)
    );

    let mut bad_magic = bytes.clone();
    bad_magic[0] ^= 1;
    assert_eq!(
        HistorySnapshotDocument::deserialize(&bad_magic),
        Err(HistorySnapshotDecodeError::InvalidMagic)
    );

    let mut trailing = bytes.clone();
    trailing.push(0);
    assert_eq!(
        HistorySnapshotDocument::deserialize(&trailing),
        Err(HistorySnapshotDecodeError::TrailingBytes)
    );

    let mut bad_boolean = bytes.clone();
    *bad_boolean.last_mut().expect("enabled flag") = 2;
    assert_eq!(
        HistorySnapshotDocument::deserialize(&bad_boolean),
        Err(HistorySnapshotDecodeError::InvalidBoolean)
    );

    let mut bad_revision_payload = bytes.clone();
    let payload_marker = bad_revision_payload
        .windows(b"RTHREV\0\x01".len())
        .position(|window| window == b"RTHREV\0\x01")
        .expect("revision payload marker");
    bad_revision_payload[payload_marker + b"RTHREV\0\x01".len() + 4] ^= 1;
    assert_eq!(
        HistorySnapshotDocument::deserialize(&bad_revision_payload),
        Err(HistorySnapshotDecodeError::InvalidPayload)
    );

    let oversized = vec![0; MAX_HISTORY_SNAPSHOT_BYTES + 1];
    assert_eq!(
        HistorySnapshotDocument::deserialize(&oversized),
        Err(HistorySnapshotDecodeError::TooLarge {
            actual: MAX_HISTORY_SNAPSHOT_BYTES + 1,
        })
    );
}
