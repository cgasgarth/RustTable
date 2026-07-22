mod support;

use redb::{Database, TableDefinition};
use rusttable_catalog::{
    EditRepository, ImportDetails, ImportMetadataStatus, ImportMetadataSummary, ImportRegistration,
    ImportRegistrationReceipt, ImportRepository, ReferencePathIdentity,
};
use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_core::{
    ByteLength, Edit, EditId, ImageMetadata, MetadataEntry, MetadataText, Revision,
};

fn edit(photo_id: rusttable_core::PhotoId, edit_id: u128) -> Edit {
    Edit::new(
        EditId::new(edit_id).expect("edit ID"),
        photo_id,
        Revision::ZERO,
        [],
    )
    .expect("empty edit")
}

fn registration(
    record: &rusttable_catalog::ImportRecord,
    edit: &Edit,
    alias: &str,
    path_identity: u8,
) -> ImportRegistration {
    let asset = record.photo().primary_asset();
    let rusttable_core::ContentHash::Sha256(content_sha256) = asset.content_hash();
    let receipt = ImportRegistrationReceipt::new(
        alias.to_owned(),
        content_sha256,
        ByteLength::from_bytes(asset.byte_length().get()),
        record.photo().id(),
        record.photo().primary_asset_id(),
        edit.id(),
    )
    .expect("safe receipt");
    let details = ImportDetails::new(ImportMetadataSummary::from_record(record), receipt);
    ImportRegistration::new(details, ReferencePathIdentity::new([path_identity; 32]))
}

fn reference_source(path: &str, marker: char) -> rusttable_catalog::SourcePath {
    let hash = marker.to_string().repeat(64);
    let mut encoded_path = String::new();
    for byte in path.bytes() {
        use std::fmt::Write as _;

        write!(&mut encoded_path, "{byte:02x}").expect("string formatting");
    }
    rusttable_catalog::SourcePath::new(&format!("reference-v1/{hash}/{encoded_path}"))
        .expect("reference source")
}

#[test]
fn registration_details_survive_reopen_and_are_found_by_photo_id() {
    let path = support::temp_path("import-details-reopen");
    let record = support::record("reference/first", 10, 11, 12);
    let edit = edit(record.photo().id(), 13);
    let registration = registration(&record, &edit, "first.png", 1);

    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        repository
            .commit_import_with_edit(&record, &edit, &registration)
            .expect("commit");
        assert_eq!(
            repository
                .find_import_details_by_photo_id(record.photo().id())
                .expect("lookup"),
            Some(registration.details().clone())
        );
    }

    let repository = RedbCatalogRepository::open(&path).expect("reopen");
    assert_eq!(
        repository
            .find_import_details_by_photo_id(record.photo().id())
            .expect("lookup after restart"),
        Some(registration.details().clone())
    );
    assert_eq!(
        repository
            .find_import_details_by_photo_id(rusttable_core::PhotoId::new(99).unwrap())
            .expect("missing lookup"),
        None
    );
    support::remove(&path);
}

#[test]
fn changed_content_at_one_path_records_the_replaced_photo_after_restart() {
    let path = support::temp_path("import-details-path-reuse");
    let first = support::record("reference/first", 10, 11, 12);
    let first_edit = edit(first.photo().id(), 13);
    let first_registration = registration(&first, &first_edit, "first.png", 7);
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        repository
            .commit_import_with_edit(&first, &first_edit, &first_registration)
            .expect("first commit");
    }

    let second = support::record("reference/second", 20, 21, 22);
    let second_edit = edit(second.photo().id(), 23);
    let second_registration = registration(&second, &second_edit, "first.png", 7);
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("restart");
        repository
            .commit_import_with_edit(&second, &second_edit, &second_registration)
            .expect("second commit");
    }

    let repository = RedbCatalogRepository::open(&path).expect("second restart");
    let details = repository
        .find_import_details_by_photo_id(second.photo().id())
        .expect("lookup")
        .expect("details");
    assert_eq!(
        details.receipt().replaces_photo_id(),
        Some(first.photo().id())
    );
    support::remove(&path);
}

#[test]
fn precommit_failure_leaves_no_record_edit_details_or_path_identity() {
    let path = support::temp_path("import-details-rollback");
    let failed = support::record("reference/failed", 10, 11, 12);
    let failed_edit = edit(failed.photo().id(), 13);
    let failed_registration = registration(&failed, &failed_edit, "failed.png", 9);
    let mut repository = RedbCatalogRepository::open_with_before_commit_hook(&path, || {
        Err(AtomicCatalogStoreError::CommitFailed)
    })
    .expect("open with hook");

    assert_eq!(
        repository.commit_import_with_edit(&failed, &failed_edit, &failed_registration),
        Err(AtomicCatalogStoreError::CommitFailed)
    );
    drop(repository);

    let succeeding = support::record("reference/succeeding", 20, 21, 22);
    let succeeding_edit = edit(succeeding.photo().id(), 23);
    let succeeding_registration = registration(&succeeding, &succeeding_edit, "failed.png", 9);
    let mut repository = RedbCatalogRepository::open(&path).expect("reopen");
    assert_eq!(
        repository
            .find_import_details_by_photo_id(failed.photo().id())
            .expect("failed details lookup"),
        None
    );
    assert!(
        repository
            .find_by_photo_id(failed.photo().id())
            .unwrap()
            .is_none()
    );
    assert!(
        repository
            .find_by_edit_id(failed_edit.id())
            .unwrap()
            .is_none()
    );
    repository
        .commit_import_with_edit(&succeeding, &succeeding_edit, &succeeding_registration)
        .expect("succeeding commit");
    let details = repository
        .find_import_details_by_photo_id(succeeding.photo().id())
        .unwrap()
        .expect("succeeding details");
    assert_eq!(details.receipt().replaces_photo_id(), None);
    support::remove(&path);
}

#[test]
fn unavailable_metadata_status_and_refresh_are_atomic_and_same_photo() {
    let path = support::temp_path("import-metadata-refresh");
    let record = support::record("reference/refresh", 10, 11, 12);
    let edit = edit(record.photo().id(), 13);
    let asset = record.photo().primary_asset();
    let rusttable_core::ContentHash::Sha256(content_sha256) = asset.content_hash();
    let receipt = ImportRegistrationReceipt::new(
        "refresh.png".to_owned(),
        content_sha256,
        ByteLength::from_bytes(asset.byte_length().get()),
        record.photo().id(),
        record.photo().primary_asset_id(),
        edit.id(),
    )
    .unwrap();
    let details = ImportDetails::new(
        ImportMetadataSummary::from_record_with_status(&record, ImportMetadataStatus::Unavailable),
        receipt,
    );
    let registration = ImportRegistration::new(details, [7; 32].into());
    let mut repository = RedbCatalogRepository::open(&path).unwrap();
    repository
        .commit_import_with_edit(&record, &edit, &registration)
        .unwrap();

    let refreshed_metadata = ImageMetadata::from_entries([MetadataEntry::CameraModel(
        MetadataText::from_bytes(b"reparsed camera".to_vec()).unwrap(),
    )])
    .unwrap();
    repository
        .refresh_import_metadata(record.photo().id(), refreshed_metadata)
        .unwrap();
    let (updated_record, _) = repository
        .find_by_content(content_sha256, asset.byte_length().get())
        .unwrap()
        .unwrap();
    let updated_details = repository
        .find_import_details_by_photo_id(record.photo().id())
        .unwrap()
        .unwrap();

    assert_eq!(updated_record.photo().id(), record.photo().id());
    assert_eq!(updated_record.metadata().len(), 1);
    assert_eq!(
        updated_details.summary().metadata_status(),
        ImportMetadataStatus::Available
    );
    assert_eq!(ImportRepository::list(&repository).unwrap().len(), 1);
    support::remove(&path);
}

#[test]
fn source_identity_migration_rebuilds_normalized_path_index_and_reports_ambiguity() {
    const SCHEMA: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_schema");
    const VERSION_KEY: &[u8] = b"schema-version";
    let path = support::temp_path("source-identity-migration");
    let physical = "/photos/./roll/../same.jpg";
    let first = support::record(reference_source(physical, 'a').as_str(), 51, 52, 11);
    let second = support::record(reference_source(physical, 'b').as_str(), 53, 54, 12);
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        let first_edit = edit(first.photo().id(), 55);
        let second_edit = edit(second.photo().id(), 56);
        repository
            .commit_import_with_edit(
                &first,
                &first_edit,
                &registration(&first, &first_edit, "first.jpg", 1),
            )
            .expect("first commit");
        repository
            .commit_import_with_edit(
                &second,
                &second_edit,
                &registration(&second, &second_edit, "second.jpg", 2),
            )
            .expect("second commit");
    }
    {
        let database = Database::open(&path).expect("raw reopen");
        let transaction = database.begin_write().expect("write version");
        let mut schema = transaction.open_table(SCHEMA).expect("schema");
        schema
            .insert(VERSION_KEY, &[9][..])
            .expect("legacy version");
        drop(schema);
        transaction.commit().expect("version commit");
    }

    let repository = RedbCatalogRepository::open(&path).expect("migrate");
    let identity = rusttable_import_path_identity(physical);
    assert!(
        repository
            .find_by_reference_path(identity.into())
            .expect("path lookup")
            .is_some()
    );
    let report = repository
        .source_reconciliation_report()
        .expect("reconciliation report");
    assert_eq!(report.migrated_entries, 1);
    assert_eq!(report.ambiguous_entries, 1);
    assert_eq!(report.invalid_entries, 0);
    assert_eq!(
        ImportRepository::list(&repository).expect("all rows").len(),
        2
    );
    support::remove(&path);
}

fn rusttable_import_path_identity(path: &str) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let normalized = "/photos/same.jpg";
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable-reference-path-v1\0");
    hasher.update(normalized.as_bytes());
    let _ = path;
    hasher.finalize().into()
}
