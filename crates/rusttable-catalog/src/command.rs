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
