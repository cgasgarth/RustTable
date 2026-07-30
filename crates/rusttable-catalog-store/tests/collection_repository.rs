use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_catalog::{
    ActiveLibraryView, ActiveLighttableProperty, ActiveLighttableSort,
    ActiveLighttableSortDirection, ActiveLighttableState, CollectionCommand, CollectionQuery,
    CollectionRepository, CollectionRepositoryError, CollectionSort, CollectionViewDefinition,
    GroupCollapsePolicy, SavedCollection,
};
use rusttable_catalog_store::RedbCollectionRepository;

static NEXT_PATH: AtomicU64 = AtomicU64::new(0);

fn path() -> PathBuf {
    let suffix = NEXT_PATH.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "rusttable-collections-{}-{suffix}.redb",
        std::process::id()
    ))
}

fn view() -> CollectionViewDefinition {
    CollectionViewDefinition::new(
        CollectionQuery::AllPhotos,
        CollectionSort::FilenameAscending,
        GroupCollapsePolicy::KeepExpanded,
    )
}

#[test]
fn collections_reopen_with_active_view_and_canonical_recent_identity() {
    let path = path();
    let id = rusttable_catalog::CollectionId::new(1).expect("id");
    let collection = SavedCollection::new(id, "Favorites", None, view()).expect("collection");
    let mut repository = RedbCollectionRepository::open(&path).expect("open");
    repository
        .apply(CollectionCommand::Create(collection))
        .expect("create");
    repository
        .apply(CollectionCommand::SetActive(ActiveLibraryView::Saved(id)))
        .expect("active");
    repository
        .apply(CollectionCommand::MarkRecent {
            definition: view(),
            last_used: 7,
        })
        .expect("recent");
    repository.check_integrity().expect("integrity");
    drop(repository);

    let repository = RedbCollectionRepository::open(&path).expect("reopen");
    let state = repository.load().expect("load");
    assert_eq!(state.active().saved_id(), Some(id));
    assert_eq!(state.recent().len(), 1);
    assert_eq!(state.recent()[0].identity(), view().identity());
    let _ = std::fs::remove_file(path);
}

#[test]
fn deleting_active_collection_reopens_as_all_photos() {
    let path = path();
    let id = rusttable_catalog::CollectionId::new(2).expect("id");
    let mut repository = RedbCollectionRepository::open(&path).expect("open");
    repository
        .apply(CollectionCommand::Create(
            SavedCollection::new(id, "Work", None, view()).expect("collection"),
        ))
        .expect("create");
    repository
        .apply(CollectionCommand::SetActive(ActiveLibraryView::Saved(id)))
        .expect("active");
    repository
        .apply(CollectionCommand::Delete {
            id,
            expected_revision: 1,
        })
        .expect("delete");
    assert!(
        repository
            .load()
            .expect("load")
            .active()
            .saved_id()
            .is_none()
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn active_lighttable_state_roundtrips_and_reconciles_as_versioned_payload() {
    let path = path();
    let state = ActiveLighttableState::new(
        ActiveLighttableProperty::Folders,
        "holiday",
        ActiveLighttableSort::Rating,
        ActiveLighttableSortDirection::Descending,
        [9, 3, 9],
    );
    let repository = RedbCollectionRepository::open(&path).expect("open");
    repository
        .persist_active_lighttable_state(&state)
        .expect("persist active lighttable state");
    drop(repository);

    let repository = RedbCollectionRepository::open(&path).expect("reopen");
    let restored = repository
        .load_active_lighttable_state()
        .expect("load active lighttable state");
    assert_eq!(restored, state);
    assert_eq!(restored.version(), 1);
    let _ = std::fs::remove_file(path);
}

#[test]
fn active_lighttable_commit_failure_keeps_the_previous_payload() {
    let path = path();
    let first = ActiveLighttableState::new(
        ActiveLighttableProperty::Filename,
        "before",
        ActiveLighttableSort::Filename,
        ActiveLighttableSortDirection::Ascending,
        [1],
    );
    let second = ActiveLighttableState::new(
        ActiveLighttableProperty::Rating,
        "after",
        ActiveLighttableSort::CaptureTime,
        ActiveLighttableSortDirection::Descending,
        [2],
    );
    let repository = RedbCollectionRepository::open(&path).expect("open");
    repository
        .persist_active_lighttable_state(&first)
        .expect("persist initial state");
    drop(repository);

    let failing = RedbCollectionRepository::open_with_before_commit_hook(&path, || {
        Err(CollectionRepositoryError::CommitFailed)
    })
    .expect("open failing repository");
    assert_eq!(
        failing.persist_active_lighttable_state(&second),
        Err(CollectionRepositoryError::CommitFailed)
    );
    drop(failing);

    let repository = RedbCollectionRepository::open(&path).expect("reopen");
    assert_eq!(
        repository
            .load_active_lighttable_state()
            .expect("load previous state"),
        first
    );
    let _ = std::fs::remove_file(path);
}
