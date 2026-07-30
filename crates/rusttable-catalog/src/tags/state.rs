use std::collections::{BTreeMap, BTreeSet};

use rusttable_core::{PhotoId, Revision};
use serde::{Deserialize, Serialize};

use super::types::{
    MAX_ASSIGNMENT_PHOTOS, MAX_ASSIGNMENT_TAGS, MAX_TAG_DEPTH, MAX_TAGGED_PHOTOS, MAX_TAGS,
    MAX_TAGS_PER_PHOTO, canonical_lookup,
};
use super::{TAG_SCHEMA_VERSION, TagDefinition, TagError, TagId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TagCommand {
    Create(TagDefinition),
    Update(TagDefinition),
    Assign {
        photo_ids: Vec<PhotoId>,
        tag_ids: Vec<TagId>,
    },
    Remove {
        photo_ids: Vec<PhotoId>,
        tag_ids: Vec<TagId>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagMutationReceipt {
    pub revision: Revision,
    pub photo_ids: Vec<PhotoId>,
    pub tag_ids: Vec<TagId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TagIndexStats {
    pub canonical_paths: usize,
    pub aliases: usize,
    pub assignments: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagProjection {
    pub id: TagId,
    pub parent_id: Option<TagId>,
    pub name: String,
    pub canonical_path: String,
    pub aliases: Vec<String>,
    pub direct_photo_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSnapshot {
    schema_version: u16,
    revision: u64,
    tags: BTreeMap<TagId, TagDefinition>,
    #[serde(with = "assignment_serde")]
    assignments: BTreeMap<PhotoId, BTreeSet<TagId>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagState {
    snapshot: TagSnapshot,
    paths_by_id: BTreeMap<TagId, String>,
    path_index: BTreeMap<String, TagId>,
    alias_index: BTreeMap<String, TagId>,
    children: BTreeMap<Option<TagId>, Vec<TagId>>,
    tag_photos: BTreeMap<TagId, BTreeSet<PhotoId>>,
}

impl TagState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: TagSnapshot {
                schema_version: TAG_SCHEMA_VERSION,
                revision: 0,
                tags: BTreeMap::new(),
                assignments: BTreeMap::new(),
            },
            paths_by_id: BTreeMap::new(),
            path_index: BTreeMap::new(),
            alias_index: BTreeMap::new(),
            children: BTreeMap::new(),
            tag_photos: BTreeMap::new(),
        }
    }

    /// Restores canonical durable values and deterministically rebuilds all indexes.
    ///
    /// # Errors
    /// Returns a bounded schema, hierarchy, identity, alias, or assignment error.
    pub fn restore(snapshot: TagSnapshot) -> Result<Self, TagError> {
        if snapshot.schema_version != TAG_SCHEMA_VERSION {
            return Err(TagError::UnsupportedSchema {
                version: snapshot.schema_version,
            });
        }
        if snapshot.tags.len() > MAX_TAGS {
            return Err(TagError::TooManyTags);
        }
        if snapshot.assignments.len() > MAX_TAGGED_PHOTOS {
            return Err(TagError::TooManyTaggedPhotos);
        }
        for (id, definition) in &snapshot.tags {
            definition.validate()?;
            if *id != definition.id() {
                return Err(TagError::CorruptPersistedData);
            }
        }
        for (photo_id, tags) in &snapshot.assignments {
            if tags.len() > MAX_TAGS_PER_PHOTO {
                return Err(TagError::TooManyTagsForPhoto {
                    photo_id: *photo_id,
                });
            }
            for tag_id in tags {
                if !snapshot.tags.contains_key(tag_id) {
                    return Err(TagError::UnknownTag { tag_id: *tag_id });
                }
            }
        }
        let mut state = Self {
            snapshot,
            paths_by_id: BTreeMap::new(),
            path_index: BTreeMap::new(),
            alias_index: BTreeMap::new(),
            children: BTreeMap::new(),
            tag_photos: BTreeMap::new(),
        };
        state.rebuild_derived()?;
        Ok(state)
    }

    #[must_use]
    pub fn snapshot(&self) -> TagSnapshot {
        self.snapshot.clone()
    }

    /// Imports explicit stable identities and assignments as revision-zero canonical state.
    ///
    /// # Errors
    /// Returns a duplicate, bounded-input, hierarchy, alias, or assignment error.
    pub fn import(
        tags: impl IntoIterator<Item = TagDefinition>,
        assignments: impl IntoIterator<Item = (PhotoId, TagId)>,
    ) -> Result<Self, TagError> {
        let mut tag_map = BTreeMap::new();
        for tag in tags {
            let id = tag.id();
            if tag_map.insert(id, tag).is_some() {
                return Err(TagError::TagIdConflict { tag_id: id });
            }
        }
        let mut assignment_map = BTreeMap::<PhotoId, BTreeSet<TagId>>::new();
        for (photo_id, tag_id) in assignments {
            if !assignment_map.entry(photo_id).or_default().insert(tag_id) {
                return Err(TagError::AssignmentExists { photo_id, tag_id });
            }
        }
        Self::restore(TagSnapshot {
            schema_version: TAG_SCHEMA_VERSION,
            revision: 0,
            tags: tag_map,
            assignments: assignment_map,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        Revision::from_u64(self.snapshot.revision)
    }

    #[must_use]
    pub fn tag(&self, id: TagId) -> Option<&TagDefinition> {
        self.snapshot.tags.get(&id)
    }

    #[must_use]
    pub fn canonical_path(&self, id: TagId) -> Option<&str> {
        self.paths_by_id.get(&id).map(String::as_str)
    }

    #[must_use]
    pub fn resolve(&self, path_or_alias: &str) -> Option<&TagDefinition> {
        let key = canonical_lookup(path_or_alias).ok()?;
        self.path_index
            .get(&key)
            .or_else(|| self.alias_index.get(&key))
            .and_then(|id| self.snapshot.tags.get(id))
    }

    pub fn children(&self, parent_id: Option<TagId>) -> impl Iterator<Item = &TagDefinition> {
        self.children
            .get(&parent_id)
            .into_iter()
            .flatten()
            .filter_map(|id| self.snapshot.tags.get(id))
    }

    pub fn tags(&self) -> impl Iterator<Item = &TagDefinition> {
        self.ordered_tag_ids()
            .into_iter()
            .filter_map(|id| self.snapshot.tags.get(&id))
    }

    pub fn tags_for_photo(&self, photo_id: PhotoId) -> impl Iterator<Item = &TagDefinition> {
        let mut ids = self
            .snapshot
            .assignments
            .get(&photo_id)
            .into_iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        ids.sort_by_key(|id| (self.paths_by_id.get(id), *id));
        ids.into_iter().filter_map(|id| self.snapshot.tags.get(&id))
    }

    /// Returns direct assignments or assignments to the selected hierarchy subtree.
    ///
    /// # Errors
    /// Returns [`TagError::UnknownTag`] when the root does not exist.
    pub fn photos_with_tag(
        &self,
        tag_id: TagId,
        include_descendants: bool,
    ) -> Result<Vec<PhotoId>, TagError> {
        let root = self
            .paths_by_id
            .get(&tag_id)
            .ok_or(TagError::UnknownTag { tag_id })?;
        let mut photos = BTreeSet::new();
        if include_descendants {
            let prefix = format!("{root}|");
            for (candidate_id, path) in &self.paths_by_id {
                if (candidate_id == &tag_id || path.starts_with(&prefix))
                    && let Some(assigned) = self.tag_photos.get(candidate_id)
                {
                    photos.extend(assigned);
                }
            }
        } else if let Some(assigned) = self.tag_photos.get(&tag_id) {
            photos.extend(assigned);
        }
        Ok(photos.into_iter().collect())
    }

    #[must_use]
    pub fn projections(&self) -> Vec<TagProjection> {
        self.ordered_tag_ids()
            .into_iter()
            .filter_map(|id| {
                let tag = self.snapshot.tags.get(&id)?;
                Some(TagProjection {
                    id,
                    parent_id: tag.parent_id(),
                    name: tag.name().display().to_owned(),
                    canonical_path: self.paths_by_id.get(&id)?.clone(),
                    aliases: tag
                        .aliases()
                        .map(|alias| alias.as_str().to_owned())
                        .collect(),
                    direct_photo_count: self.tag_photos.get(&id).map_or(0, BTreeSet::len),
                })
            })
            .collect()
    }

    pub fn assignments(&self) -> impl Iterator<Item = (PhotoId, TagId)> + '_ {
        self.snapshot
            .assignments
            .iter()
            .flat_map(|(photo_id, tags)| tags.iter().map(move |tag_id| (*photo_id, *tag_id)))
    }

    /// Applies a complete command to a clone and publishes it only after all validation succeeds.
    ///
    /// # Errors
    /// Returns a typed revision, hierarchy, conflict, or bounded-input error without mutation.
    pub fn apply(
        &mut self,
        expected: Revision,
        command: TagCommand,
    ) -> Result<TagMutationReceipt, TagError> {
        if expected != self.revision() {
            return Err(TagError::RevisionConflict {
                expected,
                actual: self.revision(),
            });
        }
        let next_revision = expected
            .checked_increment()
            .map_err(|_| TagError::RevisionOverflow)?;
        let mut candidate = self.clone();
        let (photo_ids, tag_ids) = candidate.apply_unchecked(command)?;
        candidate.snapshot.revision = next_revision.get();
        candidate.rebuild_derived()?;
        *self = candidate;
        Ok(TagMutationReceipt {
            revision: next_revision,
            photo_ids,
            tag_ids,
        })
    }

    fn apply_unchecked(
        &mut self,
        command: TagCommand,
    ) -> Result<(Vec<PhotoId>, Vec<TagId>), TagError> {
        match command {
            TagCommand::Create(definition) => {
                definition.validate()?;
                let id = definition.id();
                if self.snapshot.tags.contains_key(&id) {
                    return Err(TagError::TagIdConflict { tag_id: id });
                }
                if self.snapshot.tags.len() == MAX_TAGS {
                    return Err(TagError::TooManyTags);
                }
                self.validate_parent(definition.parent_id())?;
                self.snapshot.tags.insert(id, definition);
                Ok((Vec::new(), vec![id]))
            }
            TagCommand::Update(definition) => {
                definition.validate()?;
                let id = definition.id();
                if !self.snapshot.tags.contains_key(&id) {
                    return Err(TagError::UnknownTag { tag_id: id });
                }
                self.validate_parent(definition.parent_id())?;
                self.snapshot.tags.insert(id, definition);
                Ok((Vec::new(), vec![id]))
            }
            TagCommand::Assign { photo_ids, tag_ids } => {
                let (photo_ids, tag_ids) = self.validate_batch(photo_ids, tag_ids)?;
                for photo_id in &photo_ids {
                    let assigned = self.snapshot.assignments.get(photo_id);
                    for tag_id in &tag_ids {
                        if assigned.is_some_and(|assigned| assigned.contains(tag_id)) {
                            return Err(TagError::AssignmentExists {
                                photo_id: *photo_id,
                                tag_id: *tag_id,
                            });
                        }
                    }
                    if assigned.map_or(0, BTreeSet::len) + tag_ids.len() > MAX_TAGS_PER_PHOTO {
                        return Err(TagError::TooManyTagsForPhoto {
                            photo_id: *photo_id,
                        });
                    }
                }
                let new_photo_count = photo_ids
                    .iter()
                    .filter(|photo_id| !self.snapshot.assignments.contains_key(photo_id))
                    .count();
                if self
                    .snapshot
                    .assignments
                    .len()
                    .saturating_add(new_photo_count)
                    > MAX_TAGGED_PHOTOS
                {
                    return Err(TagError::TooManyTaggedPhotos);
                }
                for photo_id in &photo_ids {
                    self.snapshot
                        .assignments
                        .entry(*photo_id)
                        .or_default()
                        .extend(&tag_ids);
                }
                Ok((photo_ids, tag_ids))
            }
            TagCommand::Remove { photo_ids, tag_ids } => {
                let (photo_ids, tag_ids) = self.validate_batch(photo_ids, tag_ids)?;
                for photo_id in &photo_ids {
                    let assigned = self.snapshot.assignments.get(photo_id);
                    for tag_id in &tag_ids {
                        if !assigned.is_some_and(|assigned| assigned.contains(tag_id)) {
                            return Err(TagError::AssignmentMissing {
                                photo_id: *photo_id,
                                tag_id: *tag_id,
                            });
                        }
                    }
                }
                for photo_id in &photo_ids {
                    let assigned = self
                        .snapshot
                        .assignments
                        .get_mut(photo_id)
                        .expect("assignment validation completed");
                    for tag_id in &tag_ids {
                        assigned.remove(tag_id);
                    }
                    if assigned.is_empty() {
                        self.snapshot.assignments.remove(photo_id);
                    }
                }
                Ok((photo_ids, tag_ids))
            }
        }
    }

    fn validate_parent(&self, parent_id: Option<TagId>) -> Result<(), TagError> {
        if let Some(parent_id) = parent_id
            && !self.snapshot.tags.contains_key(&parent_id)
        {
            return Err(TagError::UnknownParent { parent_id });
        }
        Ok(())
    }

    fn validate_batch(
        &self,
        photo_ids: Vec<PhotoId>,
        tag_ids: Vec<TagId>,
    ) -> Result<(Vec<PhotoId>, Vec<TagId>), TagError> {
        if photo_ids.is_empty() || tag_ids.is_empty() {
            return Err(TagError::EmptyAssignmentBatch);
        }
        if photo_ids.len() > MAX_ASSIGNMENT_PHOTOS || tag_ids.len() > MAX_ASSIGNMENT_TAGS {
            return Err(TagError::AssignmentBatchTooLarge);
        }
        let mut photos = BTreeSet::new();
        for photo_id in photo_ids {
            if !photos.insert(photo_id) {
                return Err(TagError::DuplicatePhotoInBatch { photo_id });
            }
        }
        let mut tags = BTreeSet::new();
        for tag_id in tag_ids {
            if !tags.insert(tag_id) {
                return Err(TagError::DuplicateTagInBatch { tag_id });
            }
        }
        for tag_id in &tags {
            if !self.snapshot.tags.contains_key(tag_id) {
                return Err(TagError::UnknownTag { tag_id: *tag_id });
            }
        }
        Ok((photos.into_iter().collect(), tags.into_iter().collect()))
    }

    fn rebuild_derived(&mut self) -> Result<TagIndexStats, TagError> {
        let mut paths = BTreeMap::new();
        for id in self.snapshot.tags.keys().copied() {
            let mut visiting = BTreeSet::new();
            build_path(id, &self.snapshot.tags, &mut paths, &mut visiting, 0)?;
        }
        let mut path_index = BTreeMap::new();
        let mut children = BTreeMap::<Option<TagId>, Vec<TagId>>::new();
        for (id, definition) in &self.snapshot.tags {
            let path = paths.get(id).expect("all tag paths built").clone();
            if let Some(existing_tag_id) = path_index.insert(path.clone(), *id) {
                return Err(TagError::CanonicalPathConflict {
                    path,
                    existing_tag_id,
                    conflicting_tag_id: *id,
                });
            }
            children
                .entry(definition.parent_id())
                .or_default()
                .push(*id);
        }
        for ids in children.values_mut() {
            ids.sort_by_key(|id| (paths.get(id).cloned(), *id));
        }
        let mut alias_index = BTreeMap::new();
        for (id, definition) in &self.snapshot.tags {
            for alias in definition.aliases() {
                let key = alias.as_str().to_owned();
                if let Some(existing_tag_id) = path_index.get(&key).copied() {
                    return Err(TagError::AliasConflict {
                        alias: key,
                        existing_tag_id,
                        conflicting_tag_id: *id,
                    });
                }
                if let Some(existing_tag_id) = alias_index.insert(key.clone(), *id) {
                    return Err(TagError::AliasConflict {
                        alias: key,
                        existing_tag_id,
                        conflicting_tag_id: *id,
                    });
                }
            }
        }
        let mut tag_photos = BTreeMap::<TagId, BTreeSet<PhotoId>>::new();
        for (photo_id, tag_ids) in &self.snapshot.assignments {
            for tag_id in tag_ids {
                tag_photos.entry(*tag_id).or_default().insert(*photo_id);
            }
        }
        let stats = TagIndexStats {
            canonical_paths: path_index.len(),
            aliases: alias_index.len(),
            assignments: self.snapshot.assignments.values().map(BTreeSet::len).sum(),
        };
        self.paths_by_id = paths;
        self.path_index = path_index;
        self.alias_index = alias_index;
        self.children = children;
        self.tag_photos = tag_photos;
        Ok(stats)
    }

    fn ordered_tag_ids(&self) -> Vec<TagId> {
        self.path_index.values().copied().collect()
    }
}

impl Default for TagState {
    fn default() -> Self {
        Self::new()
    }
}

fn build_path(
    id: TagId,
    tags: &BTreeMap<TagId, TagDefinition>,
    paths: &mut BTreeMap<TagId, String>,
    visiting: &mut BTreeSet<TagId>,
    depth: usize,
) -> Result<String, TagError> {
    if let Some(path) = paths.get(&id) {
        return Ok(path.clone());
    }
    if depth >= MAX_TAG_DEPTH {
        return Err(TagError::HierarchyTooDeep { tag_id: id });
    }
    if !visiting.insert(id) {
        return Err(TagError::HierarchyCycle { tag_id: id });
    }
    let tag = tags.get(&id).ok_or(TagError::UnknownTag { tag_id: id })?;
    let path = if let Some(parent_id) = tag.parent_id() {
        if !tags.contains_key(&parent_id) {
            return Err(TagError::UnknownParent { parent_id });
        }
        format!(
            "{}|{}",
            build_path(parent_id, tags, paths, visiting, depth + 1)?,
            tag.name().canonical()
        )
    } else {
        tag.name().canonical().to_owned()
    };
    visiting.remove(&id);
    paths.insert(id, path.clone());
    Ok(path)
}

mod assignment_serde {
    use std::collections::{BTreeMap, BTreeSet};

    use rusttable_core::PhotoId;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::TagId;

    pub fn serialize<S: Serializer>(
        value: &BTreeMap<PhotoId, BTreeSet<TagId>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .iter()
            .map(|(photo_id, tags)| (photo_id.get(), tags))
            .collect::<BTreeMap<_, _>>()
            .serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<PhotoId, BTreeSet<TagId>>, D::Error> {
        BTreeMap::<u128, BTreeSet<TagId>>::deserialize(deserializer)?
            .into_iter()
            .map(|(value, tags)| {
                PhotoId::new(value)
                    .map(|photo_id| (photo_id, tags))
                    .ok_or_else(|| serde::de::Error::custom("photo ID cannot be zero"))
            })
            .collect()
    }
}
