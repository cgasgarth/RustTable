use std::fmt;

use rusttable_core::{Asset, Edit, EditId, ImageMetadata, Photo, PhotoId, Revision};
use rusttable_image::ImageProbe;

use crate::{CatalogSnapshot, ImportRecord, SourcePath};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DevelopSelection {
    catalog_revision: Revision,
    photo_id: PhotoId,
    photo_revision: Revision,
    edit_id: EditId,
    edit_revision: Revision,
}

impl DevelopSelection {
    #[must_use]
    pub const fn new(
        catalog_revision: Revision,
        photo_id: PhotoId,
        photo_revision: Revision,
        edit_id: EditId,
        edit_revision: Revision,
    ) -> Self {
        Self {
            catalog_revision,
            photo_id,
            photo_revision,
            edit_id,
            edit_revision,
        }
    }

    #[must_use]
    pub const fn catalog_revision(self) -> Revision {
        self.catalog_revision
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn photo_revision(self) -> Revision {
        self.photo_revision
    }

    #[must_use]
    pub const fn edit_id(self) -> EditId {
        self.edit_id
    }

    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopInput {
    catalog_revision: Revision,
    record: ImportRecord,
    edit: Edit,
}

impl DevelopInput {
    #[must_use]
    pub const fn catalog_revision(&self) -> Revision {
        self.catalog_revision
    }

    #[must_use]
    pub fn source(&self) -> &SourcePath {
        self.record.source()
    }

    #[must_use]
    pub fn photo(&self) -> &Photo {
        self.record.photo()
    }

    #[must_use]
    pub fn primary_asset(&self) -> &Asset {
        self.record.photo().primary_asset()
    }

    #[must_use]
    pub const fn probe(&self) -> ImageProbe {
        self.record.probe()
    }

    #[must_use]
    pub fn metadata(&self) -> &ImageMetadata {
        self.record.metadata()
    }

    #[must_use]
    pub const fn edit(&self) -> &Edit {
        &self.edit
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevelopInputError {
    CatalogRevisionConflict {
        expected: Revision,
        actual: Revision,
    },
    UnknownPhoto {
        photo_id: PhotoId,
    },
    UnknownEdit {
        edit_id: EditId,
    },
    EditPhotoMismatch {
        edit_id: EditId,
        expected_photo_id: PhotoId,
        actual_photo_id: PhotoId,
    },
    PhotoRevisionConflict {
        photo_id: PhotoId,
        expected: Revision,
        actual: Revision,
    },
    EditRevisionConflict {
        edit_id: EditId,
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for DevelopInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CatalogRevisionConflict { expected, actual } => write!(
                formatter,
                "develop catalog revision conflict: expected {expected}, actual {actual}"
            ),
            Self::UnknownPhoto { photo_id } => {
                write!(formatter, "develop photo {photo_id} is unknown")
            }
            Self::UnknownEdit { edit_id } => write!(formatter, "develop edit {edit_id} is unknown"),
            Self::EditPhotoMismatch {
                edit_id,
                expected_photo_id,
                actual_photo_id,
            } => write!(
                formatter,
                "develop edit {edit_id} targets photo {actual_photo_id}, expected {expected_photo_id}"
            ),
            Self::PhotoRevisionConflict {
                photo_id,
                expected,
                actual,
            } => write!(
                formatter,
                "develop photo {photo_id} revision conflict: expected {expected}, actual {actual}"
            ),
            Self::EditRevisionConflict {
                edit_id,
                expected,
                actual,
            } => write!(
                formatter,
                "develop edit {edit_id} revision conflict: expected {expected}, actual {actual}"
            ),
        }
    }
}

impl std::error::Error for DevelopInputError {}

impl CatalogSnapshot {
    /// Resolves an explicit photo/edit pair against one immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns a distinct typed error for each revision, identity, or ownership
    /// mismatch in the documented validation order.
    pub fn resolve_develop(
        &self,
        selection: DevelopSelection,
    ) -> Result<DevelopInput, DevelopInputError> {
        if selection.catalog_revision != self.revision() {
            return Err(DevelopInputError::CatalogRevisionConflict {
                expected: selection.catalog_revision,
                actual: self.revision(),
            });
        }
        let entry =
            self.by_photo_id(selection.photo_id)
                .ok_or(DevelopInputError::UnknownPhoto {
                    photo_id: selection.photo_id,
                })?;
        let edit = self
            .edit_by_id(selection.edit_id)
            .ok_or(DevelopInputError::UnknownEdit {
                edit_id: selection.edit_id,
            })?;
        if edit.photo_id() != selection.photo_id {
            return Err(DevelopInputError::EditPhotoMismatch {
                edit_id: selection.edit_id,
                expected_photo_id: selection.photo_id,
                actual_photo_id: edit.photo_id(),
            });
        }
        if entry.photo().revision() != selection.photo_revision {
            return Err(DevelopInputError::PhotoRevisionConflict {
                photo_id: selection.photo_id,
                expected: selection.photo_revision,
                actual: entry.photo().revision(),
            });
        }
        if edit.revision() != selection.edit_revision {
            return Err(DevelopInputError::EditRevisionConflict {
                edit_id: selection.edit_id,
                expected: selection.edit_revision,
                actual: edit.revision(),
            });
        }
        Ok(DevelopInput {
            catalog_revision: self.revision(),
            record: entry.clone_record(),
            edit: edit.clone(),
        })
    }
}
