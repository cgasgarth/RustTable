use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use rusttable_catalog::{
    TagAlias, TagCommand, TagError, TagId, TagIndexStats, TagMutationReceipt, TagRepository,
    TagSnapshot, TagState,
};
use rusttable_core::{PhotoId, Revision};

use crate::schema;

const STATE_KEY: &[u8] = b"tag-state-v1";
type BeforeCommitHook = Arc<dyn Fn() -> Result<(), TagError> + Send + Sync>;

/// Atomic redb adapter for canonical tags and rebuildable lookup/assignment indexes.
pub struct RedbTagRepository {
    database: Arc<Database>,
    before_commit: Option<BeforeCommitHook>,
}

impl RedbTagRepository {
    /// Opens the tag repository in the shared schema-versioned catalog.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    pub fn open(path: &Path) -> Result<Self, TagError> {
        Self::open_with_hook(path, None)
    }

    /// Opens a repository with a test-only failure seam immediately before commit.
    ///
    /// # Errors
    /// Returns a typed storage or corruption error.
    #[doc(hidden)]
    pub fn open_with_before_commit_hook<F>(path: &Path, hook: F) -> Result<Self, TagError>
    where
        F: Fn() -> Result<(), TagError> + Send + Sync + 'static,
    {
        Self::open_with_hook(path, Some(Arc::new(hook)))
    }

    fn open_with_hook(
        path: &Path,
        before_commit: Option<BeforeCommitHook>,
    ) -> Result<Self, TagError> {
        let database = schema::open(path).map_err(|error| map_schema_error(&error))?;
        Ok(Self {
            database,
            before_commit,
        })
    }

    fn decode_state(bytes: &[u8]) -> Result<TagState, TagError> {
        let snapshot: TagSnapshot =
            postcard::from_bytes(bytes).map_err(|_| TagError::CorruptPersistedData)?;
        TagState::restore(snapshot).map_err(|_| TagError::CorruptPersistedData)
    }

    fn load_from_write(transaction: &redb::WriteTransaction) -> Result<TagState, TagError> {
        let table = transaction
            .open_table(schema::TAG_STATE_TABLE)
            .map_err(|_| TagError::Unavailable)?;
        table
            .get(STATE_KEY)
            .map_err(|_| TagError::Unavailable)?
            .map_or_else(
                || Ok(TagState::new()),
                |value| Self::decode_state(value.value()),
            )
    }
}

impl TagRepository for RedbTagRepository {
    fn load(&self) -> Result<TagState, TagError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| TagError::Unavailable)?;
        let table = transaction
            .open_table(schema::TAG_STATE_TABLE)
            .map_err(|_| TagError::CorruptPersistedData)?;
        table
            .get(STATE_KEY)
            .map_err(|_| TagError::CorruptPersistedData)?
            .map_or_else(
                || Ok(TagState::new()),
                |value| Self::decode_state(value.value()),
            )
    }

    fn apply(
        &mut self,
        expected: Revision,
        command: TagCommand,
    ) -> Result<TagMutationReceipt, TagError> {
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| TagError::Unavailable)?;
        let mut state = Self::load_from_write(&transaction)?;
        if expected != state.revision() {
            return Err(TagError::RevisionConflict {
                expected,
                actual: state.revision(),
            });
        }
        validate_photos(&transaction, &command)?;
        let receipt = state.apply(expected, command)?;
        persist_state(&transaction, &state)?;
        replace_indexes(&transaction, &state)?;
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction.commit().map_err(|_| TagError::CommitFailure)?;
        Ok(receipt)
    }

    fn resolve(&self, path_or_alias: &str) -> Result<Option<TagId>, TagError> {
        let key = TagAlias::new(path_or_alias)?;
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| TagError::Unavailable)?;
        let paths = transaction
            .open_table(schema::TAG_PATH_INDEX_TABLE)
            .map_err(|_| TagError::CorruptPersistedData)?;
        if let Some(value) = paths
            .get(key.as_str().as_bytes())
            .map_err(|_| TagError::CorruptPersistedData)?
        {
            return decode_tag_id(value.value()).map(Some);
        }
        let aliases = transaction
            .open_table(schema::TAG_ALIAS_INDEX_TABLE)
            .map_err(|_| TagError::CorruptPersistedData)?;
        aliases
            .get(key.as_str().as_bytes())
            .map_err(|_| TagError::CorruptPersistedData)?
            .map(|value| decode_tag_id(value.value()))
            .transpose()
    }

    fn tags_for_photo(&self, photo_id: PhotoId) -> Result<Vec<TagId>, TagError> {
        let prefix = photo_id.get().to_be_bytes();
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| TagError::Unavailable)?;
        let index = transaction
            .open_table(schema::PHOTO_TAG_INDEX_TABLE)
            .map_err(|_| TagError::CorruptPersistedData)?;
        let mut ids = Vec::new();
        for entry in index.iter().map_err(|_| TagError::CorruptPersistedData)? {
            let (key, _) = entry.map_err(|_| TagError::CorruptPersistedData)?;
            let key = key.value();
            if key.starts_with(&prefix) {
                ids.push(decode_tag_id(
                    key.get(16..).ok_or(TagError::CorruptPersistedData)?,
                )?);
            }
        }
        let state = self.load()?;
        ids.sort_by_key(|id| (state.canonical_path(*id).map(str::to_owned), *id));
        Ok(ids)
    }

    fn photos_with_tag(
        &self,
        tag_id: TagId,
        include_descendants: bool,
    ) -> Result<Vec<PhotoId>, TagError> {
        let state = self.load()?;
        let root = state
            .canonical_path(tag_id)
            .ok_or(TagError::UnknownTag { tag_id })?;
        let prefix = format!("{root}|");
        let selected = state
            .projections()
            .into_iter()
            .filter(|tag| {
                tag.id == tag_id || (include_descendants && tag.canonical_path.starts_with(&prefix))
            })
            .map(|tag| tag.id)
            .collect::<std::collections::BTreeSet<_>>();
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| TagError::Unavailable)?;
        let index = transaction
            .open_table(schema::TAG_PHOTO_INDEX_TABLE)
            .map_err(|_| TagError::CorruptPersistedData)?;
        let mut photos = std::collections::BTreeSet::new();
        for entry in index.iter().map_err(|_| TagError::CorruptPersistedData)? {
            let (key, _) = entry.map_err(|_| TagError::CorruptPersistedData)?;
            let key = key.value();
            let stored_tag = decode_tag_id(key.get(..16).ok_or(TagError::CorruptPersistedData)?)?;
            if selected.contains(&stored_tag) {
                photos.insert(decode_photo_id(
                    key.get(16..).ok_or(TagError::CorruptPersistedData)?,
                )?);
            }
        }
        Ok(photos.into_iter().collect())
    }

    fn rebuild_indexes(&mut self) -> Result<TagIndexStats, TagError> {
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| TagError::Unavailable)?;
        let state = Self::load_from_write(&transaction)?;
        let index_stats = replace_indexes(&transaction, &state)?;
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction.commit().map_err(|_| TagError::CommitFailure)?;
        Ok(index_stats)
    }
}

fn validate_photos(
    transaction: &redb::WriteTransaction,
    command: &TagCommand,
) -> Result<(), TagError> {
    let photo_ids = match command {
        TagCommand::Assign { photo_ids, .. } | TagCommand::Remove { photo_ids, .. } => photo_ids,
        TagCommand::Create(_) | TagCommand::Update(_) => return Ok(()),
    };
    let photos = transaction
        .open_table(schema::PHOTO_INDEX_TABLE)
        .map_err(|_| TagError::Unavailable)?;
    for photo_id in photo_ids {
        if photos
            .get(photo_id.get().to_be_bytes().as_slice())
            .map_err(|_| TagError::Unavailable)?
            .is_none()
        {
            return Err(TagError::UnknownPhoto {
                photo_id: *photo_id,
            });
        }
    }
    Ok(())
}

fn persist_state(transaction: &redb::WriteTransaction, state: &TagState) -> Result<(), TagError> {
    let encoded =
        postcard::to_allocvec(&state.snapshot()).map_err(|_| TagError::CorruptPersistedData)?;
    let mut table = transaction
        .open_table(schema::TAG_STATE_TABLE)
        .map_err(|_| TagError::Unavailable)?;
    table
        .insert(STATE_KEY, encoded.as_slice())
        .map_err(|_| TagError::Unavailable)?;
    Ok(())
}

fn replace_indexes(
    transaction: &redb::WriteTransaction,
    state: &TagState,
) -> Result<TagIndexStats, TagError> {
    for definition in [
        schema::TAG_PATH_INDEX_TABLE,
        schema::TAG_ALIAS_INDEX_TABLE,
        schema::TAG_PHOTO_INDEX_TABLE,
        schema::PHOTO_TAG_INDEX_TABLE,
    ] {
        clear_table(transaction, definition)?;
    }

    let projections = state.projections();
    {
        let mut paths = transaction
            .open_table(schema::TAG_PATH_INDEX_TABLE)
            .map_err(|_| TagError::Unavailable)?;
        let mut aliases = transaction
            .open_table(schema::TAG_ALIAS_INDEX_TABLE)
            .map_err(|_| TagError::Unavailable)?;
        for projection in &projections {
            let id = projection.id.get().to_be_bytes();
            paths
                .insert(projection.canonical_path.as_bytes(), id.as_slice())
                .map_err(|_| TagError::Unavailable)?;
            for alias in &projection.aliases {
                aliases
                    .insert(alias.as_bytes(), id.as_slice())
                    .map_err(|_| TagError::Unavailable)?;
            }
        }
    }

    let assignments = state.assignments().collect::<Vec<_>>();
    {
        let mut tag_photos = transaction
            .open_table(schema::TAG_PHOTO_INDEX_TABLE)
            .map_err(|_| TagError::Unavailable)?;
        let mut photo_tags = transaction
            .open_table(schema::PHOTO_TAG_INDEX_TABLE)
            .map_err(|_| TagError::Unavailable)?;
        for (photo_id, tag_id) in &assignments {
            tag_photos
                .insert(tag_photo_key(*tag_id, *photo_id).as_slice(), &[][..])
                .map_err(|_| TagError::Unavailable)?;
            photo_tags
                .insert(photo_tag_key(*photo_id, *tag_id).as_slice(), &[][..])
                .map_err(|_| TagError::Unavailable)?;
        }
    }

    Ok(TagIndexStats {
        canonical_paths: projections.len(),
        aliases: projections.iter().map(|tag| tag.aliases.len()).sum(),
        assignments: assignments.len(),
    })
}

fn clear_table(
    transaction: &redb::WriteTransaction,
    definition: TableDefinition<&[u8], &[u8]>,
) -> Result<(), TagError> {
    let mut table = transaction
        .open_table(definition)
        .map_err(|_| TagError::Unavailable)?;
    let keys = table
        .iter()
        .map_err(|_| TagError::Unavailable)?
        .map(|entry| {
            entry
                .map(|(key, _)| key.value().to_vec())
                .map_err(|_| TagError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
    for key in keys {
        table
            .remove(key.as_slice())
            .map_err(|_| TagError::Unavailable)?;
    }
    Ok(())
}

fn decode_tag_id(bytes: &[u8]) -> Result<TagId, TagError> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| TagError::CorruptPersistedData)?;
    TagId::new(u128::from_be_bytes(bytes)).ok_or(TagError::CorruptPersistedData)
}

fn decode_photo_id(bytes: &[u8]) -> Result<PhotoId, TagError> {
    let bytes: [u8; 16] = bytes
        .try_into()
        .map_err(|_| TagError::CorruptPersistedData)?;
    PhotoId::new(u128::from_be_bytes(bytes)).ok_or(TagError::CorruptPersistedData)
}

fn tag_photo_key(tag_id: TagId, photo_id: PhotoId) -> [u8; 32] {
    let mut key = [0_u8; 32];
    key[..16].copy_from_slice(&tag_id.get().to_be_bytes());
    key[16..].copy_from_slice(&photo_id.get().to_be_bytes());
    key
}

fn photo_tag_key(photo_id: PhotoId, tag_id: TagId) -> [u8; 32] {
    let mut key = [0_u8; 32];
    key[..16].copy_from_slice(&photo_id.get().to_be_bytes());
    key[16..].copy_from_slice(&tag_id.get().to_be_bytes());
    key
}

fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> TagError {
    match error {
        rusttable_catalog::RepositoryError::CorruptPersistedData => TagError::CorruptPersistedData,
        _ => TagError::Unavailable,
    }
}
