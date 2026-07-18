use std::collections::BTreeMap;

use rusttable_core::{Edit, EditId, Photo, PhotoId, Revision};

use crate::{CatalogCommand, CatalogError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogState {
    revision: Revision,
    photos: BTreeMap<PhotoId, Photo>,
    edits: BTreeMap<EditId, Edit>,
}

impl CatalogState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            revision: Revision::ZERO,
            photos: BTreeMap::new(),
            edits: BTreeMap::new(),
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

    fn register_photo(&mut self, photo: Photo) -> Result<Revision, CatalogError> {
        let photo_id = photo.id();
        if self.photos.contains_key(&photo_id) {
            return Err(CatalogError::DuplicatePhoto { photo_id });
        }
        let next_revision = next_revision(self.revision)?;
        self.photos.insert(photo_id, photo);
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
