#[path = "support/develop.rs"]
mod develop_support;

use develop_support::fixture;
use rusttable_catalog::{DevelopInput, DevelopSelection};
use rusttable_core::{EditId, PhotoId, Revision};

fn selection(catalog_revision: Revision, photo_id: PhotoId, edit_id: EditId) -> DevelopSelection {
    DevelopSelection::new(
        catalog_revision,
        photo_id,
        Revision::ZERO,
        edit_id,
        Revision::ZERO,
    )
}

#[test]
fn resolves_one_exact_revision_pinned_input() {
    let (snapshot, catalog_revision, photo_id, edit_id, _, _, _) = fixture();

    let input = snapshot
        .resolve_develop(selection(catalog_revision, photo_id, edit_id))
        .expect("selection resolves");

    assert_eq!(input.catalog_revision(), catalog_revision);
    assert_eq!(input.source().to_string(), "first.png");
    assert_eq!(input.photo().id(), photo_id);
    assert_eq!(input.primary_asset().id().get(), 11);
    assert_eq!(input.probe().dimensions().width(), 2);
    assert!(input.metadata().is_empty());
    assert_eq!(input.edit().id(), edit_id);
    assert_eq!(input.edit().base_photo_revision(), input.photo().revision());
}

#[test]
fn explicit_selection_can_choose_each_edit_without_a_current_policy() {
    let (snapshot, catalog_revision, photo_id, first_edit_id, second_edit_id, _, _) = fixture();

    let first = snapshot
        .resolve_develop(selection(catalog_revision, photo_id, first_edit_id))
        .expect("first edit resolves");
    let second = snapshot
        .resolve_develop(selection(catalog_revision, photo_id, second_edit_id))
        .expect("second edit resolves");

    assert_eq!(first.edit().id(), first_edit_id);
    assert_eq!(second.edit().id(), second_edit_id);
}

#[test]
fn input_owns_values_after_snapshot_and_fixtures_are_dropped() {
    let input: DevelopInput = {
        let (snapshot, catalog_revision, photo_id, edit_id, _, _, _) = fixture();
        snapshot
            .resolve_develop(selection(catalog_revision, photo_id, edit_id))
            .expect("selection resolves")
    };

    assert_eq!(input.photo().id(), PhotoId::new(1).unwrap());
    assert_eq!(input.edit().id(), EditId::new(2).unwrap());
}
