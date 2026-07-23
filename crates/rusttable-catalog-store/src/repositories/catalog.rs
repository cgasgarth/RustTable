use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, WriteTransaction};
use rusttable_catalog::{
    CatalogChangeEvent, CatalogCommand, ColorLabel, DuplicateEvidence, EditRepository,
    EditRepositoryError, ImportDetails, ImportMetadataStatus, ImportRecord, ImportRegistration,
    ImportRepository, PhotoOrganizationState, Rating, ReferencePathIdentity, RepositoryError,
    SourcePath,
};
use rusttable_core::{AssetId, ContentHash, Edit, EditId, ImageMetadata, PhotoId, Revision};

use super::RedbImportRepository;
use super::edit::RedbEditRepository;
use super::history::stage_history_commit;
use super::recipe::RedbRecipeRepository;
use crate::schema;

mod duplicates;
mod photo_groups;

use duplicates::stage_duplicate_evidence;

/// Shared redb catalog adapter for application compositions that need imports and edits.
pub struct RedbCatalogRepository {
    database: Arc<Database>,
    imports: RedbImportRepository,
    edits: RedbEditRepository,
    recipes: RedbRecipeRepository,
    before_commit: Option<BeforeCommitHook>,
    change_listener: Option<ChangeListener>,
}

type BeforeCommitHook = Arc<dyn Fn() -> Result<(), AtomicCatalogStoreError> + Send + Sync>;
type ChangeListener = Arc<dyn Fn(&CatalogChangeEvent) + Send + Sync>;

struct PreparedImport {
    encoded_record: Vec<u8>,
    encoded_edit: Vec<u8>,
    source: Vec<u8>,
    photo_id: [u8; 16],
    asset_id: [u8; 16],
    edit_id: [u8; 16],
}

enum OrganizationUpdate {
    Rating(Rating),
    Rejection(bool),
    Label(ColorLabel, bool),
    ToggleLabel(ColorLabel),
}

impl OrganizationUpdate {
    fn apply(&self, state: &mut PhotoOrganizationState) {
        match self {
            Self::Rating(rating) => {
                state.rating = *rating;
                state.rejected = false;
            }
            Self::Rejection(rejected) => state.rejected = *rejected,
            Self::Label(label, enabled) => {
                if *enabled {
                    state.color_labels.insert(*label);
                } else {
                    state.color_labels.remove(label);
                }
            }
            Self::ToggleLabel(label) => {
                if !state.color_labels.insert(*label) {
                    state.color_labels.remove(label);
                }
            }
        }
    }
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

/// Read-only outcome of the source-identity reconciliation migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SourceReconciliationReport {
    pub migrated_entries: usize,
    pub ambiguous_entries: usize,
    pub invalid_entries: usize,
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
            change_listener: None,
        })
    }

    /// Installs a callback invoked after a durable organization transaction commits.
    pub fn set_change_listener<F>(&mut self, listener: F)
    where
        F: Fn(&CatalogChangeEvent) + Send + Sync + 'static,
    {
        self.change_listener = Some(Arc::new(listener));
    }

    /// Returns versioned export recipes backed by this same catalog database.
    #[must_use]
    pub const fn recipes(&self) -> &RedbRecipeRepository {
        &self.recipes
    }

    /// Finds the current registration for one canonical source path.
    ///
    /// This is the authoritative import lookup. Content matches are exposed
    /// separately as duplicate hints and never participate in this decision.
    ///
    /// # Errors
    ///
    /// Returns a typed store failure when the path index, record, or edit is
    /// unavailable or inconsistent.
    pub fn find_by_reference_path(
        &self,
        identity: ReferencePathIdentity,
    ) -> Result<Option<(ImportRecord, Edit)>, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let paths = transaction
            .open_table(schema::REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let Some(source) = paths
            .get(identity.as_bytes().as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
        else {
            return Ok(None);
        };
        let source = source.value().to_vec();
        let records = transaction
            .open_table(schema::RECORDS_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let record = records
            .get(source.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| crate::codec::decode(value.value()))
            .transpose()
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        drop(records);
        drop(paths);
        drop(transaction);
        let edit = self.current_edit(record.photo().id())?;
        Ok(Some((record, edit)))
    }

    /// Reports legacy source rows that were migrated, preserved as ambiguous,
    /// or left for manual repair. No row is deleted by this operation.
    ///
    /// # Errors
    ///
    /// Returns a typed store failure when reconciliation metadata is unreadable.
    pub fn source_reconciliation_report(
        &self,
    ) -> Result<SourceReconciliationReport, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let table = transaction
            .open_table(schema::SOURCE_RECONCILIATION_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let mut report = SourceReconciliationReport::default();
        for entry in table.iter().map_err(|_| AtomicCatalogStoreError::Corrupt)? {
            let (key, _) = entry.map_err(|_| AtomicCatalogStoreError::Corrupt)?;
            if key.value().starts_with(b"migrated/") {
                report.migrated_entries = report.migrated_entries.saturating_add(1);
            } else if key.value().starts_with(b"ambiguous/") {
                report.ambiguous_entries = report.ambiguous_entries.saturating_add(1);
            } else if key.value().starts_with(b"invalid/") {
                report.invalid_entries = report.invalid_entries.saturating_add(1);
            }
        }
        Ok(report)
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
        let edit = self.current_edit(record.photo().id())?;
        Ok(Some((record, edit)))
    }

    fn current_edit(&self, photo_id: PhotoId) -> Result<Edit, AtomicCatalogStoreError> {
        self.edits
            .list()
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .into_iter()
            .filter(|edit| edit.photo_id() == photo_id)
            .max_by_key(|edit| (edit.revision().get(), edit.id().get()))
            .ok_or(AtomicCatalogStoreError::Corrupt)
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

    /// Returns all persisted organization state, defaulting legacy photos to neutral values.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error when the catalog cannot be read.
    pub fn organization_states(
        &self,
    ) -> Result<BTreeMap<PhotoId, PhotoOrganizationState>, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let records = transaction
            .open_table(schema::RECORDS_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let organization = transaction
            .open_table(schema::PHOTO_ORGANIZATION_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let mut states = BTreeMap::new();
        for entry in records
            .iter()
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
        {
            let (_, value) = entry.map_err(|_| AtomicCatalogStoreError::Corrupt)?;
            let record = crate::codec::decode(value.value())
                .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
            let photo_id = record.photo().id();
            let state = organization
                .get(photo_id.get().to_be_bytes().as_slice())
                .map_err(|_| AtomicCatalogStoreError::Corrupt)?
                .map(|value| {
                    crate::organization_codec::decode(photo_id, value.value())
                        .map_err(|()| AtomicCatalogStoreError::Corrupt)
                })
                .transpose()?
                .unwrap_or_else(|| PhotoOrganizationState::new(photo_id));
            states.insert(photo_id, state);
        }
        Ok(states)
    }

    /// Returns the current durable organization revision.
    ///
    /// # Errors
    ///
    /// Returns a typed storage or corruption error when the revision cannot be read.
    pub fn organization_revision(&self) -> Result<Revision, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let revisions = transaction
            .open_table(schema::ORGANIZATION_REVISION_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let value = revisions
            .get(schema::ORGANIZATION_REVISION_KEY)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| {
                value
                    .value()
                    .try_into()
                    .map(u64::from_be_bytes)
                    .map(Revision::from_u64)
                    .map_err(|_| AtomicCatalogStoreError::Corrupt)
            })
            .transpose()?
            .unwrap_or(Revision::ZERO);
        Ok(value)
    }

    /// Applies one rating/rejection/label command as a single durable transaction.
    ///
    /// # Errors
    ///
    /// Returns a typed validation, storage, or commit error. No event is emitted when the
    /// transaction fails.
    pub fn apply_organization_command(
        &mut self,
        command: &CatalogCommand,
    ) -> Result<CatalogChangeEvent, AtomicCatalogStoreError> {
        let (photo_ids, states) = self.prepare_organization_command(command)?;
        let current_revision = self.organization_revision()?;
        let next_revision = current_revision
            .checked_increment()
            .map_err(|_| AtomicCatalogStoreError::Conflict)?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let mut organization = transaction
            .open_table(schema::PHOTO_ORGANIZATION_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        for state in &states {
            let encoded = crate::organization_codec::encode(state)
                .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
            organization
                .insert(
                    state.photo_id.get().to_be_bytes().as_slice(),
                    encoded.as_slice(),
                )
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        drop(organization);
        let mut revisions = transaction
            .open_table(schema::ORGANIZATION_REVISION_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        revisions
            .insert(
                schema::ORGANIZATION_REVISION_KEY,
                next_revision.get().to_be_bytes().as_slice(),
            )
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        drop(revisions);
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)?;
        let event = CatalogChangeEvent::new(next_revision, photo_ids);
        if let Some(listener) = &self.change_listener {
            listener(&event);
        }
        Ok(event)
    }

    fn prepare_organization_command(
        &self,
        command: &CatalogCommand,
    ) -> Result<(Vec<PhotoId>, Vec<PhotoOrganizationState>), AtomicCatalogStoreError> {
        let states = self.organization_states()?;
        let (photo_ids, update) = match command {
            CatalogCommand::SetRating { photo_ids, rating } => {
                (photo_ids, OrganizationUpdate::Rating(*rating))
            }
            CatalogCommand::SetRejection {
                photo_ids,
                rejected,
            } => (photo_ids, OrganizationUpdate::Rejection(*rejected)),
            CatalogCommand::SetColorLabel {
                photo_ids,
                label,
                enabled,
            } => (photo_ids, OrganizationUpdate::Label(*label, *enabled)),
            CatalogCommand::ToggleColorLabel { photo_ids, label } => {
                (photo_ids, OrganizationUpdate::ToggleLabel(*label))
            }
            CatalogCommand::RegisterPhoto(_)
            | CatalogCommand::CreateEdit(_)
            | CatalogCommand::ReplaceEdit { .. } => return Err(AtomicCatalogStoreError::Corrupt),
        };
        if photo_ids.is_empty() {
            return Err(AtomicCatalogStoreError::Conflict);
        }
        let mut unique = photo_ids.clone();
        unique.sort_unstable();
        if unique.windows(2).any(|window| window[0] == window[1]) {
            return Err(AtomicCatalogStoreError::Conflict);
        }
        let mut updated = Vec::with_capacity(unique.len());
        for photo_id in &unique {
            let mut state = states
                .get(photo_id)
                .cloned()
                .ok_or(AtomicCatalogStoreError::Conflict)?;
            update.apply(&mut state);
            updated.push(state);
        }
        Ok((unique, updated))
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
        if registration.details().validate(record, edit).is_err()
            || registration.duplicate_evidence().is_some_and(|evidence| {
                evidence.source() != registration.reference_path_identity()
                    || !evidence.describes(record)
            })
        {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let prepared = PreparedImport::new(record, edit)?;
        let prepared_group = registration
            .photo_group()
            .map(|group_id| self.prepare_import_photo_group(group_id, record.photo().id()))
            .transpose()?;
        let (history, expected_history, _) = self
            .edits
            .prepare_history(edit)
            .map_err(|error| map_edit_error(&error))?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        stage_import(&transaction, &prepared, registration)?;
        if let Some(group) = prepared_group {
            Self::stage_photo_group_membership(&transaction, &group, record.photo().id())?;
        }
        if let Some(evidence) = registration.duplicate_evidence() {
            stage_duplicate_evidence(&transaction, evidence, false)?;
        }
        stage_history_commit(&transaction, edit.photo_id(), expected_history, &history)
            .map_err(|error| map_history_error(&error))?;
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)
    }

    /// Atomically updates source evidence for an existing canonical path while
    /// retaining its photo, edit, organization, and history identities.
    ///
    /// # Errors
    ///
    /// Returns before commit on any identity, validation, or storage conflict.
    #[expect(
        clippy::too_many_lines,
        reason = "the replacement transaction keeps source, index, details, and hook ordering visible"
    )]
    pub fn replace_import(
        &mut self,
        record: &ImportRecord,
        replacement_edit: &Edit,
        registration: &ImportRegistration,
    ) -> Result<(), AtomicCatalogStoreError> {
        let path_source = self
            .source_for_reference_path(registration.reference_path_identity())?
            .ok_or(AtomicCatalogStoreError::Conflict)?;
        let old_record = self
            .imports
            .find_by_source(
                &SourcePath::new(
                    std::str::from_utf8(&path_source)
                        .map_err(|_| AtomicCatalogStoreError::Corrupt)?,
                )
                .map_err(|_| AtomicCatalogStoreError::Corrupt)?,
            )
            .map_err(|error| map_repository_error(&error))?
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        let edit = self.current_edit(old_record.photo().id())?;
        if old_record.photo().id() != record.photo().id()
            || old_record.photo().primary_asset_id() != record.photo().primary_asset_id()
            || edit.id() != replacement_edit.id()
        {
            // A legacy content-derived record may own this path after the
            // v10 index reconciliation. Keep it reachable and publish the
            // path-derived registration as an explicit replacement.
            return self.commit_import_with_edit(record, replacement_edit, registration);
        }
        if registration.details().validate(record, &edit).is_err()
            || registration.duplicate_evidence().is_some_and(|evidence| {
                evidence.source() != registration.reference_path_identity()
                    || !evidence.describes(record)
            })
        {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let prepared =
            crate::codec::encode(record).map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let details = crate::import_details_codec::encode(registration.details())
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let new_source = record.source().as_str().as_bytes().to_vec();
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        {
            let mut records = transaction
                .open_table(schema::RECORDS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            let mut details_table = transaction
                .open_table(schema::IMPORT_DETAILS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            if path_source != new_source {
                if records
                    .get(new_source.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?
                    .is_some()
                {
                    return Err(AtomicCatalogStoreError::Conflict);
                }
                records
                    .remove(path_source.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
                details_table
                    .remove(path_source.as_slice())
                    .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            }
            records
                .insert(new_source.as_slice(), prepared.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            details_table
                .insert(new_source.as_slice(), details.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        if path_source != new_source {
            let mut photos = transaction
                .open_table(schema::PHOTO_INDEX_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            let mut assets = transaction
                .open_table(schema::ASSET_INDEX_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            photos
                .insert(
                    record.photo().id().get().to_be_bytes().as_slice(),
                    new_source.as_slice(),
                )
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            assets
                .insert(
                    record
                        .photo()
                        .primary_asset_id()
                        .get()
                        .to_be_bytes()
                        .as_slice(),
                    new_source.as_slice(),
                )
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        let mut references = transaction
            .open_table(schema::REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        references
            .insert(
                registration.reference_path_identity().as_bytes().as_slice(),
                new_source.as_slice(),
            )
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        drop(references);
        if let Some(evidence) = registration.duplicate_evidence() {
            stage_duplicate_evidence(&transaction, evidence, true)?;
        }
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)
    }

    fn source_for_reference_path(
        &self,
        identity: ReferencePathIdentity,
    ) -> Result<Option<Vec<u8>>, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let paths = transaction
            .open_table(schema::REFERENCE_PATH_INDEX_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        paths
            .get(identity.as_bytes().as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)
            .map(|value| value.map(|value| value.value().to_vec()))
    }

    /// Atomically replaces metadata for an existing photo while preserving its photo and edit.
    ///
    /// # Errors
    ///
    /// Returns before commit on any missing, corrupt, or storage state, leaving the old record
    /// and import details intact.
    #[expect(
        clippy::too_many_lines,
        reason = "metadata, duplicate evidence, and import details share one auditable transaction"
    )]
    pub fn refresh_import_metadata(
        &mut self,
        photo_id: PhotoId,
        metadata: ImageMetadata,
    ) -> Result<ImportRecord, AtomicCatalogStoreError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        let photos = transaction
            .open_table(schema::PHOTO_INDEX_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let source = photos
            .get(photo_id.get().to_be_bytes().as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| value.value().to_vec())
            .ok_or(AtomicCatalogStoreError::Conflict)?;
        let records = transaction
            .open_table(schema::RECORDS_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?;
        let record = records
            .get(source.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| crate::codec::decode(value.value()))
            .transpose()
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        if record.photo().id() != photo_id {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let details = transaction
            .open_table(schema::IMPORT_DETAILS_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .get(source.as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| crate::import_details_codec::decode(value.value()))
            .transpose()
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        let duplicate_evidence = transaction
            .open_table(schema::DUPLICATE_EVIDENCE_TABLE)
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .get(photo_id.get().to_be_bytes().as_slice())
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .map(|value| crate::duplicate_codec::decode(value.value()))
            .transpose()
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        drop(records);
        drop(photos);
        drop(transaction);

        let edit = self
            .edits
            .list()
            .map_err(|_| AtomicCatalogStoreError::Corrupt)?
            .into_iter()
            .filter(|edit| edit.photo_id() == photo_id)
            .max_by_key(|edit| (edit.revision().get(), edit.id().get()))
            .ok_or(AtomicCatalogStoreError::Corrupt)?;
        let refreshed = record.with_metadata(metadata);
        let refreshed_details = ImportDetails::new(
            rusttable_catalog::ImportMetadataSummary::from_record_with_status(
                &refreshed,
                ImportMetadataStatus::Available,
            ),
            details.receipt().clone(),
        );
        let refreshed_duplicate_evidence = duplicate_evidence.map(|evidence| {
            DuplicateEvidence::from_record(&refreshed, evidence.source(), evidence.visual())
        });
        if refreshed_details.validate(&refreshed, &edit).is_err() {
            return Err(AtomicCatalogStoreError::Corrupt);
        }
        let encoded_record =
            crate::codec::encode(&refreshed).map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let encoded_details = crate::import_details_codec::encode(&refreshed_details)
            .map_err(|()| AtomicCatalogStoreError::Corrupt)?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        {
            let mut records = transaction
                .open_table(schema::RECORDS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            records
                .insert(source.as_slice(), encoded_record.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        {
            let mut details_table = transaction
                .open_table(schema::IMPORT_DETAILS_TABLE)
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
            details_table
                .insert(source.as_slice(), encoded_details.as_slice())
                .map_err(|_| AtomicCatalogStoreError::Unavailable)?;
        }
        if let Some(evidence) = refreshed_duplicate_evidence {
            stage_duplicate_evidence(&transaction, evidence, true)?;
        }
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| AtomicCatalogStoreError::CommitFailed)?;
        Ok(refreshed)
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
