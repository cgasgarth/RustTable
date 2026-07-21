use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, WriteTransaction};
use rusttable_catalog::{
    EditRepository, EditRepositoryError, ImportDetails, ImportRecord, ImportRegistration,
    ImportRepository, RepositoryError, SourcePath,
};
use rusttable_core::{AssetId, ContentHash, Edit, EditId, PhotoId, Revision};

use super::RedbImportRepository;
use super::edit::RedbEditRepository;
use super::history::stage_history_commit;
use super::recipe::RedbRecipeRepository;
use crate::schema;

/// Shared redb catalog adapter for application compositions that need imports and edits.
pub struct RedbCatalogRepository {
    database: Arc<Database>,
    imports: RedbImportRepository,
    edits: RedbEditRepository,
    recipes: RedbRecipeRepository,
    before_commit: Option<BeforeCommitHook>,
}

type BeforeCommitHook = Arc<dyn Fn() -> Result<(), AtomicCatalogStoreError> + Send + Sync>;

struct PreparedImport {
    encoded_record: Vec<u8>,
    encoded_edit: Vec<u8>,
    source: Vec<u8>,
    photo_id: [u8; 16],
    asset_id: [u8; 16],
    edit_id: [u8; 16],
}

impl PreparedImport {
    fn new(record: &ImportRecord, edit: &Edit) -> Result<Self, AtomicCatalogStoreError> {
        Ok(Self {
            encoded_record: crate::codec::encode(record)
                .map_err(|()| AtomicCatalogStoreError::Corrupt)?,
            encoded_edit: crate::edit_codec::encode(edit)
                .map_err(|()| AtomicCatalogStoreError::Corrupt)?,
            source: record.source().as_str().as_bytes().to_vec(),
            photo_id: record.photo().id().get().to_be_bytes(),
            asset_id: record.photo().primary_asset_id().get().to_be_bytes(),
            edit_id: edit.id().get().to_be_bytes(),
        })
    }
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
        Self::open_with_hook(path, None)
    }

    /// Opens a repository with a hook immediately before an atomic commit.
    ///
    /// This is a test seam for verifying rollback after every write has been staged.
    /// Production callers should use [`Self::open`].
    ///
    /// # Errors
    ///
    /// Returns a typed storage failure when the catalog cannot be opened or validated.
    #[doc(hidden)]
    pub fn open_with_before_commit_hook<F>(path: &Path, hook: F) -> Result<Self, RepositoryError>
    where
        F: Fn() -> Result<(), AtomicCatalogStoreError> + Send + Sync + 'static,
    {
        Self::open_with_hook(path, Some(Arc::new(hook)))
    }

    fn open_with_hook(
        path: &Path,
        before_commit: Option<BeforeCommitHook>,
    ) -> Result<Self, RepositoryError> {
        let database = schema::open(path)?;
        Ok(Self {
            database: Arc::clone(&database),
            imports: RedbImportRepository::from_database(Arc::clone(&database)),
            edits: RedbEditRepository::from_database(Arc::clone(&database)),
            recipes: RedbRecipeRepository::from_database(Arc::clone(&database)),
            before_commit,
        })
    }

    /// Returns versioned export recipes backed by this same catalog database.
    #[must_use]
    pub const fn recipes(&self) -> &RedbRecipeRepository {
        &self.recipes
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

    /// Finds durable registration details by a persisted photo identity.
    ///
    /// Older catalog entries created before schema v3 return `None`.
    ///
    /// # Errors
    ///
    /// Returns a typed store failure when persisted data cannot be read consistently.
    pub fn find_import_details_by_photo_id(
        &self,
        photo_id: PhotoId,
    ) -> Result<Option<ImportDetails>, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let photos = transaction
            .open_table(schema::PHOTO_INDEX_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let source = photos
            .get(photo_id.get().to_be_bytes().as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let Some(source) = source else {
            return Ok(None);
        };
        let details = transaction
            .open_table(schema::IMPORT_DETAILS_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let details = details
            .get(source.value())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| {
                crate::import_details_codec::decode(value.value())
                    .map_err(|()| AtomicCatalogStoreError::Corrupt)
            })
            .transpose()?;
        if details
            .as_ref()
            .is_some_and(|details| details.receipt().photo_id() != photo_id)
        {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        Ok(details)
    }

    /// Atomically persists one import record, neutral default edit, and durable details.
    ///
    /// # Errors
    ///
    /// Returns before commit on any identity conflict, leaving no partial photo or edit.
    pub fn commit_import_with_edit(
        &mut self,
        record: &ImportRecord,
        edit: &Edit,
        registration: &ImportRegistration,
    ) -> Result<(), AtomicCatalogStoreError> {
        if registration.details().validate(record, edit).is_err() {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let prepared = PreparedImport::new(record, edit)?;
        let (history, expected_history, _) = self
            .edits
            .prepare_history(edit)
            .map_err(|error| map_edit_error(&error))?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        stage_import(&transaction, &prepared, registration)?;
        stage_history_commit(&transaction, edit.photo_id(), expected_history, &history)
            .map_err(|error| map_history_error(&error))?;
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)
    }
}

fn stage_import(
    transaction: &WriteTransaction,
    prepared: &PreparedImport,
    registration: &ImportRegistration,
) -> Result<(), AtomicCatalogStoreError> {
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
    let mut details = transaction
        .open_table(schema::IMPORT_DETAILS_TABLE)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    let mut reference_paths = transaction
        .open_table(schema::REFERENCE_PATH_INDEX_TABLE)
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    if records
        .get(prepared.source.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .is_some()
        || photos
            .get(prepared.photo_id.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?
            .is_some()
        || assets
            .get(prepared.asset_id.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?
            .is_some()
        || edits
            .get(prepared.edit_id.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?
            .is_some()
        || details
            .get(prepared.source.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?
            .is_some()
    {
        return Err(AtomicCatalogStoreError::Conflict);
    }
    let path_identity = registration.reference_path_identity().as_bytes();
    let previous_source = reference_paths
        .get(path_identity.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?
        .map(|value| value.value().to_vec());
    let replaced_photo_id = previous_source
        .as_deref()
        .map(|previous_source| {
            let previous_record = records
                .get(previous_source)
                .map_err(|_| AtomicCatalogStoreError::Corrupt)?
                .ok_or(AtomicCatalogStoreError::Corrupt)?;
            crate::codec::decode(previous_record.value())
                .map_err(|()| AtomicCatalogStoreError::Corrupt)
                .map(|record| record.photo().id())
        })
        .transpose()?;
    let details_with_reuse = registration
        .details()
        .clone()
        .with_replaces_photo_id(replaced_photo_id);
    let encoded_details = crate::import_details_codec::encode(&details_with_reuse)
        .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
    records
        .insert(
            prepared.source.as_slice(),
            prepared.encoded_record.as_slice(),
        )
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    photos
        .insert(prepared.photo_id.as_slice(), prepared.source.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    assets
        .insert(prepared.asset_id.as_slice(), prepared.source.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    edits
        .insert(
            prepared.edit_id.as_slice(),
            prepared.encoded_edit.as_slice(),
        )
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    details
        .insert(prepared.source.as_slice(), encoded_details.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    reference_paths
        .insert(path_identity.as_slice(), prepared.source.as_slice())
        .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
    Ok(())
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

fn map_edit_error(error: &EditRepositoryError) -> AtomicCatalogStoreError {
    match error {
        EditRepositoryError::Unavailable => AtomicCatalogStoreError::Unavailable,
        EditRepositoryError::CommitFailure => AtomicCatalogStoreError::CommitFailed,
        EditRepositoryError::NewEditIdConflict { .. }
        | EditRepositoryError::EditRevisionConflict { .. }
        | EditRepositoryError::UnknownEdit { .. }
        | EditRepositoryError::PhotoIdentityMismatch { .. }
        | EditRepositoryError::BasePhotoRevisionMismatch { .. } => {
            AtomicCatalogStoreError::Conflict
        }
        EditRepositoryError::CorruptPersistedData => AtomicCatalogStoreError::Corrupt,
    }
}

fn map_history_error(error: &rusttable_catalog::HistoryRepositoryError) -> AtomicCatalogStoreError {
    match error {
        rusttable_catalog::HistoryRepositoryError::Unavailable => {
            AtomicCatalogStoreError::Unavailable
        }
        rusttable_catalog::HistoryRepositoryError::CommitFailure => {
            AtomicCatalogStoreError::CommitFailed
        }
        rusttable_catalog::HistoryRepositoryError::VersionConflict { .. } => {
            AtomicCatalogStoreError::Conflict
        }
        rusttable_catalog::HistoryRepositoryError::CorruptPersistedData => {
            AtomicCatalogStoreError::Corrupt
        }
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
