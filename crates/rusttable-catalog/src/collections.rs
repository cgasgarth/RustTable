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

// Direct source mapping: src/common/collection.h, src/common/collection.c, and
// src/libs/collect.h. These lossless records are persistence preparation only;
// query compilation and the configuration adapter remain deferred.
pub const NATIVE_COLLECTION_MAX_RULES: usize = 10;
pub const NATIVE_COLLECTION_MODE_AND: i32 = 0;
pub const NATIVE_COLLECTION_MODE_OR: i32 = 1;
pub const NATIVE_COLLECTION_MODE_AND_NOT: i32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCollectionRule {
    mode: i32,
    item: i32,
    off: i32,
    top: i32,
    value: Vec<u8>,
}

impl NativeCollectionRule {
    #[must_use]
    pub fn collect(mode: i32, item: i32, value: impl Into<Vec<u8>>) -> Self {
        Self {
            mode,
            item,
            off: 0,
            top: 0,
            value: value.into(),
        }
    }

    #[must_use]
    pub fn filtering(mode: i32, item: i32, off: i32, top: i32, value: impl Into<Vec<u8>>) -> Self {
        Self {
            mode,
            item,
            off,
            top,
            value: value.into(),
        }
    }

    #[must_use]
    pub const fn mode(&self) -> i32 {
        self.mode
    }

    #[must_use]
    pub const fn item(&self) -> i32 {
        self.item
    }

    #[must_use]
    pub const fn off(&self) -> i32 {
        self.off
    }

    #[must_use]
    pub const fn top(&self) -> i32 {
        self.top
    }

    #[must_use]
    pub fn value(&self) -> &[u8] {
        &self.value
    }

    #[must_use]
    pub fn with_value(&self, value: impl Into<Vec<u8>>) -> Self {
        Self {
            value: value.into(),
            ..self.clone()
        }
    }

    #[must_use]
    pub const fn mode_kind(&self) -> NativeCollectionMode {
        match self.mode {
            NATIVE_COLLECTION_MODE_AND => NativeCollectionMode::And,
            NATIVE_COLLECTION_MODE_OR => NativeCollectionMode::Or,
            NATIVE_COLLECTION_MODE_AND_NOT => NativeCollectionMode::AndNot,
            mode => NativeCollectionMode::Unknown(mode),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NativeCollectionMode {
    And,
    Or,
    AndNot,
    Unknown(i32),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCollectionRules {
    filtering: bool,
    num_rules: i32,
    rules: Vec<NativeCollectionRule>,
}

impl NativeCollectionRules {
    pub fn new(
        filtering: bool,
        rules: Vec<NativeCollectionRule>,
    ) -> Result<Self, NativeCollectionError> {
        let num_rules =
            i32::try_from(rules.len()).map_err(|_| NativeCollectionError::RuleCountOverflow)?;
        Self::from_parts(filtering, num_rules, rules)
    }

    pub fn from_parts(
        filtering: bool,
        num_rules: i32,
        rules: Vec<NativeCollectionRule>,
    ) -> Result<Self, NativeCollectionError> {
        if num_rules < 0 && !rules.is_empty() {
            return Err(NativeCollectionError::RulePrefixExceedsDeclaredCount);
        }
        if num_rules >= 0 {
            let declared =
                usize::try_from(num_rules).map_err(|_| NativeCollectionError::RuleCountOverflow)?;
            if declared > rules.len() {
                return Err(NativeCollectionError::RulePrefixExceedsDeclaredCount);
            }
        }
        Ok(Self {
            filtering,
            num_rules,
            rules,
        })
    }

    pub fn collect(rules: Vec<NativeCollectionRule>) -> Result<Self, NativeCollectionError> {
        Self::new(false, rules)
    }

    pub fn filtering(rules: Vec<NativeCollectionRule>) -> Result<Self, NativeCollectionError> {
        Self::new(true, rules)
    }

    #[must_use]
    pub const fn filtering_mode(&self) -> bool {
        self.filtering
    }

    #[must_use]
    pub const fn num_rules(&self) -> i32 {
        self.num_rules
    }

    #[must_use]
    pub fn rules(&self) -> &[NativeCollectionRule] {
        &self.rules
    }

    /// Returns the ordered, bounded input that the native query builder consumes.
    ///
    /// This deliberately does not produce SQL or claim that an unknown property is
    /// executable. It only preserves the native bounds, order, modes, and disabled
    /// filtering rules for the later configuration/query integration seam.
    #[must_use]
    pub fn query_rules(&self) -> Vec<NativeCollectionRule> {
        let count = if self.filtering {
            self.num_rules.clamp(0, 10)
        } else {
            self.num_rules.clamp(1, 10)
        };
        let count = usize::try_from(count).unwrap_or_default();
        let mut rules = self.rules.iter().take(count).cloned().collect::<Vec<_>>();
        if !self.filtering && rules.is_empty() && count != 0 {
            rules.push(NativeCollectionRule::collect(
                NATIVE_COLLECTION_MODE_AND,
                0,
                b"%".to_vec(),
            ));
        }
        if self.filtering {
            rules = rules
                .iter()
                .map(|rule| {
                    if rule.off != 0 {
                        rule.with_value(Vec::new())
                    } else {
                        rule.clone()
                    }
                })
                .collect();
        }
        rules
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCollectionSortRule {
    sort_id: i32,
    sort_order: i32,
}

impl NativeCollectionSortRule {
    #[must_use]
    pub const fn new(sort_id: i32, sort_order: i32) -> Self {
        Self {
            sort_id,
            sort_order,
        }
    }

    #[must_use]
    pub const fn sort_id(&self) -> i32 {
        self.sort_id
    }

    #[must_use]
    pub const fn sort_order(&self) -> i32 {
        self.sort_order
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeCollectionSorts {
    num_sort: i32,
    rules: Vec<NativeCollectionSortRule>,
}

impl NativeCollectionSorts {
    pub fn new(rules: Vec<NativeCollectionSortRule>) -> Result<Self, NativeCollectionError> {
        let num_sort =
            i32::try_from(rules.len()).map_err(|_| NativeCollectionError::SortCountOverflow)?;
        Self::from_parts(num_sort, rules)
    }

    pub fn from_parts(
        num_sort: i32,
        rules: Vec<NativeCollectionSortRule>,
    ) -> Result<Self, NativeCollectionError> {
        if num_sort < 0 && !rules.is_empty() {
            return Err(NativeCollectionError::SortPrefixExceedsDeclaredCount);
        }
        if num_sort >= 0 {
            let declared =
                usize::try_from(num_sort).map_err(|_| NativeCollectionError::SortCountOverflow)?;
            if declared > rules.len() {
                return Err(NativeCollectionError::SortPrefixExceedsDeclaredCount);
            }
        }
        Ok(Self { num_sort, rules })
    }

    #[must_use]
    pub const fn num_sort(&self) -> i32 {
        self.num_sort
    }

    #[must_use]
    pub fn rules(&self) -> &[NativeCollectionSortRule] {
        &self.rules
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativeCollectionError {
    RuleCountOverflow,
    SortCountOverflow,
    RulePrefixExceedsDeclaredCount,
    SortPrefixExceedsDeclaredCount,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCollectionHistoryEntry {
    query: Vec<u8>,
    position: i32,
}

impl NativeCollectionHistoryEntry {
    #[must_use]
    pub fn new(query: impl Into<Vec<u8>>, position: i32) -> Self {
        Self {
            query: query.into(),
            position,
        }
    }

    #[must_use]
    pub fn query(&self) -> &[u8] {
        &self.query
    }

    #[must_use]
    pub const fn position(&self) -> i32 {
        self.position
    }
}

/// Applies `dt_collection_history_save`'s duplicate removal and backward shift
/// to configuration-like slots without touching any application or UI state.
#[must_use]
pub fn save_native_collection_history(
    entries: &[NativeCollectionHistoryEntry],
    current: impl Into<Vec<u8>>,
    history_max: usize,
    recent_max_items: usize,
) -> Vec<NativeCollectionHistoryEntry> {
    let current = current.into();
    let limit = history_max.max(recent_max_items);
    let slot_count = limit.max(1);
    let mut result = entries.iter().take(slot_count).cloned().collect::<Vec<_>>();
    result.resize_with(slot_count, || {
        NativeCollectionHistoryEntry::new(Vec::new(), 0)
    });

    if result[0].query == current {
        return result;
    }

    let mut move_count = 0_usize;
    for index in 1..limit {
        if result[index].query == current {
            move_count += 1;
            result[index].query.clear();
        } else if move_count > 0 {
            let query = std::mem::take(&mut result[index].query);
            let position = result[index].position;
            result[index - move_count].query = query;
            result[index - move_count].position = position;
        }
    }

    for index in (0..limit.saturating_sub(1)).rev() {
        result[index + 1] = result[index].clone();
    }
    result[0].query = current;
    result
}

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

/// The collection property supported by the lighttable toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActiveLighttableProperty {
    Filmroll,
    Folders,
    Rating,
    ColorLabel,
    Filename,
}

/// The collection sort mode supported by the lighttable toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActiveLighttableSort {
    Filename,
    CaptureTime,
    Rating,
}

/// The direction applied to the active lighttable sort mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ActiveLighttableSortDirection {
    Ascending,
    Descending,
}

/// Current version of the durable active-lighttable payload.
pub const ACTIVE_LIGHTTABLE_STATE_VERSION: u8 = 1;

/// Versioned durable state for the implemented lighttable collection surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveLighttableState {
    version: u8,
    property: ActiveLighttableProperty,
    search_text: String,
    sort: ActiveLighttableSort,
    direction: ActiveLighttableSortDirection,
    selected_photo_ids: Vec<u128>,
}

impl ActiveLighttableState {
    #[must_use]
    pub fn new(
        property: ActiveLighttableProperty,
        search_text: impl Into<String>,
        sort: ActiveLighttableSort,
        direction: ActiveLighttableSortDirection,
        selected_photo_ids: impl IntoIterator<Item = u128>,
    ) -> Self {
        let mut selected_photo_ids = selected_photo_ids.into_iter().collect::<Vec<_>>();
        selected_photo_ids.sort_unstable();
        selected_photo_ids.dedup();
        Self {
            version: ACTIVE_LIGHTTABLE_STATE_VERSION,
            property,
            search_text: search_text.into(),
            sort,
            direction,
            selected_photo_ids,
        }
    }

    #[must_use]
    pub const fn default_state() -> Self {
        Self {
            version: ACTIVE_LIGHTTABLE_STATE_VERSION,
            property: ActiveLighttableProperty::Filename,
            search_text: String::new(),
            sort: ActiveLighttableSort::Filename,
            direction: ActiveLighttableSortDirection::Ascending,
            selected_photo_ids: Vec::new(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }
    #[must_use]
    pub const fn property(&self) -> ActiveLighttableProperty {
        self.property
    }
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }
    #[must_use]
    pub const fn sort(&self) -> ActiveLighttableSort {
        self.sort
    }
    #[must_use]
    pub const fn direction(&self) -> ActiveLighttableSortDirection {
        self.direction
    }
    #[must_use]
    pub fn selected_photo_ids(&self) -> &[u128] {
        &self.selected_photo_ids
    }

    /// Validates the serialized shape before it reaches an application controller.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.version != ACTIVE_LIGHTTABLE_STATE_VERSION {
            return Err("unsupported active lighttable state version");
        }
        if self.search_text.len() > 4_096 {
            return Err("active lighttable search text is too long");
        }
        if self
            .selected_photo_ids
            .windows(2)
            .any(|ids| ids[0] >= ids[1])
        {
            return Err("active lighttable selection is not canonical");
        }
        if self.selected_photo_ids.contains(&0) {
            return Err("active lighttable selection contains a zero photo ID");
        }
        Ok(())
    }
}

impl Default for ActiveLighttableState {
    fn default() -> Self {
        Self::default_state()
    }
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
