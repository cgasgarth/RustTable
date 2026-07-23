use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use rusttable_catalog::{CanonicalPayload, RepositoryError};
use sha2::{Digest, Sha256};

mod duplicates;
#[path = "schema/history_migration.rs"]
mod history_migration;
#[path = "schema/metadata.rs"]
mod metadata_schema;
#[path = "schema/photo_groups.rs"]
mod photo_group_schema;
#[path = "schema/tags.rs"]
mod tag_schema;
#[path = "schema/validation.rs"]
mod validation;

pub(crate) use duplicates::*;
use history_migration::{blob_key, open_history_tables};
pub(crate) use metadata_schema::*;
pub(crate) use tag_schema::*;

pub const CURRENT_SCHEMA_VERSION: u8 = 14;

pub(crate) const SCHEMA_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_schema");
pub(crate) const RECORDS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_import_records");
pub(crate) const PHOTO_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_index");
pub(crate) const ASSET_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_asset_index");
pub(crate) const PHOTO_ORGANIZATION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_organization");
pub(crate) const ORGANIZATION_REVISION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_organization_revision");
pub(crate) const PHOTO_GROUPS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_groups");
pub(crate) const PHOTO_GROUP_MEMBER_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_group_member_index");
pub(crate) const EDITS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_edits");
pub(crate) const IMPORT_DETAILS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_import_details");
pub(crate) const REFERENCE_PATH_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_reference_path_index");
pub(crate) const SOURCE_RECONCILIATION_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_source_reconciliation");
pub(crate) const RECIPES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipes");
pub(crate) const RECIPE_HEADS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipe_heads");
pub(crate) const RECIPE_REFERENCES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipe_references");
pub(crate) const COLLECTION_STATE_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_collection_state");
pub(crate) const COLLECTIONS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_saved_collections");
pub(crate) const COLLECTION_NAME_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_collection_name_index");
pub(crate) const RECENT_QUERY_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_recent_queries");
pub(crate) const RECENT_ORDER_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_recent_order_index");
pub(crate) const ACTIVE_VIEW_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_active_library_view");
pub(crate) const COLLECTION_INTEGRITY_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_collection_integrity");
pub(crate) const HISTORY_STATE_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_state");
pub(crate) const HISTORY_REVISIONS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_revisions");
pub(crate) const HISTORY_BLOBS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blobs");
pub(crate) const HISTORY_BLOB_REFS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_history_blob_refs");
pub(crate) const VERSION_KEY: &[u8] = b"schema-version";
pub(crate) const ORGANIZATION_REVISION_KEY: &[u8] = b"organization-revision";
const DATABASE_OPEN_RETRIES: u8 = 8;
const DATABASE_OPEN_RETRY_DELAY: Duration = Duration::from_millis(2);

pub(crate) fn open(path: &Path) -> Result<Arc<Database>, RepositoryError> {
    let existed = path.exists();
    for attempt in 0..DATABASE_OPEN_RETRIES {
        let mut databases = database_registry()
            .lock()
            .map_err(|_| RepositoryError::Unavailable)?;
        if let Some(database) = databases.get(path).and_then(Weak::upgrade) {
            return Ok(database);
        }
        let database = match Database::create(path) {
            Ok(database) => Arc::new(database),
            Err(redb::DatabaseError::DatabaseAlreadyOpen)
                if attempt + 1 < DATABASE_OPEN_RETRIES =>
            {
                drop(databases);
                std::thread::sleep(DATABASE_OPEN_RETRY_DELAY);
                continue;
            }
            Err(error) => {
                return Err(match error {
                    redb::DatabaseError::DatabaseAlreadyOpen => RepositoryError::Unavailable,
                    _ if existed => RepositoryError::CorruptPersistedData,
                    _ => RepositoryError::Unavailable,
                });
            }
        };
        if existed {
            validate(&database)?;
        } else {
            initialize(&database)?;
        }
        databases.insert(PathBuf::from(path), Arc::downgrade(&database));
        return Ok(database);
    }
    Err(RepositoryError::Unavailable)
}

fn database_registry() -> &'static Mutex<BTreeMap<PathBuf, Weak<Database>>> {
    static DATABASES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<Database>>>> = OnceLock::new();
    DATABASES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn initialize(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(PHOTO_INDEX_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(ASSET_INDEX_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        photo_group_schema::open_organization_tables(&transaction)?;
        transaction
            .open_table(EDITS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(IMPORT_DETAILS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        open_duplicate_tables(&transaction)?;
        transaction
            .open_table(RECIPES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_HEADS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_REFERENCES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        open_collection_tables(&transaction)?;
        open_history_tables(&transaction)?;
        open_metadata_tables(&transaction)?;
        open_tag_tables(&transaction)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::Unavailable)
}

fn validate(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_read()
        .map_err(|_| RepositoryError::Unavailable)?;
    let schema = transaction
        .open_table(SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let version = schema
        .get(VERSION_KEY)
        .map_err(|_| RepositoryError::CorruptPersistedData)?
        .ok_or(RepositoryError::CorruptPersistedData)?;
    let version_value = version.value().to_vec();
    drop(version);
    match version_value.as_slice() {
        [CURRENT_SCHEMA_VERSION] => {
            drop(schema);
            drop(transaction);
            history_migration::repair_legacy_blob_refs(database)?;
            let transaction = database
                .begin_read()
                .map_err(|_| RepositoryError::Unavailable)?;
            validation::validate_tables(&transaction)
        }
        [13] => photo_group_schema::migrate_to_v14(database),
        [6] => {
            drop(schema);
            drop(transaction);
            migrate_to_v7(database).and_then(|()| migrate_to_v8(database))
        }
        [7] => {
            drop(schema);
            drop(transaction);
            migrate_to_v8(database)
        }
        [8] => {
            drop(schema);
            drop(transaction);
            migrate_to_v9(database).and_then(|()| migrate_to_v10(database))
        }
        [9] => {
            drop(schema);
            drop(transaction);
            migrate_to_v10(database)
        }
        [10] => {
            drop(schema);
            drop(transaction);
            migrate_metadata_and_tags_to_v12(database)
                .and_then(|()| migrate_duplicates_to_v13(database))
        }
        [11] => {
            drop(schema);
            drop(transaction);
            migrate_tags_to_v12(database).and_then(|()| migrate_duplicates_to_v13(database))
        }
        [12] => {
            drop(schema);
            drop(transaction);
            migrate_duplicates_to_v13(database)
        }
        [5] => {
            drop(schema);
            drop(transaction);
            migrate_to_v6(database)
        }
        [4] => {
            drop(schema);
            drop(transaction);
            migrate_to_v5(database)
        }
        [3] => {
            drop(schema);
            drop(transaction);
            migrate_to_v4(database)
        }
        [1 | 2] => {
            drop(schema);
            drop(transaction);
            migrate_legacy_to_v4(database)
        }
        _ => Err(RepositoryError::CorruptPersistedData),
    }
}

fn migrate_legacy_to_v4(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        let is_legacy = matches!(version.value(), [1 | 2]);
        drop(version);
        if !is_legacy {
            return Err(RepositoryError::CorruptPersistedData);
        }
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECORDS_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        transaction
            .open_table(PHOTO_INDEX_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        transaction
            .open_table(ASSET_INDEX_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        transaction
            .open_table(EDITS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(IMPORT_DETAILS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_HEADS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_REFERENCES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        open_collection_tables(&transaction)?;
        open_history_tables(&transaction)?;
        photo_group_schema::open_organization_tables(&transaction)?;
        open_metadata_tables(&transaction)?;
        open_tag_tables(&transaction)?;
        open_duplicate_tables(&transaction)?;
        backfill_duplicate_evidence(&transaction)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v4(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if version.value() != [3] {
            return Err(RepositoryError::CorruptPersistedData);
        }
        drop(version);
        transaction
            .open_table(RECIPES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_HEADS_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        transaction
            .open_table(RECIPE_REFERENCES_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        open_collection_tables(&transaction)?;
        open_history_tables(&transaction)?;
        photo_group_schema::open_organization_tables(&transaction)?;
        open_metadata_tables(&transaction)?;
        open_tag_tables(&transaction)?;
        open_duplicate_tables(&transaction)?;
        backfill_duplicate_evidence(&transaction)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v5(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_collection_tables(&transaction)?;
    open_history_tables(&transaction)?;
    photo_group_schema::open_organization_tables(&transaction)?;
    open_metadata_tables(&transaction)?;
    open_tag_tables(&transaction)?;
    open_duplicate_tables(&transaction)?;
    backfill_duplicate_evidence(&transaction)?;
    transaction
        .open_table(SOURCE_RECONCILIATION_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    let mut schema = transaction
        .open_table(SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    schema
        .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(schema);
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v6(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if version.value() != [5] {
            return Err(RepositoryError::CorruptPersistedData);
        }
        drop(version);
        open_history_tables(&transaction)?;
        photo_group_schema::open_organization_tables(&transaction)?;
        open_metadata_tables(&transaction)?;
        open_tag_tables(&transaction)?;
        open_duplicate_tables(&transaction)?;
        backfill_duplicate_evidence(&transaction)?;
        backfill_history_blobs(&transaction)?;
        backfill_history_blob_refs(&transaction)?;
        migrate_current_edits_to_history(&transaction)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn backfill_history_blobs(transaction: &redb::WriteTransaction) -> Result<(), RepositoryError> {
    let revisions = transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let records = revisions
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
        .map(|entry| {
            let (_, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
            let revision = crate::history_codec::decode_revision(value.value())
                .map_err(|()| RepositoryError::CorruptPersistedData)?;
            let payload = CanonicalPayload::from_history(revision.payload())
                .map_err(|_| RepositoryError::CorruptPersistedData)?;
            Ok::<_, RepositoryError>([
                (
                    blob_key(payload.edit().id()),
                    payload.edit().bytes().to_vec(),
                ),
                (
                    blob_key(payload.mask_blend().id()),
                    payload.mask_blend().bytes().to_vec(),
                ),
                (
                    blob_key(payload.pipeline().id()),
                    payload.pipeline().bytes().to_vec(),
                ),
            ])
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(revisions);
    let mut blobs = transaction
        .open_table(HISTORY_BLOBS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    for group in records {
        for (key, bytes) in group {
            if let Some(existing) = blobs
                .get(key.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?
            {
                if existing.value() != bytes.as_slice() {
                    return Err(RepositoryError::CorruptPersistedData);
                }
            } else {
                blobs
                    .insert(key.as_slice(), bytes.as_slice())
                    .map_err(|_| RepositoryError::Unavailable)?;
            }
        }
    }
    Ok(())
}

fn migrate_to_v7(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if version.value() != [6] {
            return Err(RepositoryError::CorruptPersistedData);
        }
        drop(version);
        open_history_tables(&transaction)?;
        backfill_history_blobs(&transaction)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[7][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v8(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if version.value() != [7] {
            return Err(RepositoryError::CorruptPersistedData);
        }
        drop(version);
        open_history_tables(&transaction)?;
        photo_group_schema::open_organization_tables(&transaction)?;
        open_metadata_tables(&transaction)?;
        open_tag_tables(&transaction)?;
        open_duplicate_tables(&transaction)?;
        backfill_duplicate_evidence(&transaction)?;
        backfill_history_blob_refs(&transaction)?;
        migrate_current_edits_to_history(&transaction)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v9(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    {
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        let version = schema
            .get(VERSION_KEY)
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .ok_or(RepositoryError::CorruptPersistedData)?;
        if version.value() != [8] {
            return Err(RepositoryError::CorruptPersistedData);
        }
        drop(version);
        photo_group_schema::open_organization_tables(&transaction)?;
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        schema
            .insert(VERSION_KEY, &[9][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn migrate_to_v10(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_metadata_tables(&transaction)?;
    open_tag_tables(&transaction)?;
    open_duplicate_tables(&transaction)?;
    let existing = {
        let paths = transaction
            .open_table(REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        paths
            .iter()
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .map(|entry| {
                let (key, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
                Ok::<_, RepositoryError>((key.value().to_vec(), value.value().to_vec()))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    if existing.is_empty() {
        transaction
            .open_table(SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| RepositoryError::Unavailable)?;
        let mut schema = transaction
            .open_table(SCHEMA_TABLE)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
        drop(schema);
        return transaction
            .commit()
            .map_err(|_| RepositoryError::CommitFailure);
    }
    let mut canonical = transaction
        .open_table(REFERENCE_PATH_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    let mut reconciliation = transaction
        .open_table(SOURCE_RECONCILIATION_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    let mut owners = BTreeMap::<[u8; 32], Vec<u8>>::new();
    let mut sequence = 0_u64;
    for (old_identity, source) in existing {
        let Ok(path) = decode_reference_path(&source) else {
            let key = format!("invalid/{sequence}").into_bytes();
            reconciliation
                .insert(key.as_slice(), source.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?;
            sequence = sequence.saturating_add(1);
            continue;
        };
        let Some(identity) = canonical_path_identity(&path) else {
            let key = format!("invalid/{sequence}").into_bytes();
            reconciliation
                .insert(key.as_slice(), source.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?;
            sequence = sequence.saturating_add(1);
            continue;
        };
        if let Some(previous_source) = owners.get(&identity) {
            if previous_source != &source {
                let mut key = b"ambiguous/".to_vec();
                key.extend_from_slice(&identity);
                key.extend_from_slice(&sequence.to_be_bytes());
                reconciliation
                    .insert(key.as_slice(), source.as_slice())
                    .map_err(|_| RepositoryError::Unavailable)?;
                sequence = sequence.saturating_add(1);
                continue;
            }
        } else {
            owners.insert(identity, source.clone());
            canonical
                .insert(identity.as_slice(), source.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?;
        }
        if old_identity.as_slice() != identity.as_slice() {
            let mut key = b"migrated/".to_vec();
            key.extend_from_slice(&identity);
            reconciliation
                .insert(key.as_slice(), old_identity.as_slice())
                .map_err(|_| RepositoryError::Unavailable)?;
        }
    }
    drop(reconciliation);
    drop(canonical);
    backfill_duplicate_evidence(&transaction)?;
    let mut schema = transaction
        .open_table(SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    schema
        .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(schema);
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}

fn decode_reference_path(source: &[u8]) -> Result<PathBuf, ()> {
    let source = std::str::from_utf8(source).map_err(|_| ())?;
    let mut components = source.split('/');
    if components.next() != Some("reference-v1")
        || components.next().is_none()
        || components.next().is_none()
        || components.next().is_some()
    {
        return Err(());
    }
    let encoded = source.split('/').nth(2).ok_or(())?;
    let bytes = decode_hex(encoded)?;
    let path = String::from_utf8(bytes).map_err(|_| ())?;
    Ok(PathBuf::from(path))
}

fn canonical_path_identity(path: &Path) -> Option<[u8; 32]> {
    let mut normalized = PathBuf::new();
    let mut components = 0_usize;
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new(std::path::MAIN_SEPARATOR_STR)),
            Component::CurDir => {}
            Component::Normal(value) => {
                normalized.push(value);
                components = components.saturating_add(1);
            }
            Component::ParentDir => {
                if components == 0 {
                    if path.is_absolute() {
                        continue;
                    }
                    return None;
                }
                normalized.pop();
                components = components.saturating_sub(1);
            }
        }
    }
    let value = normalized.to_str()?;
    if value.is_empty() || (path.is_relative() && components == 0) {
        return None;
    }
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable-reference-path-v1\0");
    hasher.update(value.as_bytes());
    Some(hasher.finalize().into())
}

fn decode_hex(encoded: &str) -> Result<Vec<u8>, ()> {
    if !encoded.len().is_multiple_of(2) {
        return Err(());
    }
    encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = match pair[0] {
                b'0'..=b'9' => pair[0] - b'0',
                b'a'..=b'f' => pair[0] - b'a' + 10,
                _ => return Err(()),
            };
            let low = match pair[1] {
                b'0'..=b'9' => pair[1] - b'0',
                b'a'..=b'f' => pair[1] - b'a' + 10,
                _ => return Err(()),
            };
            Ok((high << 4) | low)
        })
        .collect()
}

fn backfill_history_blob_refs(transaction: &redb::WriteTransaction) -> Result<(), RepositoryError> {
    let revisions = transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let keys = revisions
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
        .map(|entry| {
            let (_, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
            let revision = crate::history_codec::decode_revision(value.value())
                .map_err(|()| RepositoryError::CorruptPersistedData)?;
            let payload = CanonicalPayload::from_history(revision.payload())
                .map_err(|_| RepositoryError::CorruptPersistedData)?;
            Ok::<_, RepositoryError>([
                blob_key(payload.edit().id()),
                blob_key(payload.mask_blend().id()),
                blob_key(payload.pipeline().id()),
            ])
        })
        .collect::<Result<Vec<_>, _>>()?;
    drop(revisions);
    let mut counts = std::collections::BTreeMap::<[u8; 43], u64>::new();
    for group in keys {
        for key in group {
            let count = counts.entry(key).or_default();
            *count = count
                .checked_add(1)
                .ok_or(RepositoryError::CorruptPersistedData)?;
        }
    }
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    for (key, count) in counts {
        refs.insert(key.as_slice(), count.to_be_bytes().as_slice())
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    Ok(())
}

fn migrate_current_edits_to_history(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    use rusttable_catalog::{
        HistoryCommand, HistoryOperationKind, HistoryOperationSummary, HistoryPayload, HistoryState,
    };
    use rusttable_core::PhotoId;

    let edits = transaction
        .open_table(EDITS_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut current = std::collections::BTreeMap::<PhotoId, rusttable_core::Edit>::new();
    for entry in edits
        .iter()
        .map_err(|_| RepositoryError::CorruptPersistedData)?
    {
        let (_, value) = entry.map_err(|_| RepositoryError::CorruptPersistedData)?;
        let edit = crate::edit_codec::decode(value.value())
            .map_err(|()| RepositoryError::CorruptPersistedData)?;
        if current.get(&edit.photo_id()).is_none_or(|existing| {
            (edit.revision(), edit.id()) > (existing.revision(), existing.id())
        }) {
            current.insert(edit.photo_id(), edit);
        }
    }
    drop(edits);
    let states = transaction
        .open_table(HISTORY_STATE_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut missing = Vec::new();
    for (photo_id, edit) in current {
        if states
            .get(photo_id.get().to_be_bytes().as_slice())
            .map_err(|_| RepositoryError::CorruptPersistedData)?
            .is_none()
        {
            missing.push((photo_id, edit));
        }
    }
    drop(states);
    for (photo_id, edit) in missing {
        let mut state = HistoryState::new(photo_id);
        let summary = HistoryOperationSummary::new(
            HistoryOperationKind::Parameter,
            None,
            None,
            "migrated current edit",
        )
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
        state
            .apply(
                state.version(),
                HistoryCommand::Append {
                    payload: HistoryPayload::new(edit, Vec::new(), Vec::new(), summary),
                },
            )
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
        stage_migrated_history(transaction, &state)?;
    }
    Ok(())
}

fn stage_migrated_history(
    transaction: &redb::WriteTransaction,
    state: &rusttable_catalog::HistoryState,
) -> Result<(), RepositoryError> {
    let snapshot = state.persistence_snapshot();
    let metadata = crate::history_codec::encode_meta(&snapshot)
        .map_err(|()| RepositoryError::CorruptPersistedData)?;
    let revision = snapshot
        .revisions()
        .first()
        .ok_or(RepositoryError::CorruptPersistedData)?;
    let encoded = crate::history_codec::encode_revision(revision)
        .map_err(|()| RepositoryError::CorruptPersistedData)?;
    let payload = CanonicalPayload::from_history(revision.payload())
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let mut states = transaction
        .open_table(HISTORY_STATE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    states
        .insert(
            state.photo_id().get().to_be_bytes().as_slice(),
            metadata.as_slice(),
        )
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(states);
    let mut revisions = transaction
        .open_table(HISTORY_REVISIONS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    revisions
        .insert(
            revision_key(state.photo_id(), revision.id()).as_slice(),
            encoded.as_slice(),
        )
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(revisions);
    let mut blobs = transaction
        .open_table(HISTORY_BLOBS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    let mut refs = transaction
        .open_table(HISTORY_BLOB_REFS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    for blob in [payload.edit(), payload.mask_blend(), payload.pipeline()] {
        blobs
            .insert(blob_key(blob.id()).as_slice(), blob.bytes())
            .map_err(|_| RepositoryError::Unavailable)?;
        refs.insert(
            blob_key(blob.id()).as_slice(),
            1_u64.to_be_bytes().as_slice(),
        )
        .map_err(|_| RepositoryError::Unavailable)?;
    }
    Ok(())
}

fn revision_key(
    photo_id: rusttable_core::PhotoId,
    revision: rusttable_catalog::HistoryRevisionId,
) -> [u8; 24] {
    let mut key = [0; 24];
    key[..16].copy_from_slice(&photo_id.get().to_be_bytes());
    key[16..].copy_from_slice(&revision.get().to_be_bytes());
    key
}

fn open_collection_tables(transaction: &redb::WriteTransaction) -> Result<(), RepositoryError> {
    transaction
        .open_table(COLLECTION_STATE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(COLLECTIONS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(COLLECTION_NAME_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(RECENT_QUERY_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(RECENT_ORDER_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(ACTIVE_VIEW_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(COLLECTION_INTEGRITY_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    use super::open;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn concurrent_database_open_reuses_the_live_catalog_handle() {
        let path = std::env::temp_dir().join(format!(
            "rusttable-schema-open-{}-{}.redb",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let first = open(&path).expect("first schema open");
        let second = open(&path).expect("second schema open");
        assert!(Arc::ptr_eq(&first, &second));
        drop(first);
        drop(second);
        let _ = fs::remove_file(path);
    }
}
