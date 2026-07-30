//! Application-owned bridge for persisted library-view state.

#![allow(clippy::missing_errors_doc)]

use std::path::Path;

use rusttable_catalog::{
    ActiveLighttableState, CollectionCommand, CollectionRepository, CollectionRepositoryError,
    CollectionState,
};
use rusttable_catalog_store::RedbCollectionRepository;

pub struct LibraryCollectionService {
    repository: RedbCollectionRepository,
    state: CollectionState,
    active_lighttable: ActiveLighttableState,
}

impl LibraryCollectionService {
    pub fn open(path: &Path) -> Result<Self, CollectionRepositoryError> {
        let repository = RedbCollectionRepository::open(path)?;
        let state = repository.load()?;
        let active_lighttable = repository.load_active_lighttable_state()?;
        Ok(Self {
            repository,
            state,
            active_lighttable,
        })
    }

    #[must_use]
    pub const fn state(&self) -> &CollectionState {
        &self.state
    }

    #[must_use]
    pub const fn active_lighttable(&self) -> &ActiveLighttableState {
        &self.active_lighttable
    }

    pub fn dispatch(
        &mut self,
        command: CollectionCommand,
    ) -> Result<(), CollectionRepositoryError> {
        self.state = self.repository.apply(command)?;
        Ok(())
    }

    pub fn persist_active_lighttable(
        &mut self,
        state: ActiveLighttableState,
    ) -> Result<(), CollectionRepositoryError> {
        self.repository.persist_active_lighttable_state(&state)?;
        self.active_lighttable = state;
        Ok(())
    }
}
