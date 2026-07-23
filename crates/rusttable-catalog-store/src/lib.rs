#![forbid(unsafe_code)]
#![doc = "Transactional embedded import-record persistence for `RustTable`."]

mod codecs;
mod repositories;
mod schema;

use codecs as codec;
use codecs::organization as organization_codec;
use codecs::photo_groups as photo_group_codec;
use codecs::{
    duplicates as duplicate_codec, edit as edit_codec, history as history_codec,
    import_details as import_details_codec,
};

pub use repositories::RedbImportRepository;
pub use repositories::catalog::{
    AtomicCatalogStoreError, RedbCatalogRepository, SourceReconciliationReport,
};
pub use repositories::collection::RedbCollectionRepository;
pub use repositories::edit::RedbEditRepository;
pub use repositories::history::RedbHistoryRepository;
pub use repositories::metadata::RedbCatalogMetadataRepository;
pub use repositories::recipe::{RecipeStoreError, RedbRecipeRepository};
pub use repositories::tags::RedbTagRepository;
pub use rusttable_export::{
    ExportJobId, ExportJobPriority, ExportJobRecord, ExportJobStage, ExportJobState,
    ExportQueueError, RedbExportQueueStore, queue_now_millis,
};
pub use schema::CURRENT_SCHEMA_VERSION;
