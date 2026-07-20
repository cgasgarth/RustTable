use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableTable};
use rusttable_catalog::{
    EditRepository, EditRepositoryError, ImportRecord, ImportRepository, RepositoryError,
    SourcePath,
};
use rusttable_core::{AssetId, ContentHash, Edit, EditId, PhotoId, Revision};

use crate::edit_repository::RedbEditRepository;
use crate::repository::RedbImportRepository;
use crate::schema;

/// Shared redb catalog adapter for application compositions that need imports and edits.
pub struct RedbCatalogRepository {
    database: Arc<Database>,
    imports: RedbImportRepository,
    edits: RedbEditRepository,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicCatalogStoreError {
    Unavailable,
    Conflict,
    Corrupt,
    CommitFailed,
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
            database: Arc::clone(&database),
            imports: RedbImportRepository::from_database(Arc::clone(&database)),
            edits: RedbEditRepository::from_database(database),
        })
    }

    /// Finds an exact-content import and its current persisted edit.
    ///
    /// # Errors
    ///
    /// Returns a typed store failure when persisted data cannot be read consistently.
    pub fn find_by_content(
        &self,
        sha256: [u8; 32],
        byte_length: u64,
    ) -> Result<Option<(ImportRecord, Edit)>, AtomicCatalogStoreError> {
        let record = self
            .imports
            .list()
            .map_err(|error| map_repository_error(&error))?
            .into_iter()
            .find(|record| {
                let asset = record.photo().primary_asset();
                asset.content_hash() == ContentHash::Sha256(sha256)
                    && asset.byte_length().get() == byte_length
            });
        let Some(record) = record else {
            return Ok(None);
        };
        let edit = self
            .edits
            .list()
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .into_iter()
            .filter(|edit| edit.photo_id() == record.photo().id())
            .max_by_key(|edit| (edit.revision().get(), edit.id().get()))
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        Ok(Some((record, edit)))
    }

    /// Atomically persists one import record and its neutral default edit.
    ///
    /// # Errors
    ///
    /// Returns before commit on any identity conflict, leaving no partial photo or edit.
    pub fn commit_import_with_edit(
        &mut self,
        record: &ImportRecord,
        edit: &Edit,
    ) -> Result<(), AtomicCatalogStoreError> {
        if edit.photo_id() != record.photo().id() {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let encoded_record =
            crate::codec::encode(record).map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let encoded_edit =
            crate::edit_codec::encode(edit).map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let source = record.source().as_str().as_bytes();
        let photo_id = record.photo().id().get().to_be_bytes();
        let asset_id = record.photo().primary_asset_id().get().to_be_bytes();
        let edit_id = edit.id().get().to_be_bytes();
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        {
            let mut records = transaction
                .open_table(schema::RECORDS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            let mut photos = transaction
                .open_table(schema::PHOTO_INDEX_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            let mut assets = transaction
                .open_table(schema::ASSET_INDEX_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            let mut edits = transaction
                .open_table(schema::EDITS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            if records
                .get(source)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?
                .is_some()
                || photos
                    .get(photo_id.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?
                    .is_some()
                || assets
                    .get(asset_id.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?
                    .is_some()
                || edits
                    .get(edit_id.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?
                    .is_some()
            {
                return Err(AtomicCatalogStoreError::Conflict);
            }
            records
                .insert(source, encoded_record.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            photos
                .insert(photo_id.as_slice(), source)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            assets
                .insert(asset_id.as_slice(), source)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            edits
                .insert(edit_id.as_slice(), encoded_edit.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)
    }
}

fn map_repository_error(error: &RepositoryError) -> AtomicCatalogStoreError {
    match error {
        RepositoryError::Unavailable => AtomicCatalogStoreError::Unavailable,
        RepositoryError::CommitFailure => AtomicCatalogStoreError::CommitFailed,
        RepositoryError::CorruptPersistedData => AtomicCatalogStoreError::Corrupt,
        RepositoryError::SourceConflict { .. }
        | RepositoryError::PhotoIdConflict { .. }
        | RepositoryError::AssetIdConflict { .. } => AtomicCatalogStoreError::Conflict,
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
