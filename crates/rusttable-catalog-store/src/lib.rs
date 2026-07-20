#![forbid(unsafe_code)]
#![doc = "Transactional embedded import-record persistence for `RustTable`."]

mod catalog_repository;
mod codec;
mod collection_repository;
mod edit_codec;
mod edit_repository;
mod import_details_codec;
mod recipe_repository;
mod repository;
mod schema;

pub use catalog_repository::{AtomicCatalogStoreError, RedbCatalogRepository};
pub use collection_repository::RedbCollectionRepository;
pub use edit_repository::RedbEditRepository;
pub use recipe_repository::{RecipeStoreError, RedbRecipeRepository};
pub use repository::RedbImportRepository;
pub use rusttable_export::{
    ExportJobId, ExportJobPriority, ExportJobRecord, ExportJobStage, ExportJobState,
    ExportQueueError, RedbExportQueueStore, queue_now_millis,
};
pub use schema::CURRENT_SCHEMA_VERSION;
