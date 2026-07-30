mod support;

use rusttable_catalog::{
    CatalogMetadataBatch, CatalogMetadataBatchEdit, CatalogMetadataCandidate, CatalogMetadataEdit,
    CatalogMetadataError, CatalogMetadataKey, CatalogMetadataPrivacy, CatalogMetadataProvenance,
    CatalogMetadataRepository, CatalogMetadataSource, CatalogMetadataValue, CatalogMetadataValues,
    ImportRepository,
};
use rusttable_catalog_store::{RedbCatalogMetadataRepository, RedbImportRepository};
use rusttable_core::{PhotoId, Revision};

fn set(
    key: &CatalogMetadataKey,
    value: &str,
    privacy: CatalogMetadataPrivacy,
) -> CatalogMetadataEdit {
    CatalogMetadataEdit::Set {
        key: key.clone(),
        candidate: CatalogMetadataCandidate::new(
            CatalogMetadataValues::new(vec![CatalogMetadataValue::Text(value.to_owned())]).unwrap(),
            privacy,
            CatalogMetadataProvenance::new(CatalogMetadataSource::CatalogEdit, [7; 32]),
        ),
    }
}

fn seed(path: &std::path::Path) {
    let mut imports = RedbImportRepository::open(path).unwrap();
    imports
        .commit(&support::record("one.raw", 1, 11, 1))
        .unwrap();
    imports
        .commit(&support::record("two.raw", 2, 22, 2))
        .unwrap();
}

#[test]
fn batch_persistence_indexes_and_rebuild_are_deterministic() {
    let title = CatalogMetadataKey::new("Xmp.dc", "title").unwrap();
    let location = CatalogMetadataKey::new("rusttable", "location").unwrap();
    let mut receipts = Vec::new();
    for reverse in [false, true] {
        let path = support::temp_path("metadata-determinism");
        seed(&path);
        let mut repository = RedbCatalogMetadataRepository::open(&path).unwrap();
        let mut photos = vec![
            CatalogMetadataBatchEdit {
                photo_id: PhotoId::new(1).unwrap(),
                expected_revision: Revision::ZERO,
                edits: vec![
                    set(&title, "shared", CatalogMetadataPrivacy::Public),
                    set(&location, "secret-one", CatalogMetadataPrivacy::Sensitive),
                ],
            },
            CatalogMetadataBatchEdit {
                photo_id: PhotoId::new(2).unwrap(),
                expected_revision: Revision::ZERO,
                edits: vec![set(&title, "shared", CatalogMetadataPrivacy::Public)],
            },
        ];
        if reverse {
            photos.reverse();
        }
        let receipt = repository
            .apply_batch(&CatalogMetadataBatch {
                expected_catalog_revision: Revision::ZERO,
                photos,
            })
            .unwrap();
        assert_eq!(receipt.catalog_revision, Revision::from_u64(1));
        assert_eq!(
            repository
                .find(&title, &CatalogMetadataValue::Text("shared".to_owned()))
                .unwrap(),
            vec![PhotoId::new(1).unwrap(), PhotoId::new(2).unwrap()]
        );
        assert!(
            repository
                .find(
                    &location,
                    &CatalogMetadataValue::Text("secret-one".to_owned())
                )
                .unwrap()
                .is_empty()
        );
        assert_eq!(repository.rebuild_indexes().unwrap(), 2);
        drop(repository);
        let reopened = RedbCatalogMetadataRepository::open(&path).unwrap();
        assert_eq!(
            reopened
                .get(PhotoId::new(1).unwrap())
                .unwrap()
                .unwrap()
                .revision(),
            Revision::from_u64(1)
        );
        receipts.push(receipt.state_sha256);
        support::remove(&path);
    }
    assert_eq!(receipts[0], receipts[1]);
}

#[test]
fn stale_or_failed_batches_leave_all_documents_unchanged() {
    let path = support::temp_path("metadata-atomic");
    seed(&path);
    let title = CatalogMetadataKey::new("Xmp.dc", "title").unwrap();
    let batch = CatalogMetadataBatch {
        expected_catalog_revision: Revision::ZERO,
        photos: vec![CatalogMetadataBatchEdit {
            photo_id: PhotoId::new(1).unwrap(),
            expected_revision: Revision::ZERO,
            edits: vec![set(&title, "before", CatalogMetadataPrivacy::Public)],
        }],
    };
    let mut failing = RedbCatalogMetadataRepository::open_with_before_commit_hook(&path, || {
        Err(CatalogMetadataError::CommitFailure)
    })
    .unwrap();
    assert_eq!(
        failing.apply_batch(&batch),
        Err(CatalogMetadataError::CommitFailure)
    );
    drop(failing);
    let mut repository = RedbCatalogMetadataRepository::open(&path).unwrap();
    assert_eq!(repository.catalog_revision().unwrap(), Revision::ZERO);
    assert!(repository.get(PhotoId::new(1).unwrap()).unwrap().is_none());
    repository.apply_batch(&batch).unwrap();
    let stale = CatalogMetadataBatch {
        expected_catalog_revision: Revision::ZERO,
        photos: vec![CatalogMetadataBatchEdit {
            photo_id: PhotoId::new(2).unwrap(),
            expected_revision: Revision::ZERO,
            edits: vec![set(&title, "after", CatalogMetadataPrivacy::Public)],
        }],
    };
    assert!(matches!(
        repository.apply_batch(&stale),
        Err(CatalogMetadataError::RevisionConflict { .. })
    ));
    assert!(repository.get(PhotoId::new(2).unwrap()).unwrap().is_none());
    assert_eq!(
        repository.catalog_revision().unwrap(),
        Revision::from_u64(1)
    );
    let partially_stale = CatalogMetadataBatch {
        expected_catalog_revision: Revision::from_u64(1),
        photos: vec![
            CatalogMetadataBatchEdit {
                photo_id: PhotoId::new(1).unwrap(),
                expected_revision: Revision::from_u64(1),
                edits: vec![set(&title, "would-change", CatalogMetadataPrivacy::Public)],
            },
            CatalogMetadataBatchEdit {
                photo_id: PhotoId::new(2).unwrap(),
                expected_revision: Revision::from_u64(1),
                edits: vec![set(&title, "invalid", CatalogMetadataPrivacy::Public)],
            },
        ],
    };
    assert!(matches!(
        repository.apply_batch(&partially_stale),
        Err(CatalogMetadataError::RevisionConflict { .. })
    ));
    assert_eq!(
        repository
            .find(&title, &CatalogMetadataValue::Text("before".to_owned()))
            .unwrap(),
        vec![PhotoId::new(1).unwrap()]
    );
    assert!(
        repository
            .find(
                &title,
                &CatalogMetadataValue::Text("would-change".to_owned())
            )
            .unwrap()
            .is_empty()
    );
    support::remove(&path);
}
