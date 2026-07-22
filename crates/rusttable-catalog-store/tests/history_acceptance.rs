mod support;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use rusttable_catalog::{
    DurableHistoryService, HistoryCommand, HistoryOperationKind, HistoryOperationSummary,
    HistoryPayload, HistoryRepository, HistoryState,
};
use rusttable_catalog_store::RedbHistoryRepository;
use rusttable_core::{Edit, EditId, PhotoId, Revision};

const HISTORY_BLOBS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blobs");
const HISTORY_BLOB_REFS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blob_refs");

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

fn shared_payload(photo_id: PhotoId, edit_id: u128) -> HistoryPayload {
    HistoryPayload::new(
        Edit::new(
            EditId::new(edit_id).expect("edit ID"),
            photo_id,
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
fn restart_migrates_legacy_per_photo_refs_and_preserves_both_histories() {
    let path = support::temp_path("history-legacy-ref-migration");
    let first_photo = PhotoId::new(7).expect("first photo ID");
    let second_photo = PhotoId::new(8).expect("second photo ID");
    let mut exports = Vec::new();
    for (photo_id, edit_id) in [(first_photo, 7), (second_photo, 8)] {
        let mut state = HistoryState::new(photo_id);
        let mut repository = RedbHistoryRepository::open(&path, photo_id).expect("repository");
        let expected = state.version();
        DurableHistoryService::apply(
            &mut state,
            expected,
            HistoryCommand::Append {
                payload: shared_payload(photo_id, edit_id),
            },
            &mut repository,
        )
        .expect("append");
        exports.push(repository.export_canonical().expect("export"));
    }

    let database = Database::open(&path).expect("database");
    let transaction = database.begin_write().expect("write transaction");
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS)
        .expect("refs table");
    let legacy_keys = refs
        .iter()
        .expect("refs iterator")
        .map(|entry| {
            let (key, value) = entry.expect("ref entry");
            (key.value().to_vec(), value.value().to_vec())
        })
        .filter(|(_, value)| value == 2_u64.to_be_bytes().as_slice())
        .map(|(key, _)| key)
        .collect::<Vec<_>>();
    assert_eq!(
        legacy_keys.len(),
        2,
        "fixture must contain two shared blobs"
    );
    for key in legacy_keys {
        refs.insert(key.as_slice(), 1_u64.to_be_bytes().as_slice())
            .expect("write legacy per-photo count");
    }
    drop(refs);
    transaction.commit().expect("commit legacy fixture");
    drop(database);

    for (index, (photo_id, edit_id)) in [(first_photo, 7), (second_photo, 8)]
        .into_iter()
        .enumerate()
    {
        let repository = RedbHistoryRepository::open(&path, photo_id).expect("migrated reopen");
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
        assert_eq!(
            repository.export_canonical().expect("export"),
            exports[index]
        );
        assert_eq!(repository.blob_count().expect("blob count"), 4);
    }

    let database = Database::open(&path).expect("database after migration");
    let transaction = database.begin_read().expect("read transaction");
    let refs = transaction
        .open_table(HISTORY_BLOB_REFS)
        .expect("refs table");
    let counts = refs
        .iter()
        .expect("refs iterator")
        .map(|entry| {
            let (_, value) = entry.expect("ref entry");
            u64::from_be_bytes(value.value().try_into().expect("ref count"))
        })
        .collect::<Vec<_>>();
    assert_eq!(counts.len(), 4);
    assert_eq!(counts.iter().filter(|count| **count == 2).count(), 2);
    assert_eq!(counts.iter().filter(|count| **count == 1).count(), 2);
    drop(refs);
    drop(transaction);
    drop(database);
    support::remove(&path);
}

#[test]
fn restart_rejects_a_mismatched_unique_blob_ref_count() {
    let path = support::temp_path("history-mismatched-ref-count");
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
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS)
        .expect("refs table");
    let key = refs
        .iter()
        .expect("refs iterator")
        .next()
        .expect("ref")
        .expect("ref entry")
        .0
        .value()
        .to_vec();
    refs.insert(key.as_slice(), 2_u64.to_be_bytes().as_slice())
        .expect("write mismatched count");
    drop(refs);
    transaction.commit().expect("commit mismatched ref");
    drop(database);

    assert!(RedbHistoryRepository::open(&path, photo).is_err());
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
