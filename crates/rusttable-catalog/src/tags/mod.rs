//! Hierarchical tag identities, canonical lookup, and atomic photo assignments.

mod error;
mod repository;
mod state;
mod types;

pub use error::TagError;
pub use repository::TagRepository;
pub use state::{
    TagCommand, TagIndexStats, TagMutationReceipt, TagProjection, TagSnapshot, TagState,
};
pub use types::{
    MAX_ASSIGNMENT_PHOTOS, MAX_ASSIGNMENT_TAGS, MAX_TAG_ALIASES, MAX_TAG_NAME_BYTES,
    MAX_TAGS_PER_PHOTO, TAG_HIERARCHY_SEPARATOR, TAG_SCHEMA_VERSION, TagAlias, TagDefinition,
    TagId, TagName,
};
