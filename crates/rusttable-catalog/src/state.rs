use std::collections::{BTreeMap, BTreeSet};

use rusttable_core::{AssetId, Edit, EditId, Photo, PhotoId, Revision};

use crate::{
    CatalogCommand, CatalogError, CatalogQuery, ColorLabel, OrganizationProjection,
    PhotoOrganizationState, Rating,
};

/// Current catalog aggregates and their derived optimistic revision.
///
/// Every command that creates or advances an aggregate advances the catalog
/// exactly once. Restoration derives the same accounting from current values;
/// any future command that changes that accounting must update restoration
/// validation in the same change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogState {
    revision: Revision,
    photos: BTreeMap<PhotoId, Photo>,
    edits: BTreeMap<EditId, Edit>,
    asset_owners: BTreeMap<AssetId, PhotoId>,
    organization: BTreeMap<PhotoId, PhotoOrganizationState>,
    rating_index: BTreeMap<Rating, BTreeSet<PhotoId>>,
    rejected_index: BTreeSet<PhotoId>,
    color_label_index: BTreeMap<ColorLabel, BTreeSet<PhotoId>>,
}

impl CatalogState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            revision: Revision::ZERO,
            photos: BTreeMap::new(),
            edits: BTreeMap::new(),
            asset_owners: BTreeMap::new(),
            organization: BTreeMap::new(),
            rating_index: BTreeMap::new(),
            rejected_index: BTreeSet::new(),
            color_label_index: BTreeMap::new(),
        }
    }

    /// Applies one validated catalog command at an optimistic revision.
    ///
    /// # Errors
    ///
    /// Returns a typed validation or optimistic-concurrency error without partial mutation.
    pub fn apply(
        &mut self,
        expected: Revision,
        command: CatalogCommand,
    ) -> Result<Revision, CatalogError> {
        if expected != self.revision {
            return Err(CatalogError::CatalogRevisionConflict {
                expected,
                actual: self.revision,
            });
        }
        match command {
            CatalogCommand::RegisterPhoto(photo) => self.register_photo(photo),
            CatalogCommand::CreateEdit(edit) => self.create_edit(edit),
            CatalogCommand::ReplaceEdit {
                edit_id,
                expected_edit_revision,
                replacement,
            } => self.replace_edit(edit_id, expected_edit_revision, replacement),
            CatalogCommand::SetRating { photo_ids, rating } => self.set_rating(&photo_ids, rating),
            CatalogCommand::SetRejection {
                photo_ids,
                rejected,
            } => self.set_rejection(&photo_ids, rejected),
            CatalogCommand::SetColorLabel {
                photo_ids,
                label,
                enabled,
            } => self.set_color_label(&photo_ids, label, enabled),
            CatalogCommand::ToggleColorLabel { photo_ids, label } => {
                self.toggle_color_label(&photo_ids, label)
            }
        }
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn photo(&self, id: PhotoId) -> Option<&Photo> {
        self.photos.get(&id)
    }

    pub fn photos(&self) -> impl Iterator<Item = &Photo> {
        self.photos.values()
    }

    #[must_use]
    pub fn edit(&self, id: EditId) -> Option<&Edit> {
        self.edits.get(&id)
    }

    pub fn edits(&self) -> impl Iterator<Item = &Edit> {
        self.edits.values()
    }

    #[must_use]
    pub fn organization(&self, photo_id: PhotoId) -> Option<&PhotoOrganizationState> {
        self.organization.get(&photo_id)
    }

    /// Returns deterministic projections matching every supplied filter.
    #[must_use]
    pub fn query(&self, query: CatalogQuery) -> Vec<OrganizationProjection> {
        self.organization
            .values()
            .filter(|state| query.rating.is_none_or(|rating| state.rating == rating))
            .filter(|state| {
                query
                    .rejected
                    .is_none_or(|rejected| state.rejected == rejected)
            })
            .filter(|state| {
                query
                    .color_label
                    .is_none_or(|label| state.color_labels.contains(&label))
            })
            .map(|state| OrganizationProjection {
                photo_id: state.photo_id,
                rating: state.rating,
                rejected: state.rejected,
                color_labels: state.color_labels.iter().copied().collect(),
            })
            .collect()
    }

    pub fn photos_with_rating(&self, rating: Rating) -> impl Iterator<Item = PhotoId> + '_ {
        self.rating_index
            .get(&rating)
            .into_iter()
            .flatten()
            .copied()
    }

    pub fn rejected_photos(&self) -> impl Iterator<Item = PhotoId> + '_ {
        self.rejected_index.iter().copied()
    }

    pub fn photos_with_color_label(&self, label: ColorLabel) -> impl Iterator<Item = PhotoId> + '_ {
        self.color_label_index
            .get(&label)
            .into_iter()
            .flatten()
            .copied()
    }

    #[must_use]
    pub fn asset_owner(&self, asset_id: AssetId) -> Option<PhotoId> {
        self.asset_owners.get(&asset_id).copied()
    }

    pub(crate) fn from_parts(
        revision: Revision,
        photos: BTreeMap<PhotoId, Photo>,
        edits: BTreeMap<EditId, Edit>,
        asset_owners: BTreeMap<AssetId, PhotoId>,
    ) -> Self {
        let mut state = Self {
            revision,
            photos,
            edits,
            asset_owners,
            organization: BTreeMap::new(),
            rating_index: BTreeMap::new(),
            rejected_index: BTreeSet::new(),
            color_label_index: BTreeMap::new(),
        };
        let photo_ids = state.photos.keys().copied().collect::<Vec<_>>();
        for photo_id in photo_ids {
            state.insert_organization(PhotoOrganizationState::new(photo_id));
        }
        state
    }

    fn register_photo(&mut self, photo: Photo) -> Result<Revision, CatalogError> {
        let photo_id = photo.id();
        if self.photos.contains_key(&photo_id) {
            return Err(CatalogError::DuplicatePhoto { photo_id });
        }
        if photo.revision() != Revision::ZERO {
            return Err(CatalogError::InvalidInitialPhotoRevision {
                photo_id,
                revision: photo.revision(),
            });
        }
        for asset in photo.assets() {
            if let Some(existing_photo_id) = self.asset_owners.get(&asset.id()).copied() {
                return Err(CatalogError::AssetIdConflict {
                    asset_id: asset.id(),
                    existing_photo_id,
                    conflicting_photo_id: photo_id,
                });
            }
        }
        let next_revision = next_revision(self.revision)?;
        for asset in photo.assets() {
            self.asset_owners.insert(asset.id(), photo_id);
        }
        self.photos.insert(photo_id, photo);
        self.insert_organization(PhotoOrganizationState::new(photo_id));
        self.revision = next_revision;
        Ok(next_revision)
    }

    fn create_edit(&mut self, edit: Edit) -> Result<Revision, CatalogError> {
        let edit_id = edit.id();
        if self.edits.contains_key(&edit_id) {
            return Err(CatalogError::DuplicateEdit { edit_id });
        }
        let photo_id = edit.photo_id();
        let photo = self
            .photos
            .get(&photo_id)
            .ok_or(CatalogError::UnknownPhoto { photo_id })?;
        if edit.revision() != Revision::ZERO {
            return Err(CatalogError::InvalidInitialEditRevision {
                edit_id,
                revision: edit.revision(),
            });
        }
        if edit.base_photo_revision() != photo.revision() {
            return Err(CatalogError::EditBasePhotoRevisionConflict {
                edit_id,
                photo_id,
                expected: photo.revision(),
                actual: edit.base_photo_revision(),
            });
        }
        let next_revision = next_revision(self.revision)?;
        self.edits.insert(edit_id, edit);
        self.revision = next_revision;
        Ok(next_revision)
    }

    fn replace_edit(
        &mut self,
        edit_id: EditId,
        expected_edit_revision: Revision,
        replacement: Edit,
    ) -> Result<Revision, CatalogError> {
        let existing = self
            .edits
            .get(&edit_id)
            .ok_or(CatalogError::UnknownEdit { edit_id })?;
        if replacement.id() != edit_id {
            return Err(CatalogError::EditIdMismatch {
                target_edit_id: edit_id,
                replacement_edit_id: replacement.id(),
            });
        }
        if replacement.photo_id() != existing.photo_id() {
            return Err(CatalogError::EditPhotoMismatch {
                edit_id,
                expected_photo_id: existing.photo_id(),
                actual_photo_id: replacement.photo_id(),
            });
        }
        if replacement.base_photo_revision() != existing.base_photo_revision() {
            return Err(CatalogError::EditBasePhotoRevisionMismatch {
                edit_id,
                expected: existing.base_photo_revision(),
                actual: replacement.base_photo_revision(),
            });
        }
        if expected_edit_revision != existing.revision() {
            return Err(CatalogError::EditRevisionConflict {
                edit_id,
                expected: expected_edit_revision,
                actual: existing.revision(),
            });
        }
        let next_edit_revision = existing
            .revision()
            .checked_increment()
            .map_err(|_| CatalogError::EditRevisionOverflow { edit_id })?;
        if replacement.revision() != next_edit_revision {
            return Err(CatalogError::InvalidEditRevisionAdvance {
                edit_id,
                expected: next_edit_revision,
                actual: replacement.revision(),
            });
        }
        let next_catalog_revision = next_revision(self.revision)?;
        self.edits.insert(edit_id, replacement);
        self.revision = next_catalog_revision;
        Ok(next_catalog_revision)
    }

    fn set_rating(
        &mut self,
        photo_ids: &[PhotoId],
        rating: Rating,
    ) -> Result<Revision, CatalogError> {
        let photo_ids = self.validate_organization_batch(photo_ids)?;
        for photo_id in photo_ids {
            let old_rating = self
                .organization
                .get(&photo_id)
                .expect("photo organization exists")
                .rating;
            self.rating_index
                .get_mut(&old_rating)
                .expect("old rating index exists")
                .remove(&photo_id);
            self.organization
                .get_mut(&photo_id)
                .expect("photo organization exists")
                .rating = rating;
            self.rating_index
                .entry(rating)
                .or_default()
                .insert(photo_id);
        }
        self.advance_revision()
    }

    fn set_rejection(
        &mut self,
        photo_ids: &[PhotoId],
        rejected: bool,
    ) -> Result<Revision, CatalogError> {
        let photo_ids = self.validate_organization_batch(photo_ids)?;
        for photo_id in photo_ids {
            self.organization
                .get_mut(&photo_id)
                .expect("photo organization exists")
                .rejected = rejected;
            if rejected {
                self.rejected_index.insert(photo_id);
            } else {
                self.rejected_index.remove(&photo_id);
            }
        }
        self.advance_revision()
    }

    fn set_color_label(
        &mut self,
        photo_ids: &[PhotoId],
        label: ColorLabel,
        enabled: bool,
    ) -> Result<Revision, CatalogError> {
        let photo_ids = self.validate_organization_batch(photo_ids)?;
        for photo_id in photo_ids {
            let state = self
                .organization
                .get_mut(&photo_id)
                .expect("photo organization exists");
            if enabled {
                if state.color_labels.insert(label) {
                    self.color_label_index
                        .entry(label)
                        .or_default()
                        .insert(photo_id);
                }
            } else if state.color_labels.remove(&label) {
                self.color_label_index
                    .get_mut(&label)
                    .expect("label index exists")
                    .remove(&photo_id);
            }
        }
        self.advance_revision()
    }

    fn toggle_color_label(
        &mut self,
        photo_ids: &[PhotoId],
        label: ColorLabel,
    ) -> Result<Revision, CatalogError> {
        let photo_ids = self.validate_organization_batch(photo_ids)?;
        for photo_id in photo_ids {
            let enabled = !self
                .organization
                .get(&photo_id)
                .expect("photo organization exists")
                .color_labels
                .contains(&label);
            let state = self
                .organization
                .get_mut(&photo_id)
                .expect("photo organization exists");
            if enabled {
                state.color_labels.insert(label);
                self.color_label_index
                    .entry(label)
                    .or_default()
                    .insert(photo_id);
            } else {
                state.color_labels.remove(&label);
                self.color_label_index
                    .get_mut(&label)
                    .expect("label index exists")
                    .remove(&photo_id);
            }
        }
        self.advance_revision()
    }

    fn validate_organization_batch(
        &self,
        photo_ids: &[PhotoId],
    ) -> Result<Vec<PhotoId>, CatalogError> {
        if photo_ids.is_empty() {
            return Err(CatalogError::EmptyOrganizationBatch);
        }
        let mut unique = BTreeSet::new();
        for photo_id in photo_ids {
            if !unique.insert(*photo_id) {
                return Err(CatalogError::DuplicatePhotoInOrganizationBatch {
                    photo_id: *photo_id,
                });
            }
            if !self.photos.contains_key(photo_id) {
                return Err(CatalogError::UnknownPhoto {
                    photo_id: *photo_id,
                });
            }
        }
        Ok(unique.into_iter().collect())
    }

    fn insert_organization(&mut self, state: PhotoOrganizationState) {
        self.rating_index
            .entry(state.rating)
            .or_default()
            .insert(state.photo_id);
        self.organization.insert(state.photo_id, state);
    }

    fn advance_revision(&mut self) -> Result<Revision, CatalogError> {
        let next_revision = next_revision(self.revision)?;
        self.revision = next_revision;
        Ok(next_revision)
    }
}

impl Default for CatalogState {
    fn default() -> Self {
        Self::new()
    }
}

fn next_revision(revision: Revision) -> Result<Revision, CatalogError> {
    revision
        .checked_increment()
        .map_err(|_| CatalogError::CatalogRevisionOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusttable_core::{
        Asset, AssetId, AssetRole, ByteLength, ContentHash, Edit, EditId, Operation, OperationId,
        OperationKey,
    };

    fn photo() -> Photo {
        Photo::from_parts(
            PhotoId::new(1).expect("nonzero photo ID"),
            Revision::ZERO,
            [Asset::new(
                AssetId::new(1).expect("nonzero asset ID"),
                AssetRole::Primary,
                ContentHash::Sha256([0; 32]),
                ByteLength::ZERO,
            )],
        )
        .expect("valid photo")
    }

    fn edit(revision: Revision) -> Edit {
        Edit::from_parts(
            EditId::new(2).expect("nonzero edit ID"),
            PhotoId::new(1).expect("nonzero photo ID"),
            Revision::ZERO,
            revision,
            [Operation::new(
                OperationId::new(2).expect("nonzero operation ID"),
                OperationKey::new("rusttable.exposure").expect("valid key"),
                true,
                [],
            )
            .expect("valid operation")],
        )
        .expect("valid edit")
    }

    #[test]
    fn catalog_revision_overflow_is_atomic() {
        let mut state = CatalogState {
            revision: Revision::from_u64(u64::MAX),
            photos: BTreeMap::new(),
            edits: BTreeMap::new(),
            asset_owners: BTreeMap::new(),
            organization: BTreeMap::new(),
            rating_index: BTreeMap::new(),
            rejected_index: BTreeSet::new(),
            color_label_index: BTreeMap::new(),
        };
        let before = state.clone();

        let error = state
            .apply(
                Revision::from_u64(u64::MAX),
                CatalogCommand::RegisterPhoto(photo()),
            )
            .expect_err("maximum catalog revision cannot increment");

        assert_eq!(error, CatalogError::CatalogRevisionOverflow);
        assert_eq!(state, before);
    }

    #[test]
    fn edit_revision_overflow_is_atomic() {
        let mut state = CatalogState {
            revision: Revision::from_u64(2),
            photos: BTreeMap::from([(PhotoId::new(1).unwrap(), photo())]),
            edits: BTreeMap::from([(EditId::new(2).unwrap(), edit(Revision::from_u64(u64::MAX)))]),
            asset_owners: BTreeMap::from([(AssetId::new(1).unwrap(), PhotoId::new(1).unwrap())]),
            organization: BTreeMap::from([(
                PhotoId::new(1).unwrap(),
                PhotoOrganizationState::new(PhotoId::new(1).unwrap()),
            )]),
            rating_index: BTreeMap::from([(
                Rating::Zero,
                BTreeSet::from([PhotoId::new(1).unwrap()]),
            )]),
            rejected_index: BTreeSet::new(),
            color_label_index: BTreeMap::new(),
        };
        let before = state.clone();

        let error = state
            .apply(
                Revision::from_u64(2),
                CatalogCommand::ReplaceEdit {
                    edit_id: EditId::new(2).unwrap(),
                    expected_edit_revision: Revision::from_u64(u64::MAX),
                    replacement: edit(Revision::from_u64(u64::MAX)),
                },
            )
            .expect_err("maximum edit revision cannot increment");

        assert_eq!(
            error,
            CatalogError::EditRevisionOverflow {
                edit_id: EditId::new(2).unwrap()
            }
        );
        assert_eq!(state, before);
    }
}
