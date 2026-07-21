mod support;

use redb::ReadableTable;
use rusttable_catalog::{
    DurableHistoryService, HistoryCommand, HistoryOperationKind, HistoryOperationSummary,
    HistoryPayload, HistoryRepository, HistoryState,
};
use rusttable_catalog_store::RedbHistoryRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};

fn payload() -> HistoryPayload {
    HistoryPayload::new(
        Edit::new(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(7).expect("photo ID"),
            Revision::ZERO,
            [],
        )
        .expect("edit"),
        [9, 8, 7],
        [6, 5, 4],
        HistoryOperationSummary::new(HistoryOperationKind::Parameter, None, None, "parameter")
            .expect("summary"),
    )
}

#[test]
fn identical_revision_payloads_share_content_blobs_and_export_is_stable() {
    let path = support::temp_path("history-blobs");
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
    let expected = state.version();
    DurableHistoryService::apply(
        &mut state,
        expected,
        HistoryCommand::Append { payload: payload() },
        &mut repository,
    )
    .expect("first append");
    let expected = state.version();
    DurableHistoryService::apply(
        &mut state,
        expected,
        HistoryCommand::Append { payload: payload() },
        &mut repository,
    )
    .expect("second append");
    assert_eq!(repository.blob_count().expect("blob count"), 3);
    let first = repository.export_canonical().expect("export");
    let second = repository.export_canonical().expect("export");
    assert_eq!(first, second);
    drop(repository);
    support::remove(&path);
}

#[test]
fn restart_rejects_missing_content_blob_instead_of_substituting_neighboring_state() {
    let path = support::temp_path("history-missing-blob");
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    {
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
        let expected = state.version();
        DurableHistoryService::apply(
            &mut state,
            expected,
            HistoryCommand::Append { payload: payload() },
            &mut repository,
        )
        .expect("append");
    }
    let database = redb::Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut table = transaction
        .open_table(redb::TableDefinition::<&[u8], &[u8]>::new(
            "rusttable_history_blobs",
        ))
        .expect("blob table");
    let key = table
        .iter()
        .expect("blob iterator")
        .next()
        .expect("blob")
        .unwrap()
        .0
        .value()
        .to_vec();
    table.remove(key.as_slice()).expect("remove blob");
    drop(table);
    transaction.commit().expect("commit");
    drop(database);
    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert!(repository.load().is_err());
    support::remove(&path);
}
