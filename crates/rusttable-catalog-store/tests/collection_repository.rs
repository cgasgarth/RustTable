use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_catalog::{
    ActiveLibraryView, CollectionCommand, CollectionQuery, CollectionRepository, CollectionSort,
    CollectionViewDefinition, GroupCollapsePolicy, SavedCollection,
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
