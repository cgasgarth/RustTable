use redb::{Database, TableDefinition};
use rusttable_catalog::RepositoryError;

use super::{CURRENT_SCHEMA_VERSION, SCHEMA_TABLE, VERSION_KEY};

pub(crate) const TAG_STATE_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_state");
pub(crate) const TAG_PATH_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_path_index");
pub(crate) const TAG_ALIAS_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_alias_index");
pub(crate) const TAG_PHOTO_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_tag_photo_index");
pub(crate) const PHOTO_TAG_INDEX_TABLE: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_photo_tag_index");

pub(crate) fn migrate_tags_to_v12(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_tag_tables(&transaction)?;
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

pub(crate) fn open_tag_tables(transaction: &redb::WriteTransaction) -> Result<(), RepositoryError> {
    transaction
        .open_table(TAG_STATE_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(TAG_PATH_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(TAG_ALIAS_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(TAG_PHOTO_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(PHOTO_TAG_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}
