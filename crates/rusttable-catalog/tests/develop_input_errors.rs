#[path = "support/develop.rs"]
mod develop_support;

use develop_support::fixture;
use rusttable_catalog::{DevelopInputError, DevelopSelection};
use rusttable_core::{EditId, PhotoId, Revision};

fn selection(
    catalog_revision: Revision,
    photo_id: PhotoId,
    photo_revision: Revision,
    edit_id: EditId,
    edit_revision: Revision,
) -> DevelopSelection {
    DevelopSelection::new(
        catalog_revision,
        photo_id,
        photo_revision,
        edit_id,
        edit_revision,
    )
}

#[test]
fn validates_catalog_revision_before_any_other_fault() {
    let (snapshot, _, photo_id, edit_id, _, _, _) = fixture();
    let error = snapshot
        .resolve_develop(selection(
            Revision::ZERO,
            PhotoId::new(999).unwrap(),
            Revision::from_u64(99),
            EditId::new(999).unwrap(),
            Revision::from_u64(99),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        DevelopInputError::CatalogRevisionConflict { .. }
    ));
    let _ = (photo_id, edit_id);
}

#[test]
fn distinguishes_unknown_photo_and_unknown_edit() {
    let (snapshot, catalog_revision, _, _, _, _, _) = fixture();
    let photo_error = snapshot
        .resolve_develop(selection(
            catalog_revision,
            PhotoId::new(999).unwrap(),
            Revision::ZERO,
            EditId::new(2).unwrap(),
            Revision::ZERO,
        ))
        .unwrap_err();
    assert!(matches!(
        photo_error,
        DevelopInputError::UnknownPhoto { .. }
    ));

    let edit_error = snapshot
        .resolve_develop(selection(
            catalog_revision,
            PhotoId::new(1).unwrap(),
            Revision::ZERO,
            EditId::new(999).unwrap(),
            Revision::ZERO,
        ))
        .unwrap_err();
    assert!(matches!(edit_error, DevelopInputError::UnknownEdit { .. }));
}

#[test]
fn validates_edit_photo_before_revisions() {
    let (snapshot, catalog_revision, _, _, _, second_photo, second_edit) = fixture();
    let error = snapshot
        .resolve_develop(selection(
            catalog_revision,
            PhotoId::new(1).unwrap(),
            Revision::from_u64(99),
            second_edit,
            Revision::from_u64(99),
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        DevelopInputError::EditPhotoMismatch {
            expected_photo_id,
            actual_photo_id,
            ..
        } if expected_photo_id == PhotoId::new(1).unwrap() && actual_photo_id == second_photo
    ));
}

#[test]
fn distinguishes_stale_photo_and_edit_revisions() {
    let (snapshot, catalog_revision, photo_id, edit_id, _, _, _) = fixture();
    let photo_error = snapshot
        .resolve_develop(selection(
            catalog_revision,
            photo_id,
            Revision::from_u64(1),
            edit_id,
            Revision::from_u64(1),
        ))
        .unwrap_err();
    assert!(matches!(
        photo_error,
        DevelopInputError::PhotoRevisionConflict { .. }
    ));

    let edit_error = snapshot
        .resolve_develop(selection(
            catalog_revision,
            photo_id,
            Revision::ZERO,
            edit_id,
            Revision::from_u64(1),
        ))
        .unwrap_err();
    assert!(matches!(
        edit_error,
        DevelopInputError::EditRevisionConflict { .. }
    ));
}
