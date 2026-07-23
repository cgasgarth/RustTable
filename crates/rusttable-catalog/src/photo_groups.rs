use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use rusttable_core::PhotoId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhotoGroupId(u128);

impl PhotoGroupId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for PhotoGroupId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoGroup {
    id: PhotoGroupId,
    members: Vec<PhotoId>,
    representative: Option<PhotoId>,
}

impl PhotoGroup {
    /// Creates a group with canonical member ordering and one representative
    /// whenever the group is non-empty.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for duplicate members or an invalid representative.
    pub fn new(
        id: PhotoGroupId,
        members: impl IntoIterator<Item = PhotoId>,
        representative: Option<PhotoId>,
    ) -> Result<Self, PhotoGroupError> {
        let mut members = members.into_iter().collect::<Vec<_>>();
        members.sort_unstable();
        if let Some(photo_id) = members
            .windows(2)
            .find_map(|window| (window[0] == window[1]).then_some(window[0]))
        {
            return Err(PhotoGroupError::DuplicateMember { photo_id });
        }
        if members.is_empty() {
            if representative.is_some() {
                return Err(PhotoGroupError::EmptyGroupRepresentative);
            }
        } else if representative.is_none_or(|photo_id| members.binary_search(&photo_id).is_err()) {
            return Err(PhotoGroupError::RepresentativeNotMember { representative });
        }
        Ok(Self {
            id,
            members,
            representative,
        })
    }

    #[must_use]
    pub fn empty(id: PhotoGroupId) -> Self {
        Self {
            id,
            members: Vec::new(),
            representative: None,
        }
    }

    #[must_use]
    pub const fn id(&self) -> PhotoGroupId {
        self.id
    }

    #[must_use]
    pub fn members(&self) -> &[PhotoId] {
        &self.members
    }

    #[must_use]
    pub const fn representative(&self) -> Option<PhotoId> {
        self.representative
    }

    /// Adds members while preserving canonical ordering and the invariant.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for duplicate or invalid members.
    pub fn add_members(
        &self,
        members: impl IntoIterator<Item = PhotoId>,
    ) -> Result<Self, PhotoGroupError> {
        let mut next_members = self.members.clone();
        next_members.extend(members);
        let representative = self
            .representative
            .or_else(|| next_members.iter().copied().min());
        Self::new(self.id, next_members, representative)
    }

    /// Removes members and deterministically promotes a replacement when needed.
    ///
    /// # Errors
    ///
    /// Returns an invariant error for an empty or unknown membership change.
    pub fn remove_members(
        &self,
        members: impl IntoIterator<Item = PhotoId>,
    ) -> Result<Self, PhotoGroupError> {
        let mut unique_members = BTreeSet::new();
        for photo_id in members {
            if !unique_members.insert(photo_id) {
                return Err(PhotoGroupError::DuplicateMember { photo_id });
            }
        }
        let members = unique_members;
        if members.is_empty() {
            return Err(PhotoGroupError::EmptyMembershipChange);
        }
        for photo_id in &members {
            if self.members.binary_search(photo_id).is_err() {
                return Err(PhotoGroupError::MemberNotFound {
                    photo_id: *photo_id,
                });
            }
        }
        let remaining = self
            .members
            .iter()
            .copied()
            .filter(|photo_id| !members.contains(photo_id))
            .collect::<Vec<_>>();
        let representative = if remaining.is_empty() {
            None
        } else if self
            .representative
            .is_some_and(|photo_id| members.contains(&photo_id))
        {
            remaining.first().copied()
        } else {
            self.representative
        };
        Self::new(self.id, remaining, representative)
    }

    /// Selects one existing member as the explicit representative.
    ///
    /// # Errors
    ///
    /// Returns an error when the selected photo is not a member.
    pub fn set_representative(&self, representative: PhotoId) -> Result<Self, PhotoGroupError> {
        Self::new(self.id, self.members.clone(), Some(representative))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoGroupError {
    DuplicateGroup {
        group_id: PhotoGroupId,
    },
    GroupNotFound {
        group_id: PhotoGroupId,
    },
    DuplicateMember {
        photo_id: PhotoId,
    },
    EmptyMembershipChange,
    MemberNotFound {
        photo_id: PhotoId,
    },
    UnknownPhoto {
        photo_id: PhotoId,
    },
    PhotoAlreadyGrouped {
        photo_id: PhotoId,
        group_id: PhotoGroupId,
    },
    RepresentativeNotMember {
        representative: Option<PhotoId>,
    },
    EmptyGroupRepresentative,
}

impl fmt::Display for PhotoGroupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid photo group: {self:?}")
    }
}

impl std::error::Error for PhotoGroupError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhotoGroupCommand {
    Create {
        group_id: PhotoGroupId,
        photo_ids: Vec<PhotoId>,
        representative: Option<PhotoId>,
    },
    AddMembers {
        group_id: PhotoGroupId,
        photo_ids: Vec<PhotoId>,
    },
    RemoveMembers {
        group_id: PhotoGroupId,
        photo_ids: Vec<PhotoId>,
    },
    SetRepresentative {
        group_id: PhotoGroupId,
        photo_id: PhotoId,
    },
    Delete {
        group_id: PhotoGroupId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhotoGroupProjection {
    pub group_id: PhotoGroupId,
    pub member_ids: Vec<PhotoId>,
    pub representative: Option<PhotoId>,
}

impl From<&PhotoGroup> for PhotoGroupProjection {
    fn from(group: &PhotoGroup) -> Self {
        Self {
            group_id: group.id,
            member_ids: group.members.clone(),
            representative: group.representative,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PhotoGroupState {
    groups: BTreeMap<PhotoGroupId, PhotoGroup>,
    member_index: BTreeMap<PhotoId, PhotoGroupId>,
}

impl PhotoGroupState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds the registry from authoritative group rows.
    ///
    /// # Errors
    ///
    /// Returns an error when group identities or memberships conflict.
    pub fn from_groups(
        groups: impl IntoIterator<Item = PhotoGroup>,
    ) -> Result<Self, PhotoGroupError> {
        let mut state = Self::new();
        for group in groups {
            if state.groups.contains_key(&group.id()) {
                return Err(PhotoGroupError::DuplicateGroup {
                    group_id: group.id(),
                });
            }
            for photo_id in group.members() {
                if let Some(group_id) = state.member_index.insert(*photo_id, group.id()) {
                    return Err(PhotoGroupError::PhotoAlreadyGrouped {
                        photo_id: *photo_id,
                        group_id,
                    });
                }
            }
            state.groups.insert(group.id(), group);
        }
        Ok(state)
    }

    #[must_use]
    pub fn group(&self, group_id: PhotoGroupId) -> Option<&PhotoGroup> {
        self.groups.get(&group_id)
    }

    pub fn groups(&self) -> impl Iterator<Item = &PhotoGroup> {
        self.groups.values()
    }

    #[must_use]
    pub fn group_for_photo(&self, photo_id: PhotoId) -> Option<PhotoGroupId> {
        self.member_index.get(&photo_id).copied()
    }

    #[must_use]
    pub fn projections(&self) -> Vec<PhotoGroupProjection> {
        self.groups
            .values()
            .map(PhotoGroupProjection::from)
            .collect()
    }

    /// Applies one explicit group command against the known catalog photos.
    ///
    /// # Errors
    ///
    /// Returns a validation error without changing this registry.
    pub fn apply(
        &mut self,
        command: PhotoGroupCommand,
        known_photos: &BTreeSet<PhotoId>,
    ) -> Result<Vec<PhotoId>, PhotoGroupError> {
        let mut next = self.clone();
        let changed = next.apply_unchecked(command, known_photos)?;
        *self = next;
        Ok(changed)
    }

    fn apply_unchecked(
        &mut self,
        command: PhotoGroupCommand,
        known_photos: &BTreeSet<PhotoId>,
    ) -> Result<Vec<PhotoId>, PhotoGroupError> {
        match command {
            PhotoGroupCommand::Create {
                group_id,
                photo_ids,
                representative,
            } => {
                if self.groups.contains_key(&group_id) {
                    return Err(PhotoGroupError::DuplicateGroup { group_id });
                }
                self.validate_create_members(&photo_ids, known_photos)?;
                let group = PhotoGroup::new(group_id, photo_ids, representative)?;
                let changed = group.members.clone();
                self.insert(group);
                Ok(changed)
            }
            PhotoGroupCommand::AddMembers {
                group_id,
                photo_ids,
            } => {
                let group = self
                    .groups
                    .get(&group_id)
                    .ok_or(PhotoGroupError::GroupNotFound { group_id })?;
                self.validate_add_members(&photo_ids, known_photos, group)?;
                let updated = group.add_members(photo_ids)?;
                let changed = updated.members.clone();
                self.replace(updated);
                Ok(changed)
            }
            PhotoGroupCommand::RemoveMembers {
                group_id,
                photo_ids,
            } => {
                let group = self
                    .groups
                    .get(&group_id)
                    .ok_or(PhotoGroupError::GroupNotFound { group_id })?;
                let updated = group.remove_members(photo_ids)?;
                let changed = updated.members.clone();
                self.replace(updated);
                Ok(changed)
            }
            PhotoGroupCommand::SetRepresentative { group_id, photo_id } => {
                let group = self
                    .groups
                    .get(&group_id)
                    .ok_or(PhotoGroupError::GroupNotFound { group_id })?;
                let updated = group.set_representative(photo_id)?;
                let changed = updated.members.clone();
                self.replace(updated);
                Ok(changed)
            }
            PhotoGroupCommand::Delete { group_id } => {
                let group = self
                    .groups
                    .remove(&group_id)
                    .ok_or(PhotoGroupError::GroupNotFound { group_id })?;
                for photo_id in &group.members {
                    self.member_index.remove(photo_id);
                }
                Ok(group.members)
            }
        }
    }

    fn validate_create_members(
        &self,
        photo_ids: &[PhotoId],
        known_photos: &BTreeSet<PhotoId>,
    ) -> Result<(), PhotoGroupError> {
        let mut unique = BTreeSet::new();
        for photo_id in photo_ids {
            if !unique.insert(*photo_id) {
                return Err(PhotoGroupError::DuplicateMember {
                    photo_id: *photo_id,
                });
            }
            if !known_photos.contains(photo_id) {
                return Err(PhotoGroupError::UnknownPhoto {
                    photo_id: *photo_id,
                });
            }
            if let Some(group_id) = self.member_index.get(photo_id).copied() {
                return Err(PhotoGroupError::PhotoAlreadyGrouped {
                    photo_id: *photo_id,
                    group_id,
                });
            }
        }
        Ok(())
    }

    fn validate_add_members(
        &self,
        photo_ids: &[PhotoId],
        known_photos: &BTreeSet<PhotoId>,
        group: &PhotoGroup,
    ) -> Result<(), PhotoGroupError> {
        if photo_ids.is_empty() {
            return Err(PhotoGroupError::EmptyMembershipChange);
        }
        let mut unique = BTreeSet::new();
        for photo_id in photo_ids {
            if !unique.insert(*photo_id) {
                return Err(PhotoGroupError::DuplicateMember {
                    photo_id: *photo_id,
                });
            }
            if !known_photos.contains(photo_id) {
                return Err(PhotoGroupError::UnknownPhoto {
                    photo_id: *photo_id,
                });
            }
            if group.members.binary_search(photo_id).is_ok() {
                return Err(PhotoGroupError::DuplicateMember {
                    photo_id: *photo_id,
                });
            }
            if let Some(existing_group_id) = self.member_index.get(photo_id).copied() {
                return Err(PhotoGroupError::PhotoAlreadyGrouped {
                    photo_id: *photo_id,
                    group_id: existing_group_id,
                });
            }
        }
        Ok(())
    }

    fn insert(&mut self, group: PhotoGroup) {
        for photo_id in group.members() {
            self.member_index.insert(*photo_id, group.id());
        }
        self.groups.insert(group.id(), group);
    }

    fn replace(&mut self, group: PhotoGroup) {
        let old = self.groups.get(&group.id()).expect("group exists");
        for photo_id in old.members() {
            self.member_index.remove(photo_id);
        }
        self.insert(group);
    }
}
