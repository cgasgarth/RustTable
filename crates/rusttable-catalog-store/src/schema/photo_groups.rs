use redb::{Database, ReadableTable};
use rusttable_catalog::RepositoryError;

use super::{
    ORGANIZATION_REVISION_TABLE, PHOTO_GROUP_MEMBER_INDEX_TABLE, PHOTO_GROUPS_TABLE,
    PHOTO_ORGANIZATION_TABLE, SCHEMA_TABLE, VERSION_KEY,
};

pub(super) fn open_organization_tables(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    transaction
        .open_table(PHOTO_ORGANIZATION_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(ORGANIZATION_REVISION_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    open_photo_group_tables(transaction)
}

pub(super) fn open_photo_group_tables(
    transaction: &redb::WriteTransaction,
) -> Result<(), RepositoryError> {
    transaction
        .open_table(PHOTO_GROUPS_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    transaction
        .open_table(PHOTO_GROUP_MEMBER_INDEX_TABLE)
        .map_err(|_| RepositoryError::Unavailable)?;
    Ok(())
}

pub(super) fn migrate_to_v14(database: &Database) -> Result<(), RepositoryError> {
    let transaction = database
        .begin_write()
        .map_err(|_| RepositoryError::Unavailable)?;
    open_photo_group_tables(&transaction)?;
    let mut schema = transaction
        .open_table(SCHEMA_TABLE)
        .map_err(|_| RepositoryError::CorruptPersistedData)?;
    let version = schema
        .get(VERSION_KEY)
        .map_err(|_| RepositoryError::CorruptPersistedData)?
        .ok_or(RepositoryError::CorruptPersistedData)?;
    if version.value() != [13] {
        return Err(RepositoryError::CorruptPersistedData);
    }
    drop(version);
    schema
        .insert(VERSION_KEY, &[14][..])
        .map_err(|_| RepositoryError::Unavailable)?;
    drop(schema);
    transaction
        .commit()
        .map_err(|_| RepositoryError::CommitFailure)
}
