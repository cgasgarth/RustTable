use std::path::Path;

use redb::{Database, ReadableDatabase, TableDefinition};

use rusttable_catalog::RepositoryError;

pub const CURRENT_SCHEMA_VERSION: u8 = 1;

pub(crate) const SCHEMA_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_schema");
pub(crate) const RECORDS_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_import_records");
pub(crate) const PHOTO_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_index");
pub(crate) const ASSET_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_asset_index");
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
    if version.value() != [CURRENT_SCHEMA_VERSION] {
        return Err(RepositoryError::CorruptPersistedData);
    }
    for table in [RECORDS_TABLE, PHOTO_INDEX_TABLE, ASSET_INDEX_TABLE] {
        transaction
            .open_table(table)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
    }
    Ok(())
}
