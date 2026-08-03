//! Application-owned bridge for persisted library-view state.

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
    /// Opens the persisted collection and active-lighttable state.
    ///
    /// # Errors
    ///
    /// Returns an error when the collection repository cannot be opened or loaded.
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

    /// Applies and persists one collection command.
    ///
    /// # Errors
    ///
    /// Returns an error when command validation, conflict detection, or persistence fails.
    pub fn dispatch(
        &mut self,
        command: CollectionCommand,
    ) -> Result<(), CollectionRepositoryError> {
        self.state = self.repository.apply(command)?;
        Ok(())
    }

    /// Persists and adopts the active-lighttable state.
    ///
    /// # Errors
    ///
    /// Returns an error when the state cannot be persisted.
    pub fn persist_active_lighttable(
        &mut self,
        state: ActiveLighttableState,
    ) -> Result<(), CollectionRepositoryError> {
        self.repository.persist_active_lighttable_state(&state)?;
        self.active_lighttable = state;
        Ok(())
    }
}
