mod support;

use redb::{Database, ReadableDatabase, TableDefinition};
use rusttable_catalog::ImportRepository as _;
use rusttable_catalog_store::{
    CURRENT_SCHEMA_VERSION, RedbCatalogMetadataRepository, RedbCatalogRepository,
    RedbImportRepository,
};
use std::path::Path;

const SCHEMA: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_schema");
const RECORDS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_import_records");
const PHOTOS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_photo_index");
const ASSETS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_asset_index");
const EDITS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_edits");
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
fn schema_v2_migrates_and_legacy_photos_have_no_fabricated_details() {
    let path = support::temp_path("schema-v2");
    let database = Database::create(&path).unwrap();
    let transaction = database.begin_write().unwrap();
    {
        let mut schema = transaction.open_table(SCHEMA).unwrap();
        schema.insert(VERSION_KEY, &[2][..]).unwrap();
        transaction.open_table(RECORDS).unwrap();
        transaction.open_table(PHOTOS).unwrap();
        transaction.open_table(ASSETS).unwrap();
        transaction.open_table(EDITS).unwrap();
    }
    transaction.commit().unwrap();
    drop(database);

    let repository = RedbCatalogRepository::open(&path).unwrap();
    assert_eq!(
        repository
            .find_import_details_by_photo_id(rusttable_core::PhotoId::new(1).unwrap())
            .unwrap(),
        None
    );
    drop(repository);
    let database = Database::open(&path).unwrap();
    let transaction = database.begin_read().unwrap();
    let schema = transaction.open_table(SCHEMA).unwrap();
    assert_eq!(
        schema.get(VERSION_KEY).unwrap().unwrap().value(),
        &[CURRENT_SCHEMA_VERSION]
    );
    support::remove(&path);
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

#[test]
fn schema_v10_adds_catalog_metadata_tables_without_import_rewrites() {
    let path = support::temp_path("schema-v10-metadata");
    let mut imports = RedbImportRepository::open(&path).unwrap();
    let record = support::record("preserved.raw", 41, 42, 3);
    imports.commit(&record).unwrap();
    drop(imports);
    let database = Database::open(&path).unwrap();
    let transaction = database.begin_write().unwrap();
    {
        let mut schema = transaction.open_table(SCHEMA).unwrap();
        schema.insert(VERSION_KEY, &[10][..]).unwrap();
    }
    transaction.commit().unwrap();
    drop(database);
    drop(RedbCatalogMetadataRepository::open(&path).unwrap());
    let imports = RedbImportRepository::open(&path).unwrap();
    assert_eq!(
        imports.find_by_photo_id(record.photo().id()).unwrap(),
        Some(record)
    );
    support::remove(&path);
}
