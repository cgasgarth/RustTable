use std::path::Path;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use rusttable_catalog::RepositoryError;

pub const CURRENT_SCHEMA_VERSION: u8 = 4;

pub(crate) const SCHEMA_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_schema");
pub(crate) const RECORDS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_import_records");
pub(crate) const PHOTO_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_index");
pub(crate) const ASSET_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_asset_index");
pub(crate) const EDITS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_edits");
pub(crate) const IMPORT_DETAILS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_import_details");
pub(crate) const REFERENCE_PATH_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_reference_path_index");
pub(crate) const RECIPES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipes");
pub(crate) const RECIPE_HEADS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipe_heads");
pub(crate) const RECIPE_REFERENCES_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_recipe_references");
pub(crate) const VERSION_KEY: &[u8] = b"schema-version";

pub(crate) fn open(path: &Path) -> Result<Database, RepositoryError> {
    let existed = path.exists();
    let database = Database::create(path).map_err(|_| {
        if existed {
            RepositoryError::CorruptPersistedData
        } else {
            RepositoryError::Unavailable
        }
    })?;
    if existed {
        validate(&database)?;
    } else {
        initialize(&database)?;
    }
    Ok(database)
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
    match version.value() {
        [CURRENT_SCHEMA_VERSION] => validate_tables(&transaction),
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

fn validate_tables(transaction: &redb::ReadTransaction) -> Result<(), RepositoryError> {
    for table in [
        RECORDS_TABLE,
        PHOTO_INDEX_TABLE,
        ASSET_INDEX_TABLE,
        EDITS_TABLE,
        IMPORT_DETAILS_TABLE,
        REFERENCE_PATH_INDEX_TABLE,
        RECIPES_TABLE,
        RECIPE_HEADS_TABLE,
        RECIPE_REFERENCES_TABLE,
    ] {
        transaction
            .open_table(table)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
    }
    Ok(())
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
        schema
            .insert(VERSION_KEY, &[CURRENT_SCHEMA_VERSION][..])
            .map_err(|_| RepositoryError::Unavailable)?;
    }
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}
