use rusttable_catalog::RepositoryError;

pub(super) fn validate_tables(transaction: &redb::ReadTransaction) -> Result<(), RepositoryError> {
    for table in [
        super::RECORDS_TABLE,
        super::PHOTO_INDEX_TABLE,
        super::ASSET_INDEX_TABLE,
        super::PHOTO_ORGANIZATION_TABLE,
        super::ORGANIZATION_REVISION_TABLE,
        super::EDITS_TABLE,
        super::IMPORT_DETAILS_TABLE,
        super::REFERENCE_PATH_INDEX_TABLE,
        super::SOURCE_RECONCILIATION_TABLE,
        super::RECIPES_TABLE,
        super::RECIPE_HEADS_TABLE,
        super::RECIPE_REFERENCES_TABLE,
        super::COLLECTION_STATE_TABLE,
        super::COLLECTIONS_TABLE,
        super::COLLECTION_NAME_INDEX_TABLE,
        super::RECENT_QUERY_TABLE,
        super::RECENT_ORDER_INDEX_TABLE,
        super::ACTIVE_VIEW_TABLE,
        super::COLLECTION_INTEGRITY_TABLE,
        super::HISTORY_STATE_TABLE,
        super::HISTORY_REVISIONS_TABLE,
        super::HISTORY_BLOBS_TABLE,
        super::HISTORY_BLOB_REFS_TABLE,
        super::METADATA_DOCUMENTS_TABLE,
        super::METADATA_INDEX_TABLE,
        super::METADATA_REVISION_TABLE,
        super::TAG_STATE_TABLE,
        super::TAG_PATH_INDEX_TABLE,
        super::TAG_ALIAS_INDEX_TABLE,
        super::TAG_PHOTO_INDEX_TABLE,
        super::PHOTO_TAG_INDEX_TABLE,
    ] {
        transaction
            .open_table(table)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
    }
    Ok(())
}
