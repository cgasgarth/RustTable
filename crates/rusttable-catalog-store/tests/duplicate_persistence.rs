mod support;

use redb::{Database, TableDefinition};
use rusttable_catalog::{
    DuplicateClassification, DuplicateEvidence, ExactContentIdentity, ImportDetails,
    ImportMetadataSummary, ImportRegistration, ImportRegistrationReceipt, ReferencePathIdentity,
    VisualFingerprint,
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
    source_marker: u8,
    visual: u64,
) -> ImportRegistration {
    let asset = record.photo().primary_asset();
    let rusttable_core::ContentHash::Sha256(content_sha256) = asset.content_hash();
    let receipt = ImportRegistrationReceipt::new(
        format!("{}.png", record.photo().id().get()),
        content_sha256,
        ByteLength::from_bytes(asset.byte_length().get()),
        record.photo().id(),
        record.photo().primary_asset_id(),
        edit.id(),
    )
    .expect("receipt");
    let source = ReferencePathIdentity::new([source_marker; 32]);
    let evidence = DuplicateEvidence::new(
        record.photo().id(),
        source,
        ExactContentIdentity::new(content_sha256, asset.byte_length().get()),
        None,
        VisualFingerprint::new(visual, 0, 4, 3),
    );
    ImportRegistration::new(
        ImportDetails::new(ImportMetadataSummary::from_record(record), receipt),
        source,
    )
    .with_duplicate_evidence(evidence)
}

#[test]
fn exact_and_probable_evidence_survive_reopen_without_merging_photos() {
    let path = support::temp_path("duplicate-reopen");
    let first = support::record("first", 1, 11, 7);
    let first_edit = edit(first.photo().id(), 21);
    let first_registration = registration(&first, &first_edit, 1, 0);
    let exact = support::record("exact", 2, 12, 7);
    let exact_edit = edit(exact.photo().id(), 22);
    let exact_registration = registration(&exact, &exact_edit, 2, u64::MAX);
    let probable = support::record("probable", 3, 13, 8);
    let probable_edit = edit(probable.photo().id(), 23);
    let probable_registration = registration(&probable, &probable_edit, 3, 0b11_1111);
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        repository
            .commit_import_with_edit(&first, &first_edit, &first_registration)
            .expect("first commit");

        let exact_matches = repository
            .find_duplicates(exact_registration.duplicate_evidence().unwrap())
            .expect("exact lookup");
        assert_eq!(
            exact_matches.matches().next().unwrap().classification(),
            DuplicateClassification::ExactContent
        );
        repository
            .commit_import_with_edit(&exact, &exact_edit, &exact_registration)
            .expect("distinct exact import");
        repository
            .commit_import_with_edit(&probable, &probable_edit, &probable_registration)
            .expect("distinct probable import");
    }

    let repository = RedbCatalogRepository::open(&path).expect("reopen");
    let probable_matches = repository
        .find_duplicates(probable_registration.duplicate_evidence().unwrap())
        .expect("probable lookup")
        .matches()
        .copied()
        .filter(|duplicate| duplicate.photo_id() != probable.photo().id())
        .collect::<Vec<_>>();
    assert_eq!(probable_matches.len(), 1);
    assert_eq!(
        probable_matches[0].classification(),
        DuplicateClassification::ProbableVisual
    );
    assert_eq!(probable_matches[0].photo_id(), first.photo().id());
    assert_eq!(
        rusttable_catalog::ImportRepository::list(&repository)
            .unwrap()
            .len(),
        3
    );
    support::remove(&path);
}

#[test]
fn failed_commit_rolls_back_duplicate_profile_and_all_indexes() {
    let path = support::temp_path("duplicate-rollback");
    let failed = support::record("failed", 1, 11, 7);
    let failed_edit = edit(failed.photo().id(), 21);
    let failed_registration = registration(&failed, &failed_edit, 1, 0);
    let mut repository = RedbCatalogRepository::open_with_before_commit_hook(&path, || {
        Err(AtomicCatalogStoreError::CommitFailed)
    })
    .expect("open with hook");

    assert_eq!(
        repository.commit_import_with_edit(&failed, &failed_edit, &failed_registration),
        Err(AtomicCatalogStoreError::CommitFailed)
    );
    drop(repository);

    let candidate = support::record("candidate", 2, 12, 7);
    let candidate_edit = edit(candidate.photo().id(), 22);
    let candidate_registration = registration(&candidate, &candidate_edit, 2, 0);
    let repository = RedbCatalogRepository::open(&path).expect("reopen");
    let matches = repository
        .find_duplicates(candidate_registration.duplicate_evidence().unwrap())
        .expect("lookup after rollback");
    assert_eq!(matches.matches().len(), 0);
    assert!(!matches.truncated());
    support::remove(&path);
}

#[test]
fn metadata_refresh_reindexes_embedded_identity_atomically() {
    let path = support::temp_path("duplicate-metadata-refresh");
    let first = support::record("first", 1, 11, 7);
    let first_edit = edit(first.photo().id(), 21);
    let first_registration = registration(&first, &first_edit, 1, 0);
    let metadata = ImageMetadata::from_entries([
        MetadataEntry::CameraModel(MetadataText::new("camera-7").unwrap()),
        MetadataEntry::CaptureDateTimeOriginal(MetadataText::new("2026:07:22 12:34:56").unwrap()),
    ])
    .unwrap();
    let candidate = support::record("candidate", 2, 12, 8).with_metadata(metadata.clone());
    let candidate_evidence = DuplicateEvidence::from_record(
        &candidate,
        ReferencePathIdentity::new([2; 32]),
        VisualFingerprint::new(u64::MAX, u64::MAX, 4, 3),
    );
    let mut repository = RedbCatalogRepository::open(&path).expect("open");
    repository
        .commit_import_with_edit(&first, &first_edit, &first_registration)
        .expect("first commit");

    repository
        .refresh_import_metadata(first.photo().id(), metadata)
        .expect("metadata refresh");
    let duplicate = repository
        .find_duplicates(candidate_evidence)
        .expect("embedded lookup")
        .matches()
        .next()
        .copied()
        .expect("embedded match");

    assert_eq!(
        duplicate.classification(),
        DuplicateClassification::EmbeddedIdentity
    );
    assert_eq!(duplicate.photo_id(), first.photo().id());
    support::remove(&path);
}

#[test]
fn schema_v12_backfills_available_source_exact_and_embedded_evidence() {
    const SCHEMA: TableDefinition<&[u8], &[u8]> = TableDefinition::new("rusttable_schema");
    const VERSION_KEY: &[u8] = b"schema-version";
    let path = support::temp_path("duplicate-v12-backfill");
    let record = support::record("legacy", 1, 11, 7);
    let edit = edit(record.photo().id(), 21);
    let current = registration(&record, &edit, 1, 0);
    let legacy =
        ImportRegistration::new(current.details().clone(), current.reference_path_identity());
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        repository
            .commit_import_with_edit(&record, &edit, &legacy)
            .expect("legacy commit");
    }
    {
        let database = Database::open(&path).expect("raw open");
        let transaction = database.begin_write().expect("write");
        let mut schema = transaction.open_table(SCHEMA).expect("schema");
        schema.insert(VERSION_KEY, &[12][..]).expect("downgrade");
        drop(schema);
        transaction.commit().expect("commit version");
    }

    let repository = RedbCatalogRepository::open(&path).expect("migrate");
    let matches = repository
        .find_duplicates(current.duplicate_evidence().unwrap())
        .expect("backfilled query");

    assert_eq!(matches.matches().len(), 1);
    assert_eq!(
        matches.matches().next().unwrap().classification(),
        DuplicateClassification::Source
    );
    support::remove(&path);
}
