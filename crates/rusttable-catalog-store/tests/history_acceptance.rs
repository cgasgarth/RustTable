mod support;

use redb::{Database, ReadableTable, TableDefinition};
use rusttable_catalog::{
    DurableHistoryService, HistoryCommand, HistoryOperationKind, HistoryOperationSummary,
    HistoryPayload, HistoryRepository, HistoryState,
};
use rusttable_catalog_store::RedbHistoryRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};

const HISTORY_BLOBS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blobs");

fn payload(photo_id: PhotoId, edit_id: u128) -> HistoryPayload {
    HistoryPayload::new(
        Edit::new(
            EditId::new(edit_id).expect("edit ID"),
            photo_id,
            Revision::ZERO,
            [],
        )
        .expect("edit"),
        [u8::try_from(edit_id & 0xff).expect("mask byte"), 8, 7],
        [u8::try_from(edit_id & 0xff).expect("pipeline byte"), 5, 4],
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
        HistoryCommand::Append {
            payload: payload(photo, 1),
        },
        &mut repository,
    )
    .expect("first append");
    let expected = state.version();
    DurableHistoryService::apply(
        &mut state,
        expected,
        HistoryCommand::Append {
            payload: payload(photo, 1),
        },
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
fn multi_photo_catalog_fixture_loads_each_history_and_shares_global_refs() {
    let path = support::temp_path("history-multi-photo");
    let first_photo = PhotoId::new(7).expect("first photo ID");
    let second_photo = PhotoId::new(8).expect("second photo ID");
    for (photo_id, edit_id) in [(first_photo, 7), (second_photo, 8)] {
        let mut state = HistoryState::new(photo_id);
        let mut repository = RedbHistoryRepository::open(&path, photo_id).expect("repository");
        let expected = state.version();
        DurableHistoryService::apply(
            &mut state,
            expected,
            HistoryCommand::Append {
                payload: payload(photo_id, edit_id),
            },
            &mut repository,
        )
        .expect("append");
    }

    for (photo_id, edit_id) in [(first_photo, 7), (second_photo, 8)] {
        let repository = RedbHistoryRepository::open(&path, photo_id).expect("reopen");
        let state = repository.load().expect("load").expect("history");
        assert_eq!(state.photo_id(), photo_id);
        assert_eq!(
            state
                .current_revision()
                .expect("current")
                .payload()
                .edit()
                .id()
                .get(),
            edit_id
        );
        assert_eq!(repository.blob_count().expect("blob count"), 6);
    }
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
            HistoryCommand::Append {
                payload: payload(photo, 1),
            },
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

#[test]
fn restart_rejects_mismatched_content_blob() {
    let path = support::temp_path("history-mismatched-blob");
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
    let expected = state.version();
    DurableHistoryService::apply(
        &mut state,
        expected,
        HistoryCommand::Append {
            payload: payload(photo, 1),
        },
        &mut repository,
    )
    .expect("append");
    drop(repository);

    let database = Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut table = transaction.open_table(HISTORY_BLOBS).expect("blob table");
    let (key, mut bytes) = {
        let (key, value) = table
            .iter()
            .expect("blob iterator")
            .next()
            .expect("blob")
            .unwrap();
        (key.value().to_vec(), value.value().to_vec())
    };
    bytes[0] ^= 0xff;
    table
        .insert(key.as_slice(), bytes.as_slice())
        .expect("replace blob");
    drop(table);
    transaction.commit().expect("commit");
    drop(database);

    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert!(repository.load().is_err());
    support::remove(&path);
}

#[test]
fn restart_rejects_orphaned_content_blob() {
    let path = support::temp_path("history-orphaned-blob");
    let photo = PhotoId::new(7).expect("photo ID");
    let mut state = HistoryState::new(photo);
    let mut repository = RedbHistoryRepository::open(&path, photo).expect("repository");
    let expected = state.version();
    DurableHistoryService::apply(
        &mut state,
        expected,
        HistoryCommand::Append {
            payload: payload(photo, 1),
        },
        &mut repository,
    )
    .expect("append");
    drop(repository);

    let database = Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut table = transaction.open_table(HISTORY_BLOBS).expect("blob table");
    let mut orphan_key = {
        let (key, _) = table
            .iter()
            .expect("blob iterator")
            .next()
            .expect("blob")
            .unwrap();
        key.value().to_vec()
    };
    let last = orphan_key.last_mut().expect("blob key");
    *last ^= 0xff;
    table
        .insert(orphan_key.as_slice(), &[0][..])
        .expect("insert orphan");
    drop(table);
    transaction.commit().expect("commit");
    drop(database);

    let repository = RedbHistoryRepository::open(&path, photo).expect("reopen");
    assert!(repository.load().is_err());
    support::remove(&path);
}
