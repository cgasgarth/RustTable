use std::collections::{BTreeMap, BTreeSet};

use rusttable_core::PhotoId;

use super::command::HistoryCommand;
use super::error::HistoryError;
use super::types::{
    BranchTransferPolicy, HistoryBranch, HistoryBranchId, HistoryCursor, HistoryEvidence,
    HistoryPayload, HistoryRevision, HistoryRevisionId, HistorySnapshot, HistorySnapshotId,
    HistoryStateSnapshot, HistoryVersion, validate_name,
};

/// Immutable edit-history graph and mutable cursors/branch metadata for one photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryState {
    photo_id: PhotoId,
    version: HistoryVersion,
    next_revision_id: u64,
    next_branch_id: u64,
    next_snapshot_id: u64,
    active_branch: HistoryBranchId,
    revisions: BTreeMap<HistoryRevisionId, HistoryRevision>,
    branches: BTreeMap<HistoryBranchId, HistoryBranch>,
    snapshots: BTreeMap<HistorySnapshotId, HistorySnapshot>,
    evidence: BTreeSet<HistoryEvidence>,
}

impl HistoryState {
    /// Creates an empty history with a durable `main` branch.
    ///
    /// # Panics
    ///
    /// This cannot panic unless the literal main-branch ID is changed to zero.
    #[must_use]
    pub fn new(photo_id: PhotoId) -> Self {
        let main = HistoryBranchId::new(1).expect("literal branch ID is nonzero");
        let mut branches = BTreeMap::new();
        branches.insert(
            main,
            HistoryBranch::new(main, "main".to_owned(), None, Vec::new(), None),
        );
        Self {
            photo_id,
            version: HistoryVersion::ZERO,
            next_revision_id: 1,
            next_branch_id: 2,
            next_snapshot_id: 1,
            active_branch: main,
            revisions: BTreeMap::new(),
            branches,
            snapshots: BTreeMap::new(),
            evidence: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn version(&self) -> HistoryVersion {
        self.version
    }

    #[must_use]
    pub const fn active_branch_id(&self) -> HistoryBranchId {
        self.active_branch
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Panics only if an internal mutation violates the active-branch invariant.
    pub fn active_cursor(&self) -> HistoryCursor {
        // The active branch invariant is checked by `restore` and maintained by
        // every command, so an absent branch indicates an internal bug.
        let branch = self
            .branches
            .get(&self.active_branch)
            .expect("active branch is validated at construction");
        HistoryCursor::new(branch.id(), branch.cursor())
    }

    #[must_use]
    pub fn branch(&self, id: HistoryBranchId) -> Option<&HistoryBranch> {
        self.branches.get(&id)
    }

    pub fn branches(&self) -> impl Iterator<Item = &HistoryBranch> {
        self.branches.values()
    }

    #[must_use]
    pub fn revision(&self, id: HistoryRevisionId) -> Option<&HistoryRevision> {
        self.revisions.get(&id)
    }

    pub fn revisions(&self) -> impl Iterator<Item = &HistoryRevision> {
        self.revisions.values()
    }

    #[must_use]
    pub fn snapshot(&self, id: HistorySnapshotId) -> Option<&HistorySnapshot> {
        self.snapshots.get(&id)
    }

    pub fn snapshots(&self) -> impl Iterator<Item = &HistorySnapshot> {
        self.snapshots.values()
    }

    pub fn evidence(&self) -> impl Iterator<Item = HistoryEvidence> + '_ {
        self.evidence.iter().copied()
    }

    #[must_use]
    pub fn current_revision(&self) -> Option<&HistoryRevision> {
        self.active_cursor()
            .revision()
            .and_then(|id| self.revisions.get(&id))
    }

    /// Applies one optimistic command to a clone and publishes it atomically.
    ///
    /// Revisions and their payloads are never modified after insertion. Commands
    /// only append a revision or move/delete metadata pointers.
    ///
    /// # Errors
    ///
    /// Returns a typed stale-command, validation, or checked-counter error.
    pub fn apply(
        &mut self,
        expected: HistoryVersion,
        command: HistoryCommand,
    ) -> Result<HistoryApplyOutcome, HistoryError> {
        if expected != self.version {
            return Err(HistoryError::VersionConflict {
                expected,
                actual: self.version,
            });
        }
        let mut next = self.clone();
        let outcome = next.apply_mut(command)?;
        next.version = self.version.next().ok_or(HistoryError::RevisionOverflow)?;
        *self = next;
        Ok(outcome)
    }

    /// Exports a validated persistence snapshot for the redb adapter.
    #[must_use]
    pub fn persistence_snapshot(&self) -> HistoryStateSnapshot {
        HistoryStateSnapshot {
            photo_id: self.photo_id,
            version: self.version,
            next_revision_id: self.next_revision_id,
            next_branch_id: self.next_branch_id,
            next_snapshot_id: self.next_snapshot_id,
            active_branch: self.active_branch,
            revisions: self.revisions.values().cloned().collect(),
            branches: self.branches.values().cloned().collect(),
            snapshots: self.snapshots.values().cloned().collect(),
            evidence: self.evidence.iter().copied().collect(),
        }
    }

    /// Rebuilds state from a persisted snapshot, rejecting dangling references.
    ///
    /// # Errors
    ///
    /// Returns an error when any persisted ID, cursor, parent, branch, snapshot,
    /// evidence, or photo identity is invalid.
    pub fn restore(snapshot: HistoryStateSnapshot) -> Result<Self, HistoryError> {
        let HistoryStateSnapshot {
            photo_id,
            version,
            next_revision_id,
            next_branch_id,
            next_snapshot_id,
            active_branch,
            revisions: persisted_revisions,
            branches: persisted_branches,
            snapshots: persisted_snapshots,
            evidence: persisted_evidence,
        } = snapshot;
        let persisted_revision_count = persisted_revisions.len();
        let revisions = persisted_revisions
            .into_iter()
            .map(|revision| (revision.id(), revision))
            .collect::<BTreeMap<_, _>>();
        if revisions.len() != persisted_revision_count
            || revisions.values().any(|revision| {
                revision.payload().edit().photo_id() != photo_id
                    || revision
                        .parent()
                        .is_some_and(|parent| !revisions.contains_key(&parent))
            })
        {
            return Err(HistoryError::InvalidPersistedState);
        }

        let persisted_branch_count = persisted_branches.len();
        let branches = persisted_branches
            .into_iter()
            .map(|branch| (branch.id(), branch))
            .collect::<BTreeMap<_, _>>();
        if branches.len() != persisted_branch_count
            || branches.is_empty()
            || !branches.contains_key(&active_branch)
            || branches.values().any(|branch| {
                !validate_name(branch.name())
                    || branch
                        .lineage()
                        .iter()
                        .any(|revision| !revisions.contains_key(revision))
                    || branch.cursor().is_some_and(|cursor| {
                        !branch.lineage().contains(&cursor) || !revisions.contains_key(&cursor)
                    })
                    || branch
                        .redo()
                        .iter()
                        .any(|revision| !revisions.contains_key(revision))
            })
        {
            return Err(HistoryError::InvalidPersistedState);
        }

        let persisted_snapshot_count = persisted_snapshots.len();
        let snapshots = persisted_snapshots
            .into_iter()
            .map(|value| (value.id(), value))
            .collect::<BTreeMap<_, _>>();
        if snapshots.len() != persisted_snapshot_count
            || snapshots.values().any(|value| {
                !validate_name(value.name())
                    || branches.get(&value.cursor().branch()).is_none_or(|branch| {
                        value.cursor().revision().is_some_and(|revision| {
                            !branch.lineage().contains(&revision)
                                || !revisions.contains_key(&revision)
                        })
                    })
            })
        {
            return Err(HistoryError::InvalidPersistedState);
        }

        let persisted_evidence_count = persisted_evidence.len();
        let evidence = persisted_evidence.into_iter().collect::<BTreeSet<_>>();
        if evidence.len() != persisted_evidence_count
            || evidence
                .iter()
                .any(|value| !revisions.contains_key(&value.revision()))
        {
            return Err(HistoryError::InvalidPersistedState);
        }

        let max_revision = revisions.keys().map(|id| id.get()).max().unwrap_or(0);
        let max_branch = branches.keys().map(|id| id.get()).max().unwrap_or(0);
        let max_snapshot = snapshots.keys().map(|id| id.get()).max().unwrap_or(0);
        if next_revision_id <= max_revision
            || next_branch_id <= max_branch
            || next_snapshot_id <= max_snapshot
        {
            return Err(HistoryError::InvalidPersistedState);
        }
        Ok(Self {
            photo_id,
            version,
            next_revision_id,
            next_branch_id,
            next_snapshot_id,
            active_branch,
            revisions,
            branches,
            snapshots,
            evidence,
        })
    }

    fn apply_mut(&mut self, command: HistoryCommand) -> Result<HistoryApplyOutcome, HistoryError> {
        match command {
            HistoryCommand::Append { payload } => {
                let revision = self.append(payload)?;
                Ok(HistoryApplyOutcome::Appended { revision })
            }
            HistoryCommand::Undo => self.undo(),
            HistoryCommand::Redo => self.redo(),
            HistoryCommand::CreateBranch { name, from } => {
                let source = from.unwrap_or_else(|| self.active_cursor());
                let branch = self.create_branch(name, source)?;
                Ok(HistoryApplyOutcome::BranchCreated { branch })
            }
            HistoryCommand::SwitchBranch { branch } => {
                self.switch_branch(branch)?;
                Ok(HistoryApplyOutcome::BranchSwitched { branch })
            }
            HistoryCommand::Transfer { source, policy } => {
                let revision = self.transfer(source, policy)?;
                Ok(HistoryApplyOutcome::Appended { revision })
            }
            HistoryCommand::CreateSnapshot { name } => {
                let snapshot = self.create_snapshot(name)?;
                Ok(HistoryApplyOutcome::SnapshotCreated { snapshot })
            }
            HistoryCommand::DeleteSnapshot { snapshot } => {
                if self.snapshots.remove(&snapshot).is_none() {
                    return Err(HistoryError::UnknownSnapshot(snapshot));
                }
                Ok(HistoryApplyOutcome::MetadataChanged)
            }
            HistoryCommand::DeleteBranch { branch } => {
                self.delete_branch(branch)?;
                Ok(HistoryApplyOutcome::MetadataChanged)
            }
            HistoryCommand::RetainEvidence { revision, kind } => {
                if !self.revisions.contains_key(&revision) {
                    return Err(HistoryError::UnknownRevision(revision));
                }
                if !self.evidence.insert(HistoryEvidence::new(revision, kind)) {
                    return Err(HistoryError::DuplicateEvidence);
                }
                Ok(HistoryApplyOutcome::MetadataChanged)
            }
            HistoryCommand::ReleaseEvidence { revision, kind } => {
                if !self.evidence.remove(&HistoryEvidence::new(revision, kind)) {
                    return Err(HistoryError::MissingEvidence);
                }
                Ok(HistoryApplyOutcome::MetadataChanged)
            }
            HistoryCommand::PruneOrphans => {
                let removed = self.prune_orphans();
                Ok(HistoryApplyOutcome::Pruned { removed })
            }
        }
    }

    fn append(&mut self, payload: HistoryPayload) -> Result<HistoryRevisionId, HistoryError> {
        if payload.edit().photo_id() != self.photo_id {
            return Err(HistoryError::PhotoMismatch);
        }
        let active = self.active_branch;
        let active_branch = self
            .branches
            .get(&active)
            .cloned()
            .ok_or(HistoryError::UnknownBranch(active))?;
        let cursor = active_branch.cursor();
        let branch = if active_branch
            .head()
            .is_some_and(|head| Some(head) != cursor)
        {
            let branch_id = self.allocate_branch_id()?;
            let lineage = lineage_prefix(&active_branch, cursor)?;
            let branch = HistoryBranch::new(
                branch_id,
                format!("branch-{branch_id}"),
                cursor,
                lineage,
                cursor,
            );
            self.branches.insert(branch_id, branch);
            self.active_branch = branch_id;
            branch_id
        } else {
            active
        };
        let parent = self
            .branches
            .get(&branch)
            .ok_or(HistoryError::UnknownBranch(branch))?
            .cursor();
        let revision_id = self.allocate_revision_id()?;
        let revision = HistoryRevision::new(revision_id, parent, payload);
        self.revisions.insert(revision_id, revision);
        let branch = self
            .branches
            .get_mut(&branch)
            .ok_or(HistoryError::UnknownBranch(branch))?;
        branch.lineage.push(revision_id);
        branch.cursor = Some(revision_id);
        branch.redo.clear();
        Ok(revision_id)
    }

    fn undo(&mut self) -> Result<HistoryApplyOutcome, HistoryError> {
        let branch_id = self.active_branch;
        let branch = self
            .branches
            .get(&branch_id)
            .cloned()
            .ok_or(HistoryError::UnknownBranch(branch_id))?;
        let current = branch.cursor().ok_or(HistoryError::NoUndo)?;
        let parent = self
            .revisions
            .get(&current)
            .ok_or(HistoryError::UnknownRevision(current))?
            .parent()
            .ok_or(HistoryError::NoUndo)?;
        let branch = self
            .branches
            .get_mut(&branch_id)
            .ok_or(HistoryError::UnknownBranch(branch_id))?;
        branch.cursor = Some(parent);
        branch.redo.push(current);
        Ok(HistoryApplyOutcome::CursorMoved {
            cursor: HistoryCursor::new(branch_id, Some(parent)),
        })
    }

    fn redo(&mut self) -> Result<HistoryApplyOutcome, HistoryError> {
        let branch_id = self.active_branch;
        let branch = self
            .branches
            .get(&branch_id)
            .cloned()
            .ok_or(HistoryError::UnknownBranch(branch_id))?;
        let revision = *branch.redo.last().ok_or(HistoryError::NoRedo)?;
        let parent = self
            .revisions
            .get(&revision)
            .ok_or(HistoryError::UnknownRevision(revision))?
            .parent();
        if parent != branch.cursor() {
            return Err(HistoryError::InvalidPersistedState);
        }
        let branch = self
            .branches
            .get_mut(&branch_id)
            .ok_or(HistoryError::UnknownBranch(branch_id))?;
        branch.redo.pop();
        branch.cursor = Some(revision);
        Ok(HistoryApplyOutcome::CursorMoved {
            cursor: HistoryCursor::new(branch_id, Some(revision)),
        })
    }

    fn create_branch(
        &mut self,
        name: String,
        source: HistoryCursor,
    ) -> Result<HistoryBranchId, HistoryError> {
        if !validate_name(&name) {
            return Err(HistoryError::InvalidBranchName);
        }
        self.validate_cursor(source)?;
        let source_branch = self
            .branches
            .get(&source.branch())
            .ok_or(HistoryError::UnknownBranch(source.branch()))?;
        let lineage = lineage_prefix(source_branch, source.revision())?;
        let id = self.allocate_branch_id()?;
        self.branches.insert(
            id,
            HistoryBranch::new(id, name, source.revision(), lineage, source.revision()),
        );
        self.active_branch = id;
        Ok(id)
    }

    fn switch_branch(&mut self, branch: HistoryBranchId) -> Result<(), HistoryError> {
        if !self.branches.contains_key(&branch) {
            return Err(HistoryError::UnknownBranch(branch));
        }
        self.active_branch = branch;
        Ok(())
    }

    fn transfer(
        &mut self,
        source: HistoryCursor,
        policy: BranchTransferPolicy,
    ) -> Result<HistoryRevisionId, HistoryError> {
        self.validate_cursor(source)?;
        let source_revision = source.revision().ok_or(HistoryError::EmptyHistory)?;
        let source_payload = self
            .revisions
            .get(&source_revision)
            .ok_or(HistoryError::UnknownRevision(source_revision))?
            .payload()
            .clone();
        let kind = match policy {
            BranchTransferPolicy::Copy => super::types::HistoryOperationKind::Copy,
            BranchTransferPolicy::Merge => super::types::HistoryOperationKind::Merge,
        };
        let summary = super::types::HistoryOperationSummary::new(
            kind,
            source_payload.summary().operation_id(),
            source_payload.summary().operation_key().cloned(),
            match policy {
                BranchTransferPolicy::Copy => "copy/paste",
                BranchTransferPolicy::Merge => "branch merge",
            },
        )?;
        let payload = HistoryPayload::new(
            source_payload.edit().clone(),
            source_payload.mask_bytes().to_owned(),
            source_payload.pipeline_bytes().to_owned(),
            summary,
        );
        self.append(payload)
    }

    fn create_snapshot(&mut self, name: String) -> Result<HistorySnapshotId, HistoryError> {
        if !validate_name(&name) {
            return Err(HistoryError::InvalidSnapshotName);
        }
        let cursor = self.active_cursor();
        cursor.revision().ok_or(HistoryError::EmptyHistory)?;
        let id = self.allocate_snapshot_id()?;
        self.snapshots
            .insert(id, HistorySnapshot::new(id, name, cursor));
        Ok(id)
    }

    fn delete_branch(&mut self, branch: HistoryBranchId) -> Result<(), HistoryError> {
        if branch == self.active_branch {
            return Err(HistoryError::ActiveBranchDeletion);
        }
        if !self.branches.contains_key(&branch) {
            return Err(HistoryError::UnknownBranch(branch));
        }
        if self
            .snapshots
            .values()
            .any(|snapshot| snapshot.cursor().branch() == branch)
        {
            return Err(HistoryError::BranchHasSnapshot(branch));
        }
        let lineage = self
            .branches
            .get(&branch)
            .expect("branch presence checked")
            .lineage();
        if self
            .evidence
            .iter()
            .any(|evidence| lineage.contains(&evidence.revision()))
        {
            return Err(HistoryError::BranchHasEvidence(branch));
        }
        self.branches.remove(&branch);
        Ok(())
    }

    fn prune_orphans(&mut self) -> usize {
        let reachable = self
            .branches
            .values()
            .flat_map(|branch| branch.lineage().iter().copied())
            .chain(
                self.snapshots
                    .values()
                    .filter_map(|snapshot| snapshot.cursor().revision()),
            )
            .chain(self.evidence.iter().map(|evidence| evidence.revision()))
            .collect::<BTreeSet<_>>();
        let mut closure = reachable;
        let mut pending = closure.iter().copied().collect::<Vec<_>>();
        while let Some(id) = pending.pop() {
            if let Some(parent) = self.revisions.get(&id).and_then(HistoryRevision::parent)
                && closure.insert(parent)
            {
                pending.push(parent);
            }
        }
        let before = self.revisions.len();
        self.revisions.retain(|id, _| closure.contains(id));
        before - self.revisions.len()
    }

    fn validate_cursor(&self, cursor: HistoryCursor) -> Result<(), HistoryError> {
        let branch = self
            .branches
            .get(&cursor.branch())
            .ok_or(HistoryError::UnknownBranch(cursor.branch()))?;
        if cursor
            .revision()
            .is_some_and(|revision| !branch.lineage().contains(&revision))
        {
            return Err(HistoryError::InvalidCursor);
        }
        Ok(())
    }

    fn allocate_revision_id(&mut self) -> Result<HistoryRevisionId, HistoryError> {
        let id =
            HistoryRevisionId::new(self.next_revision_id).ok_or(HistoryError::RevisionOverflow)?;
        self.next_revision_id = self
            .next_revision_id
            .checked_add(1)
            .ok_or(HistoryError::RevisionOverflow)?;
        Ok(id)
    }

    fn allocate_branch_id(&mut self) -> Result<HistoryBranchId, HistoryError> {
        let id = HistoryBranchId::new(self.next_branch_id).ok_or(HistoryError::RevisionOverflow)?;
        self.next_branch_id = self
            .next_branch_id
            .checked_add(1)
            .ok_or(HistoryError::RevisionOverflow)?;
        Ok(id)
    }

    fn allocate_snapshot_id(&mut self) -> Result<HistorySnapshotId, HistoryError> {
        let id =
            HistorySnapshotId::new(self.next_snapshot_id).ok_or(HistoryError::RevisionOverflow)?;
        self.next_snapshot_id = self
            .next_snapshot_id
            .checked_add(1)
            .ok_or(HistoryError::RevisionOverflow)?;
        Ok(id)
    }
}

fn lineage_prefix(
    branch: &HistoryBranch,
    cursor: Option<HistoryRevisionId>,
) -> Result<Vec<HistoryRevisionId>, HistoryError> {
    match cursor {
        None => Ok(Vec::new()),
        Some(cursor) => branch
            .lineage()
            .iter()
            .position(|revision| *revision == cursor)
            .map(|index| branch.lineage()[..=index].to_vec())
            .ok_or(HistoryError::InvalidCursor),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryApplyOutcome {
    Appended { revision: HistoryRevisionId },
    CursorMoved { cursor: HistoryCursor },
    BranchCreated { branch: HistoryBranchId },
    BranchSwitched { branch: HistoryBranchId },
    SnapshotCreated { snapshot: HistorySnapshotId },
    Pruned { removed: usize },
    MetadataChanged,
}
