use rusttable_catalog::RepositoryError;

pub(super) fn validate_tables(transaction: &redb::ReadTransaction) -> Result<(), RepositoryError> {
    for table in [
        super::RECORDS_TABLE,
        super::PHOTO_INDEX_TABLE,
        super::ASSET_INDEX_TABLE,
        super::PHOTO_ORGANIZATION_TABLE,
        super::ORGANIZATION_REVISION_TABLE,
        super::PHOTO_GROUPS_TABLE,
        super::PHOTO_GROUP_MEMBER_INDEX_TABLE,
        super::EDITS_TABLE,
        super::IMPORT_DETAILS_TABLE,
        super::REFERENCE_PATH_INDEX_TABLE,
        super::SOURCE_RECONCILIATION_TABLE,
        super::DUPLICATE_EVIDENCE_TABLE,
        super::DUPLICATE_SOURCE_INDEX_TABLE,
        super::DUPLICATE_EXACT_INDEX_TABLE,
        super::DUPLICATE_EMBEDDED_INDEX_TABLE,
        super::DUPLICATE_VISUAL_INDEX_TABLE,
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
        super::VIRTUAL_COPIES_TABLE,
        super::VIRTUAL_COPY_STATE_TABLE,
    ] {
        transaction
            .open_table(table)
            .map_err(|_| RepositoryError::CorruptPersistedData)?;
    }
    Ok(())
}
