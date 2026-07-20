#![forbid(unsafe_code)]
#![doc = "Transactional embedded import-record persistence for `RustTable`."]

mod catalog_repository;
mod codec;
mod edit_codec;
mod edit_repository;
mod repository;
mod schema;

pub use catalog_repository::{AtomicCatalogStoreError, RedbCatalogRepository};
pub use edit_repository::RedbEditRepository;
pub use repository::RedbImportRepository;
pub use schema::CURRENT_SCHEMA_VERSION;
