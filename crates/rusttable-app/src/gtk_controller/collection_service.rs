//! Application-owned bridge for persisted library-view state.

#![allow(clippy::missing_errors_doc)]

use std::path::Path;

use rusttable_catalog::{
    CollectionCommand, CollectionRepository, CollectionRepositoryError, CollectionState,
};
use rusttable_catalog_store::RedbCollectionRepository;

pub struct LibraryCollectionService {
    repository: RedbCollectionRepository,
    state: CollectionState,
}

impl LibraryCollectionService {
    pub fn open(path: &Path) -> Result<Self, CollectionRepositoryError> {
        let repository = RedbCollectionRepository::open(path)?;
        let state = repository.load()?;
        Ok(Self { repository, state })
    }

    #[must_use]
    pub const fn state(&self) -> &CollectionState {
        &self.state
    }

    pub fn dispatch(
        &mut self,
        command: CollectionCommand,
    ) -> Result<(), CollectionRepositoryError> {
        self.state = self.repository.apply(command)?;
        Ok(())
    }
}
