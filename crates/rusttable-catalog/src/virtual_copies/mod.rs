use std::collections::BTreeMap;
use std::fmt;
use std::num::NonZeroU128;

use rusttable_core::{AssetId, Edit, EditId, Operation, PhotoId, Revision};

/// Durable format version for the virtual-copy aggregate.
pub const VIRTUAL_COPY_FORMAT_VERSION: u8 = 1;

/// Identifies a catalog virtual copy independently of its source asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VirtualCopyId(NonZeroU128);

impl VirtualCopyId {
    #[must_use]
    pub const fn new(value: u128) -> Option<Self> {
        match NonZeroU128::new(value) {
            Some(value) => Some(Self(value)),
            None => None,
        }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0.get()
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// This cannot panic because the source ID is constructed from the same
    /// nonzero invariant as [`VirtualCopyId`].
    pub fn photo_id(self) -> PhotoId {
        // The virtual-copy edit/history namespace uses the same fixed-width ID
        // representation as native edits while the aggregate ID remains typed.
        PhotoId::new(self.get()).expect("VirtualCopyId is nonzero")
    }
}

impl fmt::Display for VirtualCopyId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.get())
    }
}

/// The immutable source anchor shared by one or more virtual copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceAssetIdentity {
    photo_id: PhotoId,
    asset_id: AssetId,
}

impl SourceAssetIdentity {
    #[must_use]
    pub const fn new(photo_id: PhotoId, asset_id: AssetId) -> Self {
        Self { photo_id, asset_id }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn asset_id(self) -> AssetId {
        self.asset_id
    }
}

/// One virtual copy with its own current edit and immutable edit history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualCopy {
    id: VirtualCopyId,
    source: SourceAssetIdentity,
    order: u64,
    deleted: bool,
    current_edit: Edit,
    history: Vec<Edit>,
}

impl VirtualCopy {
    /// Creates a live virtual copy at edit revision zero.
    ///
    /// # Errors
    ///
    /// Returns an error when the edit identity or initial history is invalid.
    pub fn new(
        id: VirtualCopyId,
        source: SourceAssetIdentity,
        order: u64,
        edit: Edit,
    ) -> Result<Self, VirtualCopyError> {
        let copy = Self {
            id,
            source,
            order,
            deleted: false,
            current_edit: edit.clone(),
            history: vec![edit],
        };
        copy.validate()?;
        Ok(copy)
    }

    /// Reconstructs a copy from durable current and history values.
    ///
    /// # Errors
    ///
    /// Returns an error when the current edit and history do not form a
    /// contiguous immutable revision chain for this copy.
    pub fn from_parts(
        id: VirtualCopyId,
        source: SourceAssetIdentity,
        order: u64,
        deleted: bool,
        current_edit: Edit,
        history: Vec<Edit>,
    ) -> Result<Self, VirtualCopyError> {
        let copy = Self {
            id,
            source,
            order,
            deleted,
            current_edit,
            history,
        };
        copy.validate()?;
        Ok(copy)
    }

    #[must_use]
    pub const fn id(&self) -> VirtualCopyId {
        self.id
    }

    #[must_use]
    pub const fn source(&self) -> SourceAssetIdentity {
        self.source
    }

    #[must_use]
    pub const fn order(&self) -> u64 {
        self.order
    }

    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.deleted
    }

    #[must_use]
    pub const fn current_edit(&self) -> &Edit {
        &self.current_edit
    }

    #[must_use]
    pub fn history(&self) -> impl ExactSizeIterator<Item = &Edit> {
        self.history.iter()
    }

    /// Replaces the current edit and appends exactly one independent revision.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale revision or a replacement that skips or
    /// changes the copy's edit identity.
    pub fn replace_edit(
        &mut self,
        expected_revision: Revision,
        replacement: Edit,
    ) -> Result<(), VirtualCopyError> {
        self.validate_replacement(expected_revision, &replacement)?;
        self.current_edit = replacement.clone();
        self.history.push(replacement);
        Ok(())
    }

    /// Builds and commits a revision from a replacement operation sequence.
    ///
    /// # Errors
    ///
    /// Returns an error when the expected revision is stale, the operation
    /// sequence is invalid, or the revision would overflow.
    pub fn revise<I>(
        &mut self,
        expected_revision: Revision,
        operations: I,
    ) -> Result<(), VirtualCopyError>
    where
        I: IntoIterator<Item = Operation>,
    {
        let replacement = self
            .current_edit
            .revised(operations)
            .map_err(|_| VirtualCopyError::EditRevisionOverflow { id: self.id })?;
        self.replace_edit(expected_revision, replacement)
    }

    fn validate(&self) -> Result<(), VirtualCopyError> {
        let expected_photo_id = self.id.photo_id();
        if self.current_edit.photo_id() != expected_photo_id {
            return Err(VirtualCopyError::EditPhotoMismatch {
                id: self.id,
                expected: expected_photo_id,
                actual: self.current_edit.photo_id(),
            });
        }
        if self.history.is_empty() {
            return Err(VirtualCopyError::EmptyHistory { id: self.id });
        }
        let first = self.history.first().expect("nonempty history");
        if first.revision() != Revision::ZERO
            || first.id() != self.current_edit.id()
            || first.photo_id() != expected_photo_id
        {
            return Err(VirtualCopyError::InvalidHistory { id: self.id });
        }
        for pair in self.history.windows(2) {
            if pair[1].id() != first.id()
                || pair[1].photo_id() != expected_photo_id
                || pair[1].base_photo_revision() != first.base_photo_revision()
                || pair[0].revision().checked_increment() != Ok(pair[1].revision())
            {
                return Err(VirtualCopyError::InvalidHistory { id: self.id });
            }
        }
        if self.history.last() != Some(&self.current_edit) {
            return Err(VirtualCopyError::CurrentEditNotLast { id: self.id });
        }
        Ok(())
    }

    fn validate_replacement(
        &self,
        expected_revision: Revision,
        replacement: &Edit,
    ) -> Result<(), VirtualCopyError> {
        if expected_revision != self.current_edit.revision() {
            return Err(VirtualCopyError::EditRevisionConflict {
                id: self.id,
                expected: expected_revision,
                actual: self.current_edit.revision(),
            });
        }
        if replacement.id() != self.current_edit.id()
            || replacement.photo_id() != self.current_edit.photo_id()
            || replacement.base_photo_revision() != self.current_edit.base_photo_revision()
        {
            return Err(VirtualCopyError::InvalidReplacement { id: self.id });
        }
        let expected = self
            .current_edit
            .revision()
            .checked_increment()
            .map_err(|_| VirtualCopyError::EditRevisionOverflow { id: self.id })?;
        if replacement.revision() != expected {
            return Err(VirtualCopyError::InvalidRevisionAdvance {
                id: self.id,
                expected,
                actual: replacement.revision(),
            });
        }
        Ok(())
    }

    fn set_order(&mut self, order: u64) {
        self.order = order;
    }

    fn set_deleted(&mut self, deleted: bool) {
        self.deleted = deleted;
    }
}

/// A stable, UI-independent projection of one virtual copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VirtualCopyProjection {
    id: VirtualCopyId,
    source: SourceAssetIdentity,
    order: u64,
    deleted: bool,
    current_edit_id: EditId,
    current_edit_revision: Revision,
    history_len: usize,
}

impl VirtualCopyProjection {
    #[must_use]
    pub const fn id(self) -> VirtualCopyId {
        self.id
    }
    #[must_use]
    pub const fn source(self) -> SourceAssetIdentity {
        self.source
    }
    #[must_use]
    pub const fn order(self) -> u64 {
        self.order
    }
    #[must_use]
    pub const fn is_deleted(self) -> bool {
        self.deleted
    }
    #[must_use]
    pub const fn current_edit_id(self) -> EditId {
        self.current_edit_id
    }
    #[must_use]
    pub const fn current_edit_revision(self) -> Revision {
        self.current_edit_revision
    }
    #[must_use]
    pub const fn history_len(self) -> usize {
        self.history_len
    }
}

/// Commands that mutate virtual-copy identity, edit history, ordering, or deletion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VirtualCopyCommand {
    Create(VirtualCopy),
    ReplaceEdit {
        id: VirtualCopyId,
        expected_revision: Revision,
        replacement: Edit,
    },
    Reorder {
        id: VirtualCopyId,
        before: Option<VirtualCopyId>,
    },
    Delete {
        id: VirtualCopyId,
    },
    Restore {
        id: VirtualCopyId,
    },
}

/// In-memory virtual-copy aggregate used by catalog services and durable adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualCopyCatalog {
    revision: Revision,
    copies: BTreeMap<VirtualCopyId, VirtualCopy>,
}

impl VirtualCopyCatalog {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            revision: Revision::ZERO,
            copies: BTreeMap::new(),
        }
    }

    /// Rebuilds deterministic state from durable copies.
    ///
    /// # Errors
    ///
    /// Returns an error when a virtual-copy identity is duplicated.
    pub fn from_parts(
        revision: Revision,
        copies: impl IntoIterator<Item = VirtualCopy>,
    ) -> Result<Self, VirtualCopyError> {
        let mut values = BTreeMap::new();
        for copy in copies {
            if values.insert(copy.id(), copy).is_some() {
                return Err(VirtualCopyError::DuplicateId);
            }
        }
        Ok(Self {
            revision,
            copies: values,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub fn copy(&self, id: VirtualCopyId) -> Option<&VirtualCopy> {
        self.copies.get(&id)
    }

    pub fn copies(&self) -> impl Iterator<Item = &VirtualCopy> {
        self.sorted_ids()
            .into_iter()
            .filter_map(|id| self.copies.get(&id))
    }

    /// Returns active copies in order, with ID as the deterministic tie-breaker.
    pub fn projections(&self) -> Vec<VirtualCopyProjection> {
        self.copies()
            .filter(|copy| !copy.is_deleted())
            .map(projection)
            .collect()
    }

    /// Returns all copies, including deletion tombstones, in stable order.
    pub fn all_projections(&self) -> Vec<VirtualCopyProjection> {
        self.copies().map(projection).collect()
    }

    /// Applies one command atomically at the expected catalog revision.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale catalog revision, invalid command, or
    /// checked revision overflow; the aggregate is unchanged on error.
    pub fn apply(
        &mut self,
        expected: Revision,
        command: VirtualCopyCommand,
    ) -> Result<Revision, VirtualCopyError> {
        if expected != self.revision {
            return Err(VirtualCopyError::CatalogRevisionConflict {
                expected,
                actual: self.revision,
            });
        }
        let mut next = self.clone();
        next.apply_inner(command)?;
        next.revision = self
            .revision
            .checked_increment()
            .map_err(|_| VirtualCopyError::CatalogRevisionOverflow)?;
        *self = next;
        Ok(self.revision)
    }

    fn apply_inner(&mut self, command: VirtualCopyCommand) -> Result<(), VirtualCopyError> {
        match command {
            VirtualCopyCommand::Create(copy) => {
                if self.copies.contains_key(&copy.id()) {
                    return Err(VirtualCopyError::DuplicateId);
                }
                self.copies.insert(copy.id(), copy);
            }
            VirtualCopyCommand::ReplaceEdit {
                id,
                expected_revision,
                replacement,
            } => {
                self.copy_mut(id)?
                    .replace_edit(expected_revision, replacement)?;
            }
            VirtualCopyCommand::Delete { id } => self.copy_mut(id)?.set_deleted(true),
            VirtualCopyCommand::Restore { id } => self.copy_mut(id)?.set_deleted(false),
            VirtualCopyCommand::Reorder { id, before } => self.reorder(id, before)?,
        }
        Ok(())
    }

    fn reorder(
        &mut self,
        id: VirtualCopyId,
        before: Option<VirtualCopyId>,
    ) -> Result<(), VirtualCopyError> {
        if self.copy_result(id)?.is_deleted() {
            return Err(VirtualCopyError::DeletedCopy { id });
        }
        if let Some(before) = before {
            if before == id {
                return Err(VirtualCopyError::InvalidOrderTarget);
            }
            if self.copy_result(before)?.is_deleted() {
                return Err(VirtualCopyError::DeletedCopy { id: before });
            }
        }
        let mut ordered = self
            .copies()
            .filter(|copy| !copy.is_deleted())
            .map(VirtualCopy::id)
            .collect::<Vec<_>>();
        ordered.retain(|candidate| *candidate != id);
        let position = before.map_or(ordered.len(), |target| {
            ordered
                .iter()
                .position(|candidate| *candidate == target)
                .expect("validated order target")
        });
        ordered.insert(position, id);
        for (order, copy_id) in ordered.into_iter().enumerate() {
            self.copy_mut(copy_id)?.set_order(
                u64::try_from(order).map_err(|_| VirtualCopyError::CatalogRevisionOverflow)?,
            );
        }
        Ok(())
    }

    fn sorted_ids(&self) -> Vec<VirtualCopyId> {
        let mut ids = self.copies.keys().copied().collect::<Vec<_>>();
        ids.sort_by_key(|id| {
            let copy = self.copies.get(id).expect("ID came from copies");
            (copy.is_deleted(), copy.order(), *id)
        });
        ids
    }

    fn copy_result(&self, id: VirtualCopyId) -> Result<&VirtualCopy, VirtualCopyError> {
        self.copies
            .get(&id)
            .ok_or(VirtualCopyError::UnknownId { id })
    }

    fn copy_mut(&mut self, id: VirtualCopyId) -> Result<&mut VirtualCopy, VirtualCopyError> {
        self.copies
            .get_mut(&id)
            .ok_or(VirtualCopyError::UnknownId { id })
    }
}

impl Default for VirtualCopyCatalog {
    fn default() -> Self {
        Self::new()
    }
}

fn projection(copy: &VirtualCopy) -> VirtualCopyProjection {
    VirtualCopyProjection {
        id: copy.id,
        source: copy.source,
        order: copy.order,
        deleted: copy.deleted,
        current_edit_id: copy.current_edit.id(),
        current_edit_revision: copy.current_edit.revision(),
        history_len: copy.history.len(),
    }
}

/// Validation and optimistic-concurrency errors for virtual copies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualCopyError {
    CatalogRevisionConflict {
        expected: Revision,
        actual: Revision,
    },
    CatalogRevisionOverflow,
    DuplicateId,
    UnknownId {
        id: VirtualCopyId,
    },
    DeletedCopy {
        id: VirtualCopyId,
    },
    InvalidOrderTarget,
    EditPhotoMismatch {
        id: VirtualCopyId,
        expected: PhotoId,
        actual: PhotoId,
    },
    EmptyHistory {
        id: VirtualCopyId,
    },
    InvalidHistory {
        id: VirtualCopyId,
    },
    CurrentEditNotLast {
        id: VirtualCopyId,
    },
    EditRevisionConflict {
        id: VirtualCopyId,
        expected: Revision,
        actual: Revision,
    },
    InvalidReplacement {
        id: VirtualCopyId,
    },
    InvalidRevisionAdvance {
        id: VirtualCopyId,
        expected: Revision,
        actual: Revision,
    },
    EditRevisionOverflow {
        id: VirtualCopyId,
    },
}

impl fmt::Display for VirtualCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "virtual-copy operation failed: {self:?}")
    }
}

impl std::error::Error for VirtualCopyError {}

/// Storage boundary for the virtual-copy aggregate.
pub trait VirtualCopyRepository {
    /// Loads all current virtual-copy state.
    ///
    /// # Errors
    ///
    /// Returns a storage or corruption error.
    fn load(&self) -> Result<VirtualCopyCatalog, VirtualCopyRepositoryError>;

    /// Applies one command atomically at the expected catalog revision.
    ///
    /// # Errors
    ///
    /// Returns a storage, source-anchor, validation, or optimistic-concurrency error.
    fn apply(
        &mut self,
        expected: Revision,
        command: VirtualCopyCommand,
    ) -> Result<Revision, VirtualCopyRepositoryError>;
}

/// Stable error categories exposed by virtual-copy persistence adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VirtualCopyRepositoryError {
    Unavailable,
    Conflict,
    Corrupt,
    CommitFailed,
    SourceAssetNotFound { source: SourceAssetIdentity },
    SourceAssetMismatch { source: SourceAssetIdentity },
    EditIdConflict { edit_id: EditId },
    Domain(VirtualCopyError),
}

impl fmt::Display for VirtualCopyRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "virtual-copy repository operation failed: {self:?}"
        )
    }
}

impl std::error::Error for VirtualCopyRepositoryError {}
