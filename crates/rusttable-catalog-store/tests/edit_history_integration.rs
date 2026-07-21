mod support;

use redb::{Database, ReadableTable, TableDefinition};
use rusttable_catalog::{
    EditRepository, HistoryPageDirection, HistoryPageRequest, HistoryRepository,
};
use rusttable_catalog_store::{RedbEditRepository, RedbHistoryRepository};
use rusttable_core::{
    Edit, EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};

const SCHEMA: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_schema");
const HISTORY_STATE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_state");
const HISTORY_REVISIONS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_revisions");
const HISTORY_BLOBS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blobs");
const HISTORY_REFS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blob_refs");
const VERSION_KEY: &[u8] = b"schema-version";

fn edit(id: u128, revision: u64, value: i64) -> Edit {
    Edit::from_parts(
        EditId::new(id).unwrap(),
        PhotoId::new(7).unwrap(),
        Revision::ZERO,
        Revision::from_u64(revision),
        [Operation::new_with_opacity(
            OperationId::new(id + 100).unwrap(),
            OperationKey::new("rusttable.exposure").unwrap(),
            true,
            OperationOpacity::ONE,
            [(
                ParameterName::new("stops").unwrap(),
                ParameterValue::Integer(value),
            )],
        )
        .unwrap()],
    )
    .unwrap()
}

#[test]
fn current_edit_create_and_replace_append_one_revision_each() {
    let path = support::temp_path("current-edit-history");
    let original = edit(1, 0, 1);
    let replacement = edit(1, 1, 2);
    let mut edits = RedbEditRepository::open(&path).unwrap();
    let receipt = edits.commit_new_with_receipt(&original).unwrap();
    assert_eq!(receipt.revision().get(), 1);
    assert_eq!(receipt.revision_count(), 1);
    let receipt = edits
        .commit_replacement_with_receipt(Revision::ZERO, &replacement)
        .unwrap();
    assert_eq!(receipt.revision().get(), 2);
    assert_eq!(receipt.parent().unwrap().get(), 1);
    drop(edits);

    let history = RedbHistoryRepository::open(&path, PhotoId::new(7).unwrap()).unwrap();
    let state = history.load().unwrap().unwrap();
    assert_eq!(state.revisions().count(), 2);
    assert_eq!(history.blob_count().unwrap(), 4);
    let page = history
        .page(HistoryPageRequest::new(
            None,
            1,
            HistoryPageDirection::Ascending,
        ))
        .unwrap()
        .unwrap();
    assert!(page.has_more());
    let second = history
        .reconstruct(page.next_cursor().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(second.payload().edit(), &original);
    support::remove(&path);
}

#[test]
fn current_edit_migration_preserves_edit_and_creates_revision_one() {
    let path = support::temp_path("current-edit-migration");
    let original = edit(2, 4, 9);
    let mut edits = RedbEditRepository::open(&path).unwrap();
    edits.commit_new(&original).unwrap();
    drop(edits);

    let database = Database::open(&path).unwrap();
    let transaction = database.begin_write().unwrap();
    {
        let mut schema = transaction.open_table(SCHEMA).unwrap();
        schema.insert(VERSION_KEY, &[7][..]).unwrap();
        for table in [
            HISTORY_STATE,
            HISTORY_REVISIONS,
            HISTORY_BLOBS,
            HISTORY_REFS,
        ] {
            let mut table = transaction.open_table(table).unwrap();
            let keys = table
                .iter()
                .unwrap()
                .map(|entry| entry.unwrap().0.value().to_vec())
                .collect::<Vec<_>>();
            for key in keys {
                table.remove(key.as_slice()).unwrap();
            }
        }
    }
    transaction.commit().unwrap();
    drop(database);

    let edits = RedbEditRepository::open(&path).unwrap();
    assert_eq!(
        edits.find_by_edit_id(original.id()).unwrap(),
        Some(original)
    );
    drop(edits);
    let history = RedbHistoryRepository::open(&path, PhotoId::new(7).unwrap()).unwrap();
    let state = history.load().unwrap().unwrap();
    assert_eq!(state.revisions().count(), 1);
    assert_eq!(state.current_revision().unwrap().id().get(), 1);
    support::remove(&path);
}

#[test]
fn concurrent_replacements_have_one_winner_and_no_partial_revision() {
    let path = support::temp_path("current-edit-concurrency");
    let original = edit(3, 0, 1);
    let first = edit(3, 1, 2);
    let second = edit(3, 1, 3);
    let mut seed = RedbEditRepository::open(&path).unwrap();
    seed.commit_new(&original).unwrap();
    drop(seed);
    let repository = std::sync::Arc::new(std::sync::Mutex::new(
        RedbEditRepository::open(&path).unwrap(),
    ));
    let left_repository = std::sync::Arc::clone(&repository);
    let right_repository = std::sync::Arc::clone(&repository);
    let left = std::thread::spawn(move || {
        left_repository
            .lock()
            .unwrap()
            .commit_replacement(Revision::ZERO, &first)
    });
    let right = std::thread::spawn(move || {
        right_repository
            .lock()
            .unwrap()
            .commit_replacement(Revision::ZERO, &second)
    });
    let outcomes = [left.join().unwrap(), right.join().unwrap()];
    assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
    drop(repository);
    let history = RedbHistoryRepository::open(&path, PhotoId::new(7).unwrap()).unwrap();
    let report = history.verify_invariants().unwrap().unwrap();
    assert_eq!(report.revisions, 2);
    assert_eq!(report.journal_entries, 2);
    support::remove(&path);
}
