use rusttable_core::{Edit, EditId, Photo, PhotoId, Revision};

use crate::{ColorLabel, Rating};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogCommand {
    RegisterPhoto(Photo),
    CreateEdit(Edit),
    ReplaceEdit {
        edit_id: EditId,
        expected_edit_revision: Revision,
        replacement: Edit,
    },
    SetRating {
        photo_ids: Vec<PhotoId>,
        rating: Rating,
    },
    SetRejection {
        photo_ids: Vec<PhotoId>,
        rejected: bool,
    },
    SetColorLabel {
        photo_ids: Vec<PhotoId>,
        label: ColorLabel,
        enabled: bool,
    },
    ToggleColorLabel {
        photo_ids: Vec<PhotoId>,
        label: ColorLabel,
    },
}

/// A committed catalog change that consumers can use to refresh projections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogChangeEvent {
    revision: Revision,
    photo_ids: Vec<PhotoId>,
}

impl CatalogChangeEvent {
    #[must_use]
    pub fn new(revision: Revision, photo_ids: impl IntoIterator<Item = PhotoId>) -> Self {
        Self {
            revision,
            photo_ids: photo_ids.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn photo_ids(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.photo_ids.iter().copied()
    }
}
