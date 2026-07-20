//! Durable saved, recent, and active library-view state.

#![allow(clippy::missing_errors_doc, clippy::too_many_lines)]

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

const MAX_NAME_BYTES: usize = 128;
const MAX_DESCRIPTION_BYTES: usize = 4_096;
pub const MAX_RECENT_QUERIES: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CollectionId(u128);

impl CollectionId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for CollectionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionQuery {
    AllPhotos,
    Text {
        field: CollectionField,
        value: String,
    },
    RatingAtLeast(u8),
    Rejected(bool),
    ColorLabel(String),
    And(Vec<Self>),
    Opaque {
        source: String,
        payload: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CollectionField {
    Filename,
    Folder,
    Tag,
    Camera,
    Lens,
}

impl CollectionQuery {
    #[must_use]
    pub fn canonical(&self) -> String {
        match self {
            Self::AllPhotos => "all".to_owned(),
            Self::Text { field, value } => format!("text({:?},{})", field, canonical_text(value)),
            Self::RatingAtLeast(value) => format!("rating>=:{value}"),
            Self::Rejected(value) => format!("rejected:{value}"),
            Self::ColorLabel(value) => format!("label:{}", canonical_text(value)),
            Self::And(children) => {
                let mut children = children.iter().map(Self::canonical).collect::<Vec<_>>();
                children.sort();
                children.dedup();
                format!("and({})", children.join(","))
            }
            Self::Opaque { source, payload } => {
                format!(
                    "opaque({}, {})",
                    canonical_text(source),
                    canonical_text(payload)
                )
            }
        }
    }

    #[must_use]
    pub fn identity(&self, sort: CollectionSort, grouping: GroupCollapsePolicy) -> [u8; 32] {
        let input = format!("{}|sort={sort:?}|group={grouping:?}", self.canonical());
        let digest = Sha256::digest(input.as_bytes());
        let mut identity = [0_u8; 32];
        identity.copy_from_slice(&digest);
        identity
    }

    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CollectionSort {
    FilenameAscending,
    CaptureTimeAscending,
    RatingDescending,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum GroupCollapsePolicy {
    KeepExpanded,
    CollapseAll,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionViewDefinition {
    query: CollectionQuery,
    sort: CollectionSort,
    grouping: GroupCollapsePolicy,
}

impl CollectionViewDefinition {
    #[must_use]
    pub const fn new(
        query: CollectionQuery,
        sort: CollectionSort,
        grouping: GroupCollapsePolicy,
    ) -> Self {
        Self {
            query,
            sort,
            grouping,
        }
    }
    #[must_use]
    pub const fn query(&self) -> &CollectionQuery {
        &self.query
    }
    #[must_use]
    pub const fn sort(&self) -> CollectionSort {
        self.sort
    }
    #[must_use]
    pub const fn grouping(&self) -> GroupCollapsePolicy {
        self.grouping
    }
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.query.identity(self.sort, self.grouping)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollectionProvenance {
    Native,
    Migrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCollection {
    id: CollectionId,
    name: String,
    description: Option<String>,
    view: CollectionViewDefinition,
    revision: u64,
    provenance: CollectionProvenance,
}

impl SavedCollection {
    pub fn new(
        id: CollectionId,
        name: impl Into<String>,
        description: Option<String>,
        view: CollectionViewDefinition,
    ) -> Result<Self, CollectionValidationError> {
        let name = validate_name(name.into())?;
        validate_description(description.as_deref())?;
        Ok(Self {
            id,
            name,
            description,
            view,
            revision: 1,
            provenance: CollectionProvenance::Native,
        })
    }
    #[must_use]
    pub const fn id(&self) -> CollectionId {
        self.id
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
    #[must_use]
    pub const fn view(&self) -> &CollectionViewDefinition {
        &self.view
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn provenance(&self) -> CollectionProvenance {
        self.provenance
    }
    #[must_use]
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }
    #[must_use]
    pub fn with_provenance(mut self, provenance: CollectionProvenance) -> Self {
        self.provenance = provenance;
        self
    }
    pub fn rename(&mut self, name: impl Into<String>) -> Result<(), CollectionValidationError> {
        self.name = validate_name(name.into())?;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(CollectionValidationError::RevisionOverflow)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentQuery {
    definition: CollectionViewDefinition,
    last_used: u64,
    revision: u64,
}

impl RecentQuery {
    #[must_use]
    pub const fn new(definition: CollectionViewDefinition, last_used: u64) -> Self {
        Self {
            definition,
            last_used,
            revision: 1,
        }
    }
    #[must_use]
    pub const fn definition(&self) -> &CollectionViewDefinition {
        &self.definition
    }
    #[must_use]
    pub const fn last_used(&self) -> u64 {
        self.last_used
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn identity(&self) -> [u8; 32] {
        self.definition.identity()
    }
    fn touch(&mut self, last_used: u64) -> Result<(), CollectionValidationError> {
        self.last_used = last_used;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(CollectionValidationError::RevisionOverflow)?;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveLibraryView {
    Saved(CollectionId),
    Inline {
        definition: CollectionViewDefinition,
        selection_anchor: Option<u128>,
    },
}

impl ActiveLibraryView {
    #[must_use]
    pub fn all_photos() -> Self {
        Self::Inline {
            definition: CollectionViewDefinition::new(
                CollectionQuery::AllPhotos,
                CollectionSort::FilenameAscending,
                GroupCollapsePolicy::KeepExpanded,
            ),
            selection_anchor: None,
        }
    }
    #[must_use]
    pub const fn definition(&self) -> Option<&CollectionViewDefinition> {
        match self {
            Self::Saved(_) => None,
            Self::Inline { definition, .. } => Some(definition),
        }
    }
    #[must_use]
    pub const fn saved_id(&self) -> Option<CollectionId> {
        match self {
            Self::Saved(id) => Some(*id),
            Self::Inline { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionState {
    saved: BTreeMap<CollectionId, SavedCollection>,
    recent: BTreeMap<[u8; 32], RecentQuery>,
    active: ActiveLibraryView,
    revision: u64,
}

impl Default for CollectionState {
    fn default() -> Self {
        Self {
            saved: BTreeMap::new(),
            recent: BTreeMap::new(),
            active: ActiveLibraryView::all_photos(),
            revision: 0,
        }
    }
}

impl CollectionState {
    #[must_use]
    pub fn saved(&self) -> impl ExactSizeIterator<Item = &SavedCollection> {
        self.saved.values()
    }
    #[must_use]
    pub fn recent(&self) -> Vec<&RecentQuery> {
        let mut values = self.recent.values().collect::<Vec<_>>();
        values.sort_by_key(|query| (std::cmp::Reverse(query.last_used()), query.identity()));
        values
    }
    #[must_use]
    pub const fn active(&self) -> &ActiveLibraryView {
        &self.active
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub fn by_id(&self, id: CollectionId) -> Option<&SavedCollection> {
        self.saved.get(&id)
    }
    #[must_use]
    pub fn normalized_name_index(&self) -> BTreeMap<String, Vec<CollectionId>> {
        let mut index = BTreeMap::<String, Vec<CollectionId>>::new();
        for collection in self.saved.values() {
            index
                .entry(normalize_name(collection.name()))
                .or_default()
                .push(collection.id());
        }
        index
    }
    pub fn apply(&mut self, command: CollectionCommand) -> Result<(), CollectionError> {
        let mut next = self.clone();
        next.apply_inner(command)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or(CollectionError::RevisionOverflow)?;
        next.validate().map_err(CollectionError::InvalidState)?;
        *self = next;
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.recent.len() > MAX_RECENT_QUERIES {
            return Err("recent query cap exceeded".to_owned());
        }
        for collection in self.saved.values() {
            if collection.id().get() == 0 || collection.revision() == 0 {
                return Err("invalid saved collection identity or revision".to_owned());
            }
            if collection.view().query().is_opaque() {
                return Err("opaque collection cannot be executable".to_owned());
            }
        }
        for (identity, query) in &self.recent {
            if *identity != query.identity() {
                return Err("recent query identity index is stale".to_owned());
            }
        }
        if let ActiveLibraryView::Saved(id) = self.active {
            let Some(collection) = self.saved.get(&id) else {
                return Err("active collection is missing".to_owned());
            };
            if collection.view().query().is_opaque() {
                return Err("opaque collection cannot be active".to_owned());
            }
        }
        Ok(())
    }
    fn apply_inner(&mut self, command: CollectionCommand) -> Result<(), CollectionError> {
        match command {
            CollectionCommand::Create(collection) => {
                if collection.view().query().is_opaque() {
                    return Err(CollectionError::OpaqueCollection);
                }
                if self.saved.contains_key(&collection.id()) {
                    return Err(CollectionError::DuplicateId(collection.id()));
                }
                self.saved.insert(collection.id(), collection);
            }
            CollectionCommand::Update {
                collection,
                expected_revision,
            } => {
                let current = self
                    .saved
                    .get(&collection.id())
                    .ok_or(CollectionError::MissingCollection(collection.id()))?;
                if current.revision() != expected_revision {
                    return Err(CollectionError::StaleRevision {
                        expected: expected_revision,
                        actual: current.revision(),
                    });
                }
                if collection.revision() != expected_revision.saturating_add(1) {
                    return Err(CollectionError::InvalidRevision);
                }
                if collection.view().query().is_opaque() {
                    return Err(CollectionError::OpaqueCollection);
                }
                self.saved.insert(collection.id(), collection);
            }
            CollectionCommand::Rename {
                id,
                expected_revision,
                name,
            } => {
                let current = self
                    .saved
                    .get(&id)
                    .ok_or(CollectionError::MissingCollection(id))?;
                if current.revision() != expected_revision {
                    return Err(CollectionError::StaleRevision {
                        expected: expected_revision,
                        actual: current.revision(),
                    });
                }
                let mut renamed = current.clone();
                renamed.rename(name).map_err(CollectionError::Validation)?;
                self.saved.insert(id, renamed);
            }
            CollectionCommand::Delete {
                id,
                expected_revision,
            } => {
                let current = self
                    .saved
                    .get(&id)
                    .ok_or(CollectionError::MissingCollection(id))?;
                if current.revision() != expected_revision {
                    return Err(CollectionError::StaleRevision {
                        expected: expected_revision,
                        actual: current.revision(),
                    });
                }
                self.saved.remove(&id);
                if self.active.saved_id() == Some(id) {
                    self.active = ActiveLibraryView::all_photos();
                }
            }
            CollectionCommand::Duplicate {
                source,
                new_id,
                name,
            } => {
                let source = self
                    .saved
                    .get(&source)
                    .ok_or(CollectionError::MissingCollection(source))?
                    .clone();
                if self.saved.contains_key(&new_id) {
                    return Err(CollectionError::DuplicateId(new_id));
                }
                let duplicate = SavedCollection::new(
                    new_id,
                    name,
                    source.description().map(str::to_owned),
                    source.view().clone(),
                )
                .map_err(CollectionError::Validation)?;
                self.saved.insert(new_id, duplicate);
            }
            CollectionCommand::MarkRecent {
                definition,
                last_used,
            } => {
                let identity = definition.identity();
                if let Some(query) = self.recent.get_mut(&identity) {
                    query
                        .touch(last_used)
                        .map_err(CollectionError::Validation)?;
                } else {
                    self.recent
                        .insert(identity, RecentQuery::new(definition, last_used));
                }
                while self.recent.len() > MAX_RECENT_QUERIES {
                    let oldest = self
                        .recent
                        .iter()
                        .min_by_key(|(identity, query)| (query.last_used(), *identity))
                        .map(|(identity, _)| *identity)
                        .ok_or(CollectionError::InvalidRevision)?;
                    self.recent.remove(&oldest);
                }
            }
            CollectionCommand::SetActive(active) => {
                match &active {
                    ActiveLibraryView::Saved(id) => {
                        let collection = self
                            .saved
                            .get(id)
                            .ok_or(CollectionError::MissingCollection(*id))?;
                        if collection.view().query().is_opaque() {
                            return Err(CollectionError::OpaqueCollection);
                        }
                    }
                    ActiveLibraryView::Inline { definition, .. } => {
                        if definition.query().is_opaque() {
                            return Err(CollectionError::OpaqueCollection);
                        }
                    }
                }
                self.active = active;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionCommand {
    Create(SavedCollection),
    Update {
        collection: SavedCollection,
        expected_revision: u64,
    },
    Rename {
        id: CollectionId,
        expected_revision: u64,
        name: String,
    },
    Delete {
        id: CollectionId,
        expected_revision: u64,
    },
    Duplicate {
        source: CollectionId,
        new_id: CollectionId,
        name: String,
    },
    MarkRecent {
        definition: CollectionViewDefinition,
        last_used: u64,
    },
    SetActive(ActiveLibraryView),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionValidationError {
    EmptyName,
    NameTooLong,
    DescriptionTooLong,
    RevisionOverflow,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionError {
    DuplicateId(CollectionId),
    MissingCollection(CollectionId),
    StaleRevision { expected: u64, actual: u64 },
    InvalidRevision,
    RevisionOverflow,
    OpaqueCollection,
    Validation(CollectionValidationError),
    InvalidState(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionRepositoryError {
    Unavailable,
    Corrupt,
    Conflict(CollectionError),
    CommitFailed,
}

pub trait CollectionRepository {
    fn load(&self) -> Result<CollectionState, CollectionRepositoryError>;
    fn apply(
        &mut self,
        command: CollectionCommand,
    ) -> Result<CollectionState, CollectionRepositoryError>;
}

fn canonical_text(value: &str) -> String {
    value
        .nfkc()
        .map(|(character, _)| character)
        .collect::<String>()
        .trim()
        .to_lowercase()
}
fn normalize_name(value: &str) -> String {
    canonical_text(value)
}
fn validate_name(value: String) -> Result<String, CollectionValidationError> {
    if value.trim().is_empty() {
        return Err(CollectionValidationError::EmptyName);
    }
    if value.len() > MAX_NAME_BYTES {
        return Err(CollectionValidationError::NameTooLong);
    }
    Ok(value)
}
fn validate_description(value: Option<&str>) -> Result<(), CollectionValidationError> {
    if value.is_some_and(|value| value.len() > MAX_DESCRIPTION_BYTES) {
        Err(CollectionValidationError::DescriptionTooLong)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(query: CollectionQuery) -> CollectionViewDefinition {
        CollectionViewDefinition::new(
            query,
            CollectionSort::FilenameAscending,
            GroupCollapsePolicy::KeepExpanded,
        )
    }
    fn collection(id: u128, name: &str) -> SavedCollection {
        SavedCollection::new(
            CollectionId::new(id).unwrap(),
            name,
            None,
            view(CollectionQuery::AllPhotos),
        )
        .unwrap()
    }

    #[test]
    fn canonical_identity_is_order_independent_for_and_queries() {
        let left = CollectionQuery::And(vec![
            CollectionQuery::Rejected(false),
            CollectionQuery::AllPhotos,
        ]);
        let right = CollectionQuery::And(vec![
            CollectionQuery::AllPhotos,
            CollectionQuery::Rejected(false),
        ]);
        assert_eq!(view(left).identity(), view(right).identity());
    }

    #[test]
    fn recent_queries_deduplicate_and_evict_oldest() {
        let mut state = CollectionState::default();
        for index in 0..=MAX_RECENT_QUERIES {
            state
                .apply(CollectionCommand::MarkRecent {
                    definition: view(CollectionQuery::Text {
                        field: CollectionField::Tag,
                        value: index.to_string(),
                    }),
                    last_used: index as u64,
                })
                .unwrap();
        }
        assert_eq!(state.recent().len(), MAX_RECENT_QUERIES);
        state
            .apply(CollectionCommand::MarkRecent {
                definition: view(CollectionQuery::Text {
                    field: CollectionField::Tag,
                    value: "50".to_owned(),
                }),
                last_used: 99,
            })
            .unwrap();
        assert_eq!(state.recent().len(), MAX_RECENT_QUERIES);
        assert_eq!(state.recent()[0].last_used(), 99);
    }

    #[test]
    fn deleting_active_collection_falls_back_atomically() {
        let mut state = CollectionState::default();
        state
            .apply(CollectionCommand::Create(collection(1, "one")))
            .unwrap();
        state
            .apply(CollectionCommand::SetActive(ActiveLibraryView::Saved(
                CollectionId::new(1).unwrap(),
            )))
            .unwrap();
        state
            .apply(CollectionCommand::Delete {
                id: CollectionId::new(1).unwrap(),
                expected_revision: 1,
            })
            .unwrap();
        assert!(state.active().definition().is_some());
        assert!(state.active().saved_id().is_none());
        assert!(state.validate().is_ok());
    }

    #[test]
    fn opaque_queries_cannot_become_active() {
        let opaque = view(CollectionQuery::Opaque {
            source: "darktable".to_owned(),
            payload: "legacy".to_owned(),
        });
        assert!(
            SavedCollection::new(CollectionId::new(1).unwrap(), "opaque", None, opaque).is_ok()
        );
        let mut state = CollectionState::default();
        assert!(
            state
                .apply(CollectionCommand::SetActive(ActiveLibraryView::Inline {
                    definition: view(CollectionQuery::Opaque {
                        source: "darktable".to_owned(),
                        payload: "legacy".to_owned()
                    }),
                    selection_anchor: None
                }))
                .is_err()
        );
    }
}
