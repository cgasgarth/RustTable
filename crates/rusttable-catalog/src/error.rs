use std::fmt;

use rusttable_core::{EditId, PhotoId, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    CatalogRevisionConflict {
        expected: Revision,
        actual: Revision,
    },
    CatalogRevisionOverflow,
    DuplicatePhoto {
        photo_id: PhotoId,
    },
    UnknownPhoto {
        photo_id: PhotoId,
    },
    DuplicateEdit {
        edit_id: EditId,
    },
    InvalidInitialEditRevision {
        edit_id: EditId,
        revision: Revision,
    },
    EditBasePhotoRevisionConflict {
        edit_id: EditId,
        photo_id: PhotoId,
        expected: Revision,
        actual: Revision,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CatalogRevisionConflict { expected, actual } => {
                write!(
                    formatter,
                    "catalog revision conflict: expected {expected}, actual {actual}"
                )
            }
            Self::CatalogRevisionOverflow => {
                formatter.write_str("catalog revision cannot be incremented")
            }
            Self::DuplicatePhoto { photo_id } => {
                write!(formatter, "photo ID {photo_id} is already registered")
            }
            Self::UnknownPhoto { photo_id } => {
                write!(formatter, "photo ID {photo_id} is not registered")
            }
            Self::DuplicateEdit { edit_id } => {
                write!(formatter, "edit ID {edit_id} is already registered")
            }
            Self::InvalidInitialEditRevision { edit_id, revision } => {
                write!(
                    formatter,
                    "edit {edit_id} has initial revision {revision}, expected zero"
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
        }
    }
}

impl std::error::Error for CatalogError {}
