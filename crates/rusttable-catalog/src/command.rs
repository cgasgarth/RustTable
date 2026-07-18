use rusttable_core::{Edit, EditId, Photo, Revision};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogCommand {
    RegisterPhoto(Photo),
    CreateEdit(Edit),
    ReplaceEdit {
        edit_id: EditId,
        expected_edit_revision: Revision,
        replacement: Edit,
    },
}
