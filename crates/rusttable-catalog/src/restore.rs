use std::collections::BTreeMap;
use std::fmt;

use rusttable_core::{AssetId, Edit, EditId, Photo, PhotoId, Revision};

use crate::CatalogState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogRestoreError {
    DuplicatePhoto {
        photo_id: PhotoId,
    },
    AssetIdConflict {
        asset_id: AssetId,
        existing_photo_id: PhotoId,
        conflicting_photo_id: PhotoId,
    },
    DuplicateEdit {
        edit_id: EditId,
    },
    UnknownPhoto {
        edit_id: EditId,
        photo_id: PhotoId,
    },
    EditBasePhotoRevisionConflict {
        edit_id: EditId,
        photo_id: PhotoId,
        expected: Revision,
        actual: Revision,
    },
    RevisionOverflow {
        photo_id: Option<PhotoId>,
        edit_id: Option<EditId>,
        revision: Revision,
    },
}

impl CatalogState {
    /// Reconstructs complete current catalog state from owned aggregate values.
    ///
    /// Validation is performed before publishing any state. The catalog
    /// revision is derived as `sum(1 + aggregate revision)` with checked
    /// arithmetic, matching the accounting performed by catalog commands.
    ///
    /// # Errors
    ///
    /// Returns a typed identity, cross-aggregate, or checked-revision error.
    pub fn restore(
        photos: impl IntoIterator<Item = Photo>,
        edits: impl IntoIterator<Item = Edit>,
    ) -> Result<Self, CatalogRestoreError> {
        let photos = photos.into_iter().collect::<Vec<_>>();
        let photo_map = collect_photos(photos)?;
        let edits = edits.into_iter().collect::<Vec<_>>();
        let edit_map = collect_edits(edits)?;
        validate_edits(&photo_map, &edit_map)?;

        let mut revision = 0_u64;
        for photo in photo_map.values() {
            revision = add_revision(
                revision,
                photo.revision(),
                CatalogRestoreError::RevisionOverflow {
                    photo_id: Some(photo.id()),
                    edit_id: None,
                    revision: photo.revision(),
                },
            )?;
        }
        for edit in edit_map.values() {
            revision = add_revision(
                revision,
                edit.revision(),
                CatalogRestoreError::RevisionOverflow {
                    photo_id: None,
                    edit_id: Some(edit.id()),
                    revision: edit.revision(),
                },
            )?;
        }

        let asset_owners = collect_asset_owners(&photo_map);
        Ok(Self::from_parts(
            Revision::from_u64(revision),
            photo_map,
            edit_map,
            asset_owners,
        ))
    }
}

fn collect_photos(photos: Vec<Photo>) -> Result<BTreeMap<PhotoId, Photo>, CatalogRestoreError> {
    let mut photo_map = BTreeMap::new();
    for photo in photos {
        let photo_id = photo.id();
        if photo_map.contains_key(&photo_id) {
            return Err(CatalogRestoreError::DuplicatePhoto { photo_id });
        }
        photo_map.insert(photo_id, photo);
    }

    let mut asset_owners = BTreeMap::new();
    for photo in photo_map.values() {
        for asset in photo.assets() {
            if let Some(existing_photo_id) = asset_owners.insert(asset.id(), photo.id()) {
                return Err(CatalogRestoreError::AssetIdConflict {
                    asset_id: asset.id(),
                    existing_photo_id,
                    conflicting_photo_id: photo.id(),
                });
            }
        }
    }
    Ok(photo_map)
}

fn collect_edits(edits: Vec<Edit>) -> Result<BTreeMap<EditId, Edit>, CatalogRestoreError> {
    let mut edit_map = BTreeMap::new();
    for edit in edits {
        let edit_id = edit.id();
        if edit_map.contains_key(&edit_id) {
            return Err(CatalogRestoreError::DuplicateEdit { edit_id });
        }
        edit_map.insert(edit_id, edit);
    }
    Ok(edit_map)
}

fn validate_edits(
    photos: &BTreeMap<PhotoId, Photo>,
    edits: &BTreeMap<EditId, Edit>,
) -> Result<(), CatalogRestoreError> {
    for edit in edits.values() {
        let photo_id = edit.photo_id();
        if !photos.contains_key(&photo_id) {
            return Err(CatalogRestoreError::UnknownPhoto {
                edit_id: edit.id(),
                photo_id,
            });
        }
    }
    for edit in edits.values() {
        let photo_id = edit.photo_id();
        let photo = photos
            .get(&photo_id)
            .expect("unknown photo validation completed");
        if edit.base_photo_revision() != photo.revision() {
            return Err(CatalogRestoreError::EditBasePhotoRevisionConflict {
                edit_id: edit.id(),
                photo_id,
                expected: photo.revision(),
                actual: edit.base_photo_revision(),
            });
        }
    }
    Ok(())
}

fn collect_asset_owners(photos: &BTreeMap<PhotoId, Photo>) -> BTreeMap<AssetId, PhotoId> {
    photos
        .values()
        .flat_map(|photo| photo.assets().map(move |asset| (asset.id(), photo.id())))
        .collect()
}

fn add_revision(
    total: u64,
    aggregate_revision: Revision,
    overflow: CatalogRestoreError,
) -> Result<u64, CatalogRestoreError> {
    total
        .checked_add(
            aggregate_revision
                .get()
                .checked_add(1)
                .ok_or(overflow.clone())?,
        )
        .ok_or(overflow)
}

impl fmt::Display for CatalogRestoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePhoto { photo_id } => {
                write!(formatter, "photo ID {photo_id} occurs more than once")
            }
            Self::AssetIdConflict {
                asset_id,
                existing_photo_id,
                conflicting_photo_id,
            } => write!(
                formatter,
                "asset ID {asset_id} is owned by photos {existing_photo_id} and {conflicting_photo_id}"
            ),
            Self::DuplicateEdit { edit_id } => {
                write!(formatter, "edit ID {edit_id} occurs more than once")
            }
            Self::UnknownPhoto { edit_id, photo_id } => {
                write!(
                    formatter,
                    "edit {edit_id} references unknown photo {photo_id}"
                )
            }
            Self::EditBasePhotoRevisionConflict {
                edit_id,
                photo_id,
                expected,
                actual,
            } => write!(
                formatter,
                "edit {edit_id} for photo {photo_id} has base revision {actual}, expected {expected}"
            ),
            Self::RevisionOverflow {
                photo_id,
                edit_id,
                revision,
            } => write!(
                formatter,
                "revision accounting overflow for photo {photo_id:?}, edit {edit_id:?} at revision {revision}"
            ),
        }
    }
}

impl std::error::Error for CatalogRestoreError {}
