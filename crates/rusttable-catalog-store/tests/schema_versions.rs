mod support;

use redb::{Database, ReadableDatabase, TableDefinition};
use rusttable_catalog_store::{CURRENT_SCHEMA_VERSION, RedbImportRepository};
use std::path::Path;

const SCHEMA: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_schema");
const RECORDS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_import_records");
const PHOTOS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_photo_index");
const ASSETS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_asset_index");
const VERSION_KEY: &[u8] = b"schema-version";

fn write_version(path: &Path, version: &[u8]) {
    let database = Database::create(path).unwrap();
    let transaction = database.begin_write().unwrap();
    {
        let mut table = transaction.open_table(SCHEMA).unwrap();
        table.insert(VERSION_KEY, version).unwrap();
        transaction.open_table(RECORDS).unwrap();
        transaction.open_table(PHOTOS).unwrap();
        transaction.open_table(ASSETS).unwrap();
    }
    transaction.commit().unwrap();
}

#[test]
fn new_store_writes_current_schema_version() {
    let path = support::temp_path("schema-current");
    drop(RedbImportRepository::open(&path).unwrap());
    let database = Database::open(&path).unwrap();
    let transaction = database.begin_read().unwrap();
    let table = transaction.open_table(SCHEMA).unwrap();
    assert_eq!(
        table.get(VERSION_KEY).unwrap().unwrap().value(),
        &[CURRENT_SCHEMA_VERSION]
    );
    support::remove(&path);
}

#[test]
fn legacy_schema_migrates_without_rewriting_import_tables() {
    let old = support::temp_path("schema-old");
    write_version(&old, &[1]);
    drop(RedbImportRepository::open(&old).unwrap());
    let database = Database::open(&old).unwrap();
    let transaction = database.begin_read().unwrap();
    let table = transaction.open_table(SCHEMA).unwrap();
    assert_eq!(
        table.get(VERSION_KEY).unwrap().unwrap().value(),
        &[CURRENT_SCHEMA_VERSION]
    );
    support::remove(&old);
}

#[test]
fn unsupported_schema_versions_fail_closed() {
    let old = support::temp_path("schema-too-old");
    write_version(&old, &[0]);
    assert!(RedbImportRepository::open(&old).is_err());
    support::remove(&old);
    let newer = support::temp_path("schema-newer");
    write_version(&newer, &[CURRENT_SCHEMA_VERSION + 1]);
    assert!(RedbImportRepository::open(&newer).is_err());
    support::remove(&newer);
}
