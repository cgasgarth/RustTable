#![forbid(unsafe_code)]
#![doc = "Transactional embedded import-record persistence for `RustTable`."]

mod codec;
mod edit_codec;
mod edit_repository;
mod repository;
mod schema;

pub use edit_repository::RedbEditRepository;
pub use repository::RedbImportRepository;
pub use schema::CURRENT_SCHEMA_VERSION;
