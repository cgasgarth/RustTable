use rusttable_core::{Edit, Photo};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogCommand {
    RegisterPhoto(Photo),
    CreateEdit(Edit),
}
