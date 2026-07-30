use rusttable_catalog::{
    CatalogState, ImportCandidate, ImportError, ImportOutcome, ImportRecord, ImportRepository,
    ImportService, RepositoryError, SourcePath,
};
use rusttable_core::{
    AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, PhotoId, Revision,
};
use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};
use std::collections::BTreeMap;

fn hash(byte: u8) -> ContentHash {
    ContentHash::Sha256([byte; 32])
}

fn probe() -> ImageProbe {
    ImageProbe::new(
        InputFormat::Png,
        ImageDimensions::new(2, 1).expect("dimensions"),
    )
}

fn candidate(source: &str, photo_id: u128, asset_id: u128, byte: u8) -> ImportCandidate {
    ImportCandidate::new(
        PhotoId::new(photo_id).expect("photo ID"),
        AssetId::new(asset_id).expect("asset ID"),
        SourcePath::new(source).expect("source"),
        hash(byte),
        ByteLength::from_bytes(8),
        probe(),
        ImageMetadata::empty(),
    )
    .expect("candidate")
}

#[derive(Default)]
struct FakeRepository {
    records: BTreeMap<SourcePath, ImportRecord>,
    fail_commit: bool,
}

impl ImportRepository for FakeRepository {
    fn find_by_source(&self, source: &SourcePath) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self.records.get(source).cloned())
    }

    fn find_by_photo_id(&self, photo_id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self
            .records
            .values()
            .find(|record| record.photo().id() == photo_id)
            .cloned())
    }

    fn find_by_asset_id(&self, asset_id: AssetId) -> Result<Option<ImportRecord>, RepositoryError> {
        Ok(self
            .records
            .values()
            .find(|record| record.photo().primary_asset_id() == asset_id)
            .cloned())
    }

    fn commit(&mut self, record: &ImportRecord) -> Result<(), RepositoryError> {
        if self.fail_commit {
            return Err(RepositoryError::CommitFailure);
        }
        if self.records.contains_key(record.source()) {
            return Err(RepositoryError::SourceConflict {
                source: record.source().clone(),
            });
        }
        if self.find_by_photo_id(record.photo().id())?.is_some() {
            return Err(RepositoryError::PhotoIdConflict {
                photo_id: record.photo().id(),
            });
        }
        if self
            .find_by_asset_id(record.photo().primary_asset_id())?
            .is_some()
        {
            return Err(RepositoryError::AssetIdConflict {
                asset_id: record.photo().primary_asset_id(),
            });
        }
        self.records.insert(record.source().clone(), record.clone());
        Ok(())
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        Ok(self.records.values().cloned().collect())
    }
}

fn register(
    state: &mut CatalogState,
    repository: &mut FakeRepository,
    candidate: &ImportCandidate,
) -> Result<ImportOutcome, ImportError> {
    ImportService::register(state, state.revision(), candidate, repository)
}

#[test]
fn new_import_commits_one_primary_photo_and_advances_once() {
    let mut state = CatalogState::new();
    let mut repository = FakeRepository::default();
    let candidate = candidate("one.raw", 1, 2, 7);

    let outcome = register(&mut state, &mut repository, &candidate).expect("import");
    let ImportOutcome::Imported { record, revision } = outcome else {
        panic!("new source must import");
    };

    assert_eq!(revision, Revision::from_u64(1));
    assert_eq!(state.revision(), revision);
    assert_eq!(record.photo().revision(), Revision::ZERO);
    assert_eq!(record.photo().assets().count(), 1);
    assert_eq!(record.photo().primary_asset().role(), AssetRole::Primary);
    assert_eq!(repository.list().expect("list").len(), 1);
}

#[test]
fn identical_retry_is_already_present_without_rewriting_authoritative_values() {
    let mut state = CatalogState::new();
    let mut repository = FakeRepository::default();
    let first = candidate("one.raw", 1, 2, 7);
    let authoritative = first.clone();
    register(&mut state, &mut repository, &first).expect("first import");
    let before = state.clone();

    let retry = candidate("one.raw", 9, 10, 7);
    let outcome = register(&mut state, &mut repository, &retry).expect("idempotent retry");
    let ImportOutcome::AlreadyPresent { record, revision } = outcome else {
        panic!("same content must be already present");
    };

    assert_eq!(record.source(), authoritative.source());
    assert_eq!(record.photo().id(), PhotoId::new(1).unwrap());
    assert_eq!(revision, before.revision());
    assert_eq!(state, before);
}

#[test]
fn changed_content_and_repository_failures_are_atomic() {
    let mut state = CatalogState::new();
    let mut repository = FakeRepository::default();
    let first = candidate("one.raw", 1, 2, 7);
    register(&mut state, &mut repository, &first).expect("first import");
    let before_state = state.clone();
    let before_repo = repository.list().expect("list");

    let changed_candidate = candidate("one.raw", 3, 4, 8);
    let changed = register(&mut state, &mut repository, &changed_candidate);
    assert!(matches!(
        changed,
        Err(ImportError::SourceContentChanged { .. })
    ));
    assert_eq!(state, before_state);
    assert_eq!(repository.list().expect("list"), before_repo);

    let mut new_repository = FakeRepository {
        fail_commit: true,
        ..repository
    };
    let failed_candidate = candidate("two.raw", 3, 4, 8);
    let failed = register(&mut state, &mut new_repository, &failed_candidate);
    assert!(matches!(
        failed,
        Err(ImportError::Repository(RepositoryError::CommitFailure))
    ));
    assert_eq!(state, before_state);
}

#[test]
fn candidate_rejects_zero_length_at_import_boundary() {
    let result = ImportCandidate::new(
        PhotoId::new(1).unwrap(),
        AssetId::new(2).unwrap(),
        SourcePath::new("one.raw").unwrap(),
        hash(1),
        ByteLength::ZERO,
        probe(),
        ImageMetadata::empty(),
    );
    assert!(result.is_err());
}
