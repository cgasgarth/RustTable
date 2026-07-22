use rusttable_core::{PhotoId, Revision};

use super::{TagCommand, TagError, TagId, TagIndexStats, TagMutationReceipt, TagState};

pub trait TagRepository {
    /// Loads canonical tag state and rebuilds its in-memory indexes.
    ///
    /// # Errors
    /// Returns a storage or persisted-data validation error.
    fn load(&self) -> Result<TagState, TagError>;

    /// Applies one conflict-safe command in a single durable transaction.
    ///
    /// # Errors
    /// Returns a validation, optimistic revision, storage, or commit error.
    fn apply(
        &mut self,
        expected: Revision,
        command: TagCommand,
    ) -> Result<TagMutationReceipt, TagError>;

    /// Resolves one canonical hierarchy path or alias through the durable index.
    ///
    /// # Errors
    /// Returns a validation, storage, or corruption error.
    fn resolve(&self, path_or_alias: &str) -> Result<Option<TagId>, TagError>;

    /// Returns direct tags for one photo in canonical hierarchy order.
    ///
    /// # Errors
    /// Returns a storage or corruption error.
    fn tags_for_photo(&self, photo_id: PhotoId) -> Result<Vec<TagId>, TagError>;

    /// Returns photos carrying a tag directly or through its hierarchy subtree.
    ///
    /// # Errors
    /// Returns an unknown-tag, storage, or corruption error.
    fn photos_with_tag(
        &self,
        tag_id: TagId,
        include_descendants: bool,
    ) -> Result<Vec<PhotoId>, TagError>;

    /// Replaces every derived durable index from canonical tag state.
    ///
    /// # Errors
    /// Returns a storage, corruption, or commit error.
    fn rebuild_indexes(&mut self) -> Result<TagIndexStats, TagError>;
}
