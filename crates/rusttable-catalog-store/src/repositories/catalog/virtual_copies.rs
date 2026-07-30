use rusttable_catalog::{
    VirtualCopyCommand, VirtualCopyProjection, VirtualCopyRepository, VirtualCopyRepositoryError,
};
use rusttable_core::Revision;

use super::RedbCatalogRepository;

impl RedbCatalogRepository {
    /// Returns the virtual-copy repository backed by this same catalog file.
    #[must_use]
    pub const fn virtual_copies(&self) -> &super::RedbVirtualCopyRepository {
        &self.virtual_copies
    }

    /// Applies one virtual-copy command without creating a second catalog backend.
    ///
    /// # Errors
    ///
    /// Returns a source-anchor, validation, persistence, or optimistic-concurrency error.
    pub fn apply_virtual_copy_command(
        &mut self,
        expected: Revision,
        command: VirtualCopyCommand,
    ) -> Result<Revision, VirtualCopyRepositoryError> {
        self.virtual_copies.apply(expected, command)
    }

    /// Projects active virtual copies in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns a persistence or corruption error.
    pub fn virtual_copy_projections(
        &self,
    ) -> Result<Vec<VirtualCopyProjection>, VirtualCopyRepositoryError> {
        self.virtual_copies.projections()
    }
}
