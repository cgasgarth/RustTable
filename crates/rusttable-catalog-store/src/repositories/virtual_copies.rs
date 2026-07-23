use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{
    SourceAssetIdentity, VirtualCopy, VirtualCopyCatalog, VirtualCopyCommand,
    VirtualCopyProjection, VirtualCopyRepository, VirtualCopyRepositoryError,
};
use rusttable_core::{Edit, Revision};

use crate::schema;

type BeforeCommitHook = Arc<dyn Fn() -> Result<(), VirtualCopyRepositoryError> + Send + Sync>;

/// Transactional redb adapter for virtual-copy identity, ordering, deletion, and history.
pub struct RedbVirtualCopyRepository {
    database: Arc<Database>,
    before_commit: Option<BeforeCommitHook>,
}

impl RedbVirtualCopyRepository {
    /// Opens the shared schema-versioned catalog database.
    ///
    /// # Errors
    ///
    /// Returns a schema, availability, or migration error.
    pub fn open(path: &Path) -> Result<Self, VirtualCopyRepositoryError> {
        Self::open_with_hook(path, None)
    }

    /// Opens the repository with a pre-commit failure seam for rollback tests.
    #[doc(hidden)]
    pub fn open_with_before_commit_hook<F>(
        path: &Path,
        hook: F,
    ) -> Result<Self, VirtualCopyRepositoryError>
    where
        F: Fn() -> Result<(), VirtualCopyRepositoryError> + Send + Sync + 'static,
    {
        Self::open_with_hook(path, Some(Arc::new(hook)))
    }

    pub(crate) const fn from_database(database: Arc<Database>) -> Self {
        Self {
            database,
            before_commit: None,
        }
    }

    fn open_with_hook(
        path: &Path,
        before_commit: Option<BeforeCommitHook>,
    ) -> Result<Self, VirtualCopyRepositoryError> {
        Ok(Self {
            database: schema::open(path).map_err(|error| map_schema_error(&error))?,
            before_commit,
        })
    }

    /// Returns active virtual copies in deterministic order.
    ///
    /// # Errors
    ///
    /// Returns a persistence or corruption error.
    pub fn projections(&self) -> Result<Vec<VirtualCopyProjection>, VirtualCopyRepositoryError> {
        Ok(self.load()?.projections())
    }

    /// Returns deletion tombstones as well as active copies.
    ///
    /// # Errors
    ///
    /// Returns a persistence or corruption error.
    pub fn all_projections(
        &self,
    ) -> Result<Vec<VirtualCopyProjection>, VirtualCopyRepositoryError> {
        Ok(self.load()?.all_projections())
    }

    /// Finds one virtual copy by identity.
    ///
    /// # Errors
    ///
    /// Returns a persistence or corruption error.
    pub fn copy(
        &self,
        id: rusttable_catalog::VirtualCopyId,
    ) -> Result<Option<VirtualCopy>, VirtualCopyRepositoryError> {
        Ok(self.load()?.copy(id).cloned())
    }

    fn validate_source(
        &self,
        source: SourceAssetIdentity,
    ) -> Result<(), VirtualCopyRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
        let photos = transaction
            .open_table(schema::PHOTO_INDEX_TABLE)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
        let Some(source_key) = photos
            .get(source.photo_id().get().to_be_bytes().as_slice())
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?
        else {
            return Err(VirtualCopyRepositoryError::SourceAssetNotFound { source });
        };
        let records = transaction
            .open_table(schema::RECORDS_TABLE)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
        let Some(record) = records
            .get(source_key.value())
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?
        else {
            return Err(VirtualCopyRepositoryError::SourceAssetNotFound { source });
        };
        let record = crate::codec::decode(record.value())
            .map_err(|()| VirtualCopyRepositoryError::Corrupt)?;
        if record.photo().primary_asset_id() != source.asset_id() {
            return Err(VirtualCopyRepositoryError::SourceAssetMismatch { source });
        }
        Ok(())
    }

    fn validate_edit_ids(&self, copy: &VirtualCopy) -> Result<(), VirtualCopyRepositoryError> {
        let current = self.load()?;
        let existing = current
            .copies()
            .flat_map(|value| value.history().map(Edit::id))
            .collect::<BTreeSet<_>>();
        if existing.contains(&copy.current_edit().id()) {
            return Err(VirtualCopyRepositoryError::EditIdConflict {
                edit_id: copy.current_edit().id(),
            });
        }
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
        let edits = transaction
            .open_table(schema::EDITS_TABLE)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
        if edits
            .get(copy.current_edit().id().get().to_be_bytes().as_slice())
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?
            .is_some()
        {
            return Err(VirtualCopyRepositoryError::EditIdConflict {
                edit_id: copy.current_edit().id(),
            });
        }
        Ok(())
    }

    fn commit_state(
        &self,
        expected: Revision,
        state: &VirtualCopyCatalog,
    ) -> Result<(), VirtualCopyRepositoryError> {
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
        {
            let revision = transaction
                .open_table(schema::VIRTUAL_COPY_STATE_TABLE)
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
            let actual = revision
                .get(schema::VIRTUAL_COPY_REVISION_KEY)
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?
                .map(|value| {
                    value
                        .value()
                        .try_into()
                        .map(|bytes| Revision::from_u64(u64::from_be_bytes(bytes)))
                        .map_err(|_| VirtualCopyRepositoryError::Corrupt)
                })
                .transpose()?
                .unwrap_or(Revision::ZERO);
            if actual != expected {
                return Err(VirtualCopyRepositoryError::Domain(
                    rusttable_catalog::VirtualCopyError::CatalogRevisionConflict {
                        expected,
                        actual,
                    },
                ));
            }
        }
        {
            let mut copies = transaction
                .open_table(schema::VIRTUAL_COPIES_TABLE)
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
            let keys = copies
                .iter()
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?
                .map(|entry| {
                    entry
                        .map(|(key, _)| key.value().to_vec())
                        .map_err(|_| VirtualCopyRepositoryError::Unavailable)
                })
                .collect::<Result<Vec<_>, _>>()?;
            for key in keys {
                copies
                    .remove(key.as_slice())
                    .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
            }
            for copy in state.copies() {
                let encoded = crate::virtual_copy_codec::encode(copy)
                    .map_err(|()| VirtualCopyRepositoryError::Corrupt)?;
                copies
                    .insert(copy.id().get().to_be_bytes().as_slice(), encoded.as_slice())
                    .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
            }
        }
        {
            let mut revision = transaction
                .open_table(schema::VIRTUAL_COPY_STATE_TABLE)
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
            revision
                .insert(
                    schema::VIRTUAL_COPY_REVISION_KEY,
                    state.revision().get().to_be_bytes().as_slice(),
                )
                .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
        }
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| VirtualCopyRepositoryError::CommitFailed)
    }
}

impl VirtualCopyRepository for RedbVirtualCopyRepository {
    fn load(&self) -> Result<VirtualCopyCatalog, VirtualCopyRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| VirtualCopyRepositoryError::Unavailable)?;
        let state = transaction
            .open_table(schema::VIRTUAL_COPY_STATE_TABLE)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
        let revision = state
            .get(schema::VIRTUAL_COPY_REVISION_KEY)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?
            .map(|value| {
                value
                    .value()
                    .try_into()
                    .map(|bytes| Revision::from_u64(u64::from_be_bytes(bytes)))
                    .map_err(|_| VirtualCopyRepositoryError::Corrupt)
            })
            .transpose()?
            .unwrap_or(Revision::ZERO);
        let copies = transaction
            .open_table(schema::VIRTUAL_COPIES_TABLE)
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
        let copies = copies
            .iter()
            .map_err(|_| VirtualCopyRepositoryError::Corrupt)?
            .map(|entry| {
                let (_, value) = entry.map_err(|_| VirtualCopyRepositoryError::Corrupt)?;
                crate::virtual_copy_codec::decode(value.value())
                    .map_err(|()| VirtualCopyRepositoryError::Corrupt)
            })
            .collect::<Result<Vec<_>, _>>()?;
        VirtualCopyCatalog::from_parts(revision, copies).map_err(VirtualCopyRepositoryError::Domain)
    }

    fn apply(
        &mut self,
        expected: Revision,
        command: VirtualCopyCommand,
    ) -> Result<Revision, VirtualCopyRepositoryError> {
        let mut state = self.load()?;
        if expected != state.revision() {
            return Err(VirtualCopyRepositoryError::Domain(
                rusttable_catalog::VirtualCopyError::CatalogRevisionConflict {
                    expected,
                    actual: state.revision(),
                },
            ));
        }
        if let VirtualCopyCommand::Create(copy) = &command {
            self.validate_source(copy.source())?;
            self.validate_edit_ids(copy)?;
        }
        let revision = state
            .apply(expected, command)
            .map_err(VirtualCopyRepositoryError::Domain)?;
        self.commit_state(expected, &state)?;
        Ok(revision)
    }
}

fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> VirtualCopyRepositoryError {
    match error {
        rusttable_catalog::RepositoryError::Unavailable => VirtualCopyRepositoryError::Unavailable,
        rusttable_catalog::RepositoryError::CommitFailure => {
            VirtualCopyRepositoryError::CommitFailed
        }
        rusttable_catalog::RepositoryError::CorruptPersistedData
        | rusttable_catalog::RepositoryError::SourceConflict { .. }
        | rusttable_catalog::RepositoryError::PhotoIdConflict { .. }
        | rusttable_catalog::RepositoryError::AssetIdConflict { .. } => {
            VirtualCopyRepositoryError::Corrupt
        }
    }
}
