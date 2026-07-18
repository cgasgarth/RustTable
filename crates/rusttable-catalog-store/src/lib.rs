#![forbid(unsafe_code)]
#![doc = "Transactional embedded import-record persistence for `RustTable`."]

mod codec;
mod repository;
mod schema;

pub use repository::RedbImportRepository;
pub use schema::CURRENT_SCHEMA_VERSION;
