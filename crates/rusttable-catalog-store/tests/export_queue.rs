mod support;

use rusttable_catalog_store::{
    ExportJobId, ExportJobPriority, ExportJobRecord, ExportJobStage, ExportJobState,
    RedbExportQueueStore,
};

#[test]
fn queue_persists_snapshot_idempotency_transitions_and_recovery() {
    let path = support::temp_path("export-queue");
    let store = RedbExportQueueStore::open(&path).unwrap();
    let id = ExportJobId::new(1).unwrap();
    let job = ExportJobRecord::new(
        id,
        [7; 32],
        "copy:stable".to_owned(),
        "local".to_owned(),
        "photo".to_owned(),
        b"immutable request snapshot".to_vec(),
        ExportJobPriority::Interactive,
        10,
    );
    assert_eq!(
        store.enqueue(job.clone()).unwrap().snapshot(),
        job.snapshot()
    );
    assert_eq!(store.enqueue(job).unwrap().id(), id);
    store.transition(id, ExportJobState::Preparing, 11).unwrap();
    store.transition(id, ExportJobState::Rendering, 12).unwrap();
    store
        .update_progress(id, ExportJobStage::Rendering, 1, 2, 13)
        .unwrap();
    assert!(
        store
            .update_progress(id, ExportJobStage::Rendering, 0, 2, 14)
            .is_err()
    );
    let recovered = store.recover_in_process(15).unwrap();
    assert_eq!(recovered[0].state(), ExportJobState::Interrupted);
    store.retry(id, 16).unwrap();
    assert_eq!(
        store.get(id).unwrap().unwrap().state(),
        ExportJobState::Queued
    );
    drop(store);
    let reopened = RedbExportQueueStore::open(&path).unwrap();
    assert_eq!(reopened.get(id).unwrap().unwrap().request_hash(), [7; 32]);
    support::remove(&path);
}

#[test]
fn queue_rejects_illegal_terminal_transition_and_conflicting_idempotency() {
    let path = support::temp_path("export-queue-conflict");
    let store = RedbExportQueueStore::open(&path).unwrap();
    let first = ExportJobRecord::new(
        ExportJobId::new(1).unwrap(),
        [1; 32],
        "same".to_owned(),
        "local".to_owned(),
        "target".to_owned(),
        vec![1],
        ExportJobPriority::Normal,
        1,
    );
    store.enqueue(first).unwrap();
    let conflicting = ExportJobRecord::new(
        ExportJobId::new(2).unwrap(),
        [2; 32],
        "same".to_owned(),
        "local".to_owned(),
        "target".to_owned(),
        vec![2],
        ExportJobPriority::Normal,
        1,
    );
    assert!(matches!(
        store.enqueue(conflicting),
        Err(rusttable_catalog_store::ExportQueueError::IdempotencyConflict { .. })
    ));
    store
        .transition(ExportJobId::new(1).unwrap(), ExportJobState::Preparing, 2)
        .unwrap();
    store
        .transition(ExportJobId::new(1).unwrap(), ExportJobState::Encoding, 3)
        .unwrap();
    store
        .transition(ExportJobId::new(1).unwrap(), ExportJobState::Committing, 4)
        .unwrap();
    store
        .succeed(ExportJobId::new(1).unwrap(), vec![9], 5)
        .unwrap();
    assert!(
        store
            .transition(ExportJobId::new(1).unwrap(), ExportJobState::Cancelled, 6)
            .is_err()
    );
    support::remove(&path);
}
