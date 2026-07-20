use std::path::Path;
use std::sync::Arc;

use rusttable_catalog::{
    EditRepository, EditRepositoryError, ImportRecord, ImportRepository, RepositoryError,
    SourcePath,
};
use rusttable_core::{AssetId, Edit, EditId, PhotoId, Revision};

use crate::edit_repository::RedbEditRepository;
use crate::repository::RedbImportRepository;
use crate::schema;

/// Shared redb catalog adapter for application compositions that need imports and edits.
pub struct RedbCatalogRepository {
    imports: RedbImportRepository,
    edits: RedbEditRepository,
}

impl RedbCatalogRepository {
    /// Opens one schema-versioned database handle for import and edit access.
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure when the catalog cannot be opened or validated.
    pub fn open(path: &Path) -> Result<Self, RepositoryError> {
        let database = Arc::new(schema::open(path)?);
        Ok(Self {
            imports: RedbImportRepository::from_database(Arc::clone(&database)),
            edits: RedbEditRepository::from_database(database),
        })
    }
}

impl ImportRepository for RedbCatalogRepository {
    fn find_by_source(&self, source: &SourcePath) -> Result<Option<ImportRecord>, RepositoryError> {
        self.imports.find_by_source(source)
    }

    fn find_by_photo_id(&self, photo_id: PhotoId) -> Result<Option<ImportRecord>, RepositoryError> {
        self.imports.find_by_photo_id(photo_id)
    }

    fn find_by_asset_id(&self, asset_id: AssetId) -> Result<Option<ImportRecord>, RepositoryError> {
        self.imports.find_by_asset_id(asset_id)
    }

    fn commit(&mut self, record: &ImportRecord) -> Result<(), RepositoryError> {
        self.imports.commit(record)
    }

    fn list(&self) -> Result<Vec<ImportRecord>, RepositoryError> {
        self.imports.list()
    }
}

impl EditRepository for RedbCatalogRepository {
    fn find_by_edit_id(&self, edit_id: EditId) -> Result<Option<Edit>, EditRepositoryError> {
        self.edits.find_by_edit_id(edit_id)
    }

    fn list(&self) -> Result<Vec<Edit>, EditRepositoryError> {
        self.edits.list()
    }

    fn commit_new(&mut self, edit: &Edit) -> Result<(), EditRepositoryError> {
        self.edits.commit_new(edit)
    }

    fn commit_replacement(
        &mut self,
        expected_edit_revision: Revision,
        edit: &Edit,
    ) -> Result<(), EditRepositoryError> {
        self.edits.commit_replacement(expected_edit_revision, edit)
    }
}
