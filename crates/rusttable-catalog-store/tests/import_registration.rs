mod support;

use rusttable_catalog::{
    EditRepository, ImportDetails, ImportMetadataSummary, ImportRegistration,
    ImportRegistrationReceipt, ImportRepository, ReferencePathIdentity,
};
use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_core::{ByteLength, Edit, EditId, Revision};

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
