#![allow(clippy::missing_errors_doc)]

use std::path::Path;
use std::sync::Arc;

use redb::{Database, ReadableDatabase, ReadableTable};
use rusttable_catalog::{
    ActiveLibraryView, ActiveLighttableProperty, ActiveLighttableSort,
    ActiveLighttableSortDirection, ActiveLighttableState, CollectionCommand, CollectionField,
    CollectionQuery, CollectionRepository, CollectionRepositoryError, CollectionSort,
    CollectionState, SavedCollection,
};
use sha2::{Digest, Sha256};

use crate::schema;

const STATE_KEY: &[u8] = b"state";
const ACTIVE_LIGHTTABLE_STATE_KEY: &[u8] = b"active-lighttable-v1";

type BeforeCommitHook = Arc<dyn Fn() -> Result<(), CollectionRepositoryError> + Send + Sync>;

/// Transactional redb adapter for saved, recent, and active library views.
pub struct RedbCollectionRepository {
    database: Arc<Database>,
    before_commit: Option<BeforeCommitHook>,
}

impl RedbCollectionRepository {
    pub fn open(path: &Path) -> Result<Self, CollectionRepositoryError> {
        let database = schema::open(path).map_err(|error| map_schema_error(&error))?;
        Ok(Self {
            database,
            before_commit: None,
        })
    }

    /// Opens a repository with a test-only failure seam immediately before commit.
    #[doc(hidden)]
    pub fn open_with_before_commit_hook<F>(
        path: &Path,
        hook: F,
    ) -> Result<Self, CollectionRepositoryError>
    where
        F: Fn() -> Result<(), CollectionRepositoryError> + Send + Sync + 'static,
    {
        let database = schema::open(path).map_err(|error| map_schema_error(&error))?;
        Ok(Self {
            database,
            before_commit: Some(Arc::new(hook)),
        })
    }

    /// Rechecks the state and all derived indexes without changing the catalog.
    pub fn check_integrity(&self) -> Result<(), CollectionRepositoryError> {
        let state = self.load()?;
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        let names = transaction
            .open_table(schema::COLLECTION_NAME_INDEX_TABLE)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let recent = transaction
            .open_table(schema::RECENT_QUERY_TABLE)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let expected_names = state
            .normalized_name_index()
            .into_iter()
            .flat_map(|(name, ids)| ids.into_iter().map(move |id| name_key(&name, id)))
            .collect::<Vec<_>>();
        let actual_names = names
            .iter()
            .map_err(|_| CollectionRepositoryError::Corrupt)?
            .map(|entry| {
                entry
                    .map(|(key, _)| key.value().to_vec())
                    .map_err(|_| CollectionRepositoryError::Corrupt)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if expected_names.iter().map(Vec::as_slice).collect::<Vec<_>>()
            != actual_names.iter().map(Vec::as_slice).collect::<Vec<_>>()
        {
            return Err(CollectionRepositoryError::Corrupt);
        }
        let actual_recent = recent
            .iter()
            .map_err(|_| CollectionRepositoryError::Corrupt)?
            .count();
        if actual_recent != state.recent().len() {
            return Err(CollectionRepositoryError::Corrupt);
        }
        Ok(())
    }

    fn commit_state(&self, state: &CollectionState) -> Result<(), CollectionRepositoryError> {
        let encoded =
            postcard::to_allocvec(state).map_err(|_| CollectionRepositoryError::Corrupt)?;
        let digest = Sha256::digest(&encoded);
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        {
            let mut states = transaction
                .open_table(schema::COLLECTION_STATE_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            states
                .insert(STATE_KEY, encoded.as_slice())
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            let mut collections = transaction
                .open_table(schema::COLLECTIONS_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            for key in collections
                .iter()
                .map_err(|_| CollectionRepositoryError::Unavailable)?
                .filter_map(Result::ok)
                .map(|(key, _)| key.value().to_vec())
                .collect::<Vec<_>>()
            {
                collections
                    .remove(key.as_slice())
                    .map_err(|_| CollectionRepositoryError::Unavailable)?;
            }
            for collection in state.saved() {
                let key = collection.id().get().to_be_bytes();
                let value = postcard::to_allocvec(collection)
                    .map_err(|_| CollectionRepositoryError::Corrupt)?;
                collections
                    .insert(key.as_slice(), value.as_slice())
                    .map_err(|_| CollectionRepositoryError::Unavailable)?;
            }
            let mut names = transaction
                .open_table(schema::COLLECTION_NAME_INDEX_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            for key in names
                .iter()
                .map_err(|_| CollectionRepositoryError::Unavailable)?
                .filter_map(Result::ok)
                .map(|(key, _)| key.value().to_vec())
                .collect::<Vec<_>>()
            {
                names
                    .remove(key.as_slice())
                    .map_err(|_| CollectionRepositoryError::Unavailable)?;
            }
            for (name, ids) in state.normalized_name_index() {
                for id in ids {
                    names
                        .insert(name_key(&name, id).as_slice(), &[][..])
                        .map_err(|_| CollectionRepositoryError::Unavailable)?;
                }
            }
            let mut recent = transaction
                .open_table(schema::RECENT_QUERY_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            for key in recent
                .iter()
                .map_err(|_| CollectionRepositoryError::Unavailable)?
                .filter_map(Result::ok)
                .map(|(key, _)| key.value().to_vec())
                .collect::<Vec<_>>()
            {
                recent
                    .remove(key.as_slice())
                    .map_err(|_| CollectionRepositoryError::Unavailable)?;
            }
            for query in state.recent() {
                let value =
                    postcard::to_allocvec(query).map_err(|_| CollectionRepositoryError::Corrupt)?;
                recent
                    .insert(query.identity().as_slice(), value.as_slice())
                    .map_err(|_| CollectionRepositoryError::Unavailable)?;
            }
            let mut active = transaction
                .open_table(schema::ACTIVE_VIEW_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            let active_value = postcard::to_allocvec(state.active())
                .map_err(|_| CollectionRepositoryError::Corrupt)?;
            active
                .insert(STATE_KEY, active_value.as_slice())
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            let mut integrity = transaction
                .open_table(schema::COLLECTION_INTEGRITY_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            integrity
                .insert(state.revision().to_be_bytes().as_slice(), digest.as_slice())
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
        }
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| CollectionRepositoryError::CommitFailed)
    }

    /// Loads the durable active lighttable state, migrating the older active-view payload when
    /// this is the first access after the feature was introduced.
    pub fn load_active_lighttable_state(
        &self,
    ) -> Result<ActiveLighttableState, CollectionRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        let table = transaction
            .open_table(schema::ACTIVE_VIEW_TABLE)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let value = table
            .get(ACTIVE_LIGHTTABLE_STATE_KEY)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let Some(value) = value else {
            drop(table);
            drop(transaction);
            let migrated = self.migrate_legacy_active_lighttable_state()?;
            self.persist_active_lighttable_state(&migrated)?;
            return Ok(migrated);
        };
        let state: ActiveLighttableState =
            postcard::from_bytes(value.value()).map_err(|_| CollectionRepositoryError::Corrupt)?;
        state
            .validate()
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        Ok(state)
    }

    /// Persists one complete active lighttable state in a single catalog transaction.
    pub fn persist_active_lighttable_state(
        &self,
        state: &ActiveLighttableState,
    ) -> Result<(), CollectionRepositoryError> {
        state
            .validate()
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let encoded =
            postcard::to_allocvec(state).map_err(|_| CollectionRepositoryError::Corrupt)?;
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        {
            let mut table = transaction
                .open_table(schema::ACTIVE_VIEW_TABLE)
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
            table
                .insert(ACTIVE_LIGHTTABLE_STATE_KEY, encoded.as_slice())
                .map_err(|_| CollectionRepositoryError::Unavailable)?;
        }
        if let Some(hook) = &self.before_commit {
            hook()?;
        }
        transaction
            .commit()
            .map_err(|_| CollectionRepositoryError::CommitFailed)
    }

    fn migrate_legacy_active_lighttable_state(
        &self,
    ) -> Result<ActiveLighttableState, CollectionRepositoryError> {
        let legacy = self.load()?;
        let definition = match legacy.active() {
            ActiveLibraryView::Saved(id) => legacy.by_id(*id).map(SavedCollection::view),
            ActiveLibraryView::Inline { definition, .. } => Some(definition),
        };
        let Some(definition) = definition else {
            return Ok(ActiveLighttableState::default());
        };
        let (property, search_text) = match definition.query() {
            CollectionQuery::Text { field, value } => match field {
                CollectionField::Filename => (ActiveLighttableProperty::Filename, value.clone()),
                CollectionField::Folder => (ActiveLighttableProperty::Folders, value.clone()),
                CollectionField::Tag | CollectionField::Camera | CollectionField::Lens => {
                    (ActiveLighttableProperty::Filename, String::new())
                }
            },
            CollectionQuery::RatingAtLeast(value) => {
                (ActiveLighttableProperty::Rating, value.to_string())
            }
            CollectionQuery::AllPhotos
            | CollectionQuery::Rejected(_)
            | CollectionQuery::ColorLabel(_)
            | CollectionQuery::And(_)
            | CollectionQuery::Opaque { .. } => (ActiveLighttableProperty::Filename, String::new()),
        };
        let sort = match definition.sort() {
            CollectionSort::FilenameAscending => ActiveLighttableSort::Filename,
            CollectionSort::CaptureTimeAscending => ActiveLighttableSort::CaptureTime,
            CollectionSort::RatingDescending => ActiveLighttableSort::Rating,
        };
        let selection = match legacy.active() {
            ActiveLibraryView::Inline {
                selection_anchor, ..
            } => selection_anchor.iter().copied().collect::<Vec<_>>(),
            ActiveLibraryView::Saved(_) => Vec::new(),
        };
        Ok(ActiveLighttableState::new(
            property,
            search_text,
            sort,
            ActiveLighttableSortDirection::Ascending,
            selection,
        ))
    }
}

impl CollectionRepository for RedbCollectionRepository {
    fn load(&self) -> Result<CollectionState, CollectionRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        let table = transaction
            .open_table(schema::COLLECTION_STATE_TABLE)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let Some(value) = table
            .get(STATE_KEY)
            .map_err(|_| CollectionRepositoryError::Corrupt)?
        else {
            return Ok(CollectionState::default());
        };
        let state: CollectionState =
            postcard::from_bytes(value.value()).map_err(|_| CollectionRepositoryError::Corrupt)?;
        state
            .validate()
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        drop(value);
        drop(table);
        drop(transaction);
        self.check_digest(&state)?;
        Ok(state)
    }

    fn apply(
        &mut self,
        command: CollectionCommand,
    ) -> Result<CollectionState, CollectionRepositoryError> {
        let mut state = self.load()?;
        state
            .apply(command)
            .map_err(CollectionRepositoryError::Conflict)?;
        self.commit_state(&state)?;
        Ok(state)
    }
}

impl RedbCollectionRepository {
    fn check_digest(&self, state: &CollectionState) -> Result<(), CollectionRepositoryError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| CollectionRepositoryError::Unavailable)?;
        let table = transaction
            .open_table(schema::COLLECTION_INTEGRITY_TABLE)
            .map_err(|_| CollectionRepositoryError::Corrupt)?;
        let Some(value) = table
            .get(state.revision().to_be_bytes().as_slice())
            .map_err(|_| CollectionRepositoryError::Corrupt)?
        else {
            return Ok(());
        };
        let encoded =
            postcard::to_allocvec(state).map_err(|_| CollectionRepositoryError::Corrupt)?;
        if value.value() != Sha256::digest(&encoded).as_slice() {
            return Err(CollectionRepositoryError::Corrupt);
        }
        Ok(())
    }
}

fn name_key(name: &str, id: rusttable_catalog::CollectionId) -> Vec<u8> {
    format!("{name}\0{id}").into_bytes()
}
fn map_schema_error(error: &rusttable_catalog::RepositoryError) -> CollectionRepositoryError {
    match error {
        rusttable_catalog::RepositoryError::Unavailable => CollectionRepositoryError::Unavailable,
        rusttable_catalog::RepositoryError::CommitFailure => {
            CollectionRepositoryError::CommitFailed
        }
        _ => CollectionRepositoryError::Corrupt,
    }
}
