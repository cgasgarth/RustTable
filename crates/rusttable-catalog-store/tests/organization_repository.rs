mod support;

use std::sync::{Arc, Mutex};

use rusttable_catalog::{CatalogChangeEvent, CatalogCommand, ColorLabel, ImportRepository, Rating};
use rusttable_catalog_store::{AtomicCatalogStoreError, RedbCatalogRepository};
use rusttable_core::PhotoId;

fn seed(path: &std::path::Path) {
    let mut repository = RedbCatalogRepository::open(path).expect("open catalog");
    for (index, source) in ["first.raw", "second.raw"].into_iter().enumerate() {
        repository
            .commit(&support::record(
                source,
                u128::try_from(index + 1).unwrap(),
                u128::try_from(index + 11).unwrap(),
                u8::try_from(index + 1).unwrap(),
            ))
            .expect("seed record");
    }
}

#[test]
fn organization_commands_survive_reload_with_independent_labels() {
    let path = support::temp_path("organization-restart");
    seed(&path);
    {
        let mut repository = RedbCatalogRepository::open(&path).expect("open");
        repository
            .apply_organization_command(&CatalogCommand::SetRating {
                photo_ids: vec![photo(1), photo(2)],
                rating: Rating::Five,
            })
            .expect("rating commit");
        repository
            .apply_organization_command(&CatalogCommand::SetRejection {
                photo_ids: vec![photo(1)],
                rejected: true,
            })
            .expect("reject commit");
        repository
            .apply_organization_command(&CatalogCommand::SetColorLabel {
                photo_ids: vec![photo(1)],
                label: ColorLabel::Red,
                enabled: true,
            })
            .expect("red label commit");
        repository
            .apply_organization_command(&CatalogCommand::SetColorLabel {
                photo_ids: vec![photo(1)],
                label: ColorLabel::Blue,
                enabled: true,
            })
            .expect("blue label commit");
        assert_eq!(repository.organization_revision().unwrap().get(), 4);
    }

    let repository = RedbCatalogRepository::open(&path).expect("restart");
    let states = repository
        .organization_states()
        .expect("organization states");
    let first = states.get(&photo(1)).unwrap();
    assert_eq!(first.rating, Rating::Five);
    assert!(first.rejected);
    assert_eq!(
        first.color_labels.iter().copied().collect::<Vec<_>>(),
        [ColorLabel::Red, ColorLabel::Blue,]
    );
    assert_eq!(states.get(&photo(2)).unwrap().color_labels.len(), 0);
    support::remove(&path);
}

#[test]
fn numeric_rating_clears_rejection_and_reject_toggle_preserves_stars() {
    let path = support::temp_path("organization-semantics");
    seed(&path);
    let mut repository = RedbCatalogRepository::open(&path).expect("open");
    repository
        .apply_organization_command(&CatalogCommand::SetRating {
            photo_ids: vec![photo(1)],
            rating: Rating::Four,
        })
        .unwrap();
    repository
        .apply_organization_command(&CatalogCommand::SetRejection {
            photo_ids: vec![photo(1)],
            rejected: true,
        })
        .unwrap();
    repository
        .apply_organization_command(&CatalogCommand::SetRating {
            photo_ids: vec![photo(1)],
            rating: Rating::Two,
        })
        .unwrap();
    let state = repository.organization_states().unwrap().remove(&photo(1));
    let state = state.unwrap();
    assert_eq!(state.rating, Rating::Two);
    assert!(!state.rejected);
    support::remove(&path);
}

#[test]
fn failed_batch_rolls_back_every_photo_and_does_not_emit_an_event() {
    let path = support::temp_path("organization-rollback");
    seed(&path);
    let events = Arc::new(Mutex::new(Vec::<CatalogChangeEvent>::new()));
    let received = Arc::clone(&events);
    let mut repository = RedbCatalogRepository::open_with_before_commit_hook(&path, || {
        Err(AtomicCatalogStoreError::CommitFailed)
    })
    .unwrap();
    repository.set_change_listener(move |event| {
        received.lock().unwrap().push(event.clone());
    });
    assert_eq!(
        repository.apply_organization_command(&CatalogCommand::SetRating {
            photo_ids: vec![photo(1), photo(2)],
            rating: Rating::Three,
        }),
        Err(AtomicCatalogStoreError::CommitFailed)
    );
    assert!(events.lock().unwrap().is_empty());
    drop(repository);

    let repository = RedbCatalogRepository::open(&path).unwrap();
    assert!(
        repository
            .organization_states()
            .unwrap()
            .values()
            .all(|state| state.rating == Rating::Zero && !state.rejected)
    );
    support::remove(&path);
}

fn photo(value: u128) -> PhotoId {
    PhotoId::new(value).unwrap()
}
