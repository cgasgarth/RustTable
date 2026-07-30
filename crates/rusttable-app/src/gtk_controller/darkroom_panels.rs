//! Controller-owned projections for the canonical darkroom history and snapshot rails.
//!
//! The controller caches a [`HistoryState`] loaded from the catalog history repository. The
//! GTK-facing models below are rebuilt from that state after every accepted command; they never
//! own a second history graph.
//!
//! #761 seam
//!
//! The current canonical API has commands for cursor movement, branches, snapshots, and
//! transfers, but no persisted comparison-pointer command. Before/after is therefore represented
//! by the typed [`HistoryComparisonPair`] value at this boundary and remains view state. When
//! #761 exposes a comparison command, replace `comparison_for_snapshot` and the
//! `ToggleSnapshotCompare` arm with that command; do not add a UI persistence table.

use std::fmt;
use std::path::{Path, PathBuf};

use rusttable_catalog::{
    DurableHistoryError, DurableHistoryService, EditRepository, HistoryCommand,
    HistoryComparisonPair, HistoryOperationKind, HistoryOperationSummary, HistoryPayload,
    HistoryRepository, HistoryRevisionId, HistorySnapshotId, HistoryState,
};
use rusttable_catalog_store::{RedbCatalogRepository, RedbHistoryRepository};
use rusttable_core::{Edit, Operation, PhotoId, Revision};
use rusttable_ui::presentation::{
    DarkroomHistoryEntry, DarkroomHistoryError, DarkroomHistoryViewModel, DarkroomPanelAction,
    DarkroomPanelError, DarkroomPanelId, DarkroomPanelProjection, DarkroomPanelRouter,
    DarkroomPanelState, DarkroomPanelTarget, DarkroomSnapshotEntry, DarkroomSnapshotsViewModel,
    HistoryDirection, HistoryEntryId,
};

/// The two left-rail projections updated together for one selected photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomPanelProjections {
    history: DarkroomPanelProjection<DarkroomHistoryViewModel>,
    snapshots: DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
}

impl DarkroomPanelProjections {
    #[must_use]
    pub fn loading(target: DarkroomPanelTarget, revision: Revision) -> Self {
        Self {
            history: DarkroomPanelProjection::loading(target, revision),
            snapshots: DarkroomPanelProjection::loading(target, revision),
        }
    }

    /// Builds matching recoverable error projections for both rails.
    ///
    /// # Errors
    ///
    /// Returns the UI presentation-text validation error when the detail cannot be displayed.
    pub fn error(
        target: DarkroomPanelTarget,
        revision: Revision,
        detail: impl Into<String>,
    ) -> Result<Self, rusttable_ui::presentation::PresentationTextError> {
        let detail = detail.into();
        Ok(Self {
            history: DarkroomPanelProjection::error(target, revision, detail.clone())?,
            snapshots: DarkroomPanelProjection::error(target, revision, detail)?,
        })
    }

    #[must_use]
    pub const fn history(&self) -> &DarkroomPanelProjection<DarkroomHistoryViewModel> {
        &self.history
    }

    #[must_use]
    pub const fn snapshots(&self) -> &DarkroomPanelProjection<DarkroomSnapshotsViewModel> {
        &self.snapshots
    }
}

/// Application-side owner of the selected photo's canonical history projection.
#[derive(Debug, Clone)]
pub struct GtkDarkroomPanelController {
    catalog_path: Option<PathBuf>,
    target: Option<DarkroomPanelTarget>,
    router: DarkroomPanelRouter,
    history_state: Option<HistoryState>,
    selected_snapshot: Option<HistorySnapshotId>,
    comparison: Option<HistoryComparisonPair>,
    projections: Option<DarkroomPanelProjections>,
}

impl GtkDarkroomPanelController {
    #[must_use]
    pub fn new(catalog_path: Option<PathBuf>) -> Self {
        Self {
            catalog_path,
            target: None,
            router: DarkroomPanelRouter::default(),
            history_state: None,
            selected_snapshot: None,
            comparison: None,
            projections: None,
        }
    }

    #[must_use]
    pub const fn target(&self) -> Option<DarkroomPanelTarget> {
        self.target
    }

    pub fn set_catalog_path(&mut self, catalog_path: Option<PathBuf>) {
        self.catalog_path = catalog_path;
    }

    #[must_use]
    pub const fn projections(&self) -> Option<&DarkroomPanelProjections> {
        self.projections.as_ref()
    }

    /// Loads the selected photo's canonical history, seeding missing history through the same
    /// durable service used for later commands. Existing edits are adapted into immutable
    /// operation prefixes until #761's import/reconstruction adapter is available.
    ///
    /// # Errors
    ///
    /// Returns a typed persistence or presentation error and leaves the previous projection
    /// untouched so the caller can render a recoverable error for the requested target.
    pub fn select_photo(
        &mut self,
        target: DarkroomPanelTarget,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let catalog_path = self.catalog_path()?;
        let catalog = RedbCatalogRepository::open(catalog_path)
            .map_err(|error| persistence(error.to_string()))?;
        let edit = current_edit(&catalog, target.photo_id())?;
        drop(catalog);
        let history = load_or_seed_history(catalog_path, target.photo_id(), edit.as_ref())?;
        let edit_revision = edit.as_ref().map_or(Revision::ZERO, Edit::revision);

        self.router = DarkroomPanelRouter::default();
        self.router
            .reconcile(target, edit_revision)
            .map_err(DarkroomPanelControllerError::Panel)?;
        self.target = Some(target);
        self.history_state = Some(history);
        self.selected_snapshot = None;
        self.comparison = None;
        let projections = self.project(self.router.revision())?;
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    /// Rebinds projections to the newest preview generation without accepting older callbacks.
    ///
    /// # Errors
    ///
    /// Returns a stale-target error when the supplied target is not the current photo.
    pub fn rebind_target(
        &mut self,
        target: DarkroomPanelTarget,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let current = self
            .target
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        if current.photo_id() != target.photo_id() {
            return Err(DarkroomPanelControllerError::Panel(
                DarkroomPanelError::StaleTarget {
                    expected: target,
                    actual: Some(current),
                },
            ));
        }
        self.target = Some(target);
        let projections = self.project(self.router.revision())?;
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    /// Reloads canonical history after another controller commits the selected photo's edit.
    ///
    /// This is the application refresh seam used by the GTK module controller. It allows module
    /// edits to become durable history entries without teaching GTK widgets about persistence.
    ///
    /// # Errors
    ///
    /// Returns a persistence or projection error when the canonical state cannot be refreshed.
    pub fn refresh(&mut self) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let target = self
            .target
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let path = self.catalog_path()?;
        let catalog =
            RedbCatalogRepository::open(path).map_err(|error| persistence(error.to_string()))?;
        let edit = current_edit(&catalog, target.photo_id())?;
        drop(catalog);
        let history = load_or_seed_history(path, target.photo_id(), edit.as_ref())?;
        self.history_state = Some(history);
        let projections = self.project(self.router.revision())?;
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    /// Applies one stale-target-checked GTK action through canonical history commands.
    ///
    /// The history repository is committed before the controller publishes its new projection.
    /// Cursor-changing commands also project the canonical payload edit back into the existing
    /// catalog edit record so the worker preview observes the same revision.
    ///
    /// # Errors
    ///
    /// Returns a stale, domain, persistence, or projection error without publishing UI state.
    #[allow(clippy::too_many_lines)]
    pub fn apply(
        &mut self,
        action: &DarkroomPanelAction,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let current = self
            .projections
            .clone()
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let target = self
            .target
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let mut candidate_router = self.router;
        candidate_router
            .route(action)
            .map_err(DarkroomPanelControllerError::Panel)?;
        if action_target(action) != target {
            return Err(DarkroomPanelControllerError::Panel(
                DarkroomPanelError::StaleTarget {
                    expected: action_target(action),
                    actual: Some(target),
                },
            ));
        }

        let mut state = self
            .history_state
            .clone()
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let path = self.catalog_path()?;
        let mut history_repository = RedbHistoryRepository::open(path, target.photo_id())
            .map_err(|error| persistence(error.to_string()))?;
        let mut selected_snapshot = self.selected_snapshot;
        let mut comparison = self.comparison;
        let changes_preview = match action {
            DarkroomPanelAction::SelectHistory { index, .. } => {
                move_to_history_index(&mut state, *index, &mut history_repository)?;
                true
            }
            DarkroomPanelAction::NavigateHistory { direction, .. } => {
                apply_history_command(
                    &mut state,
                    match direction {
                        HistoryDirection::Previous => HistoryCommand::Undo,
                        HistoryDirection::Next => HistoryCommand::Redo,
                    },
                    &mut history_repository,
                )?;
                true
            }
            DarkroomPanelAction::CreateBranch { name, .. } => {
                apply_history_command(
                    &mut state,
                    HistoryCommand::CreateBranch {
                        name: name.clone(),
                        from: None,
                    },
                    &mut history_repository,
                )?;
                false
            }
            DarkroomPanelAction::CreateSnapshot { name, .. } => {
                apply_history_command(
                    &mut state,
                    HistoryCommand::CreateSnapshot { name: name.clone() },
                    &mut history_repository,
                )?;
                false
            }
            DarkroomPanelAction::SelectSnapshot { id, .. } => {
                let id = HistorySnapshotId::new(*id)
                    .ok_or(DarkroomPanelControllerError::UnknownSnapshot(*id))?;
                let snapshot = state
                    .snapshots()
                    .find(|snapshot| snapshot.id() == id)
                    .ok_or(DarkroomPanelControllerError::UnknownSnapshot(id.get()))?;
                selected_snapshot = Some(snapshot.id());
                if comparison.is_some() {
                    comparison = comparison_for_snapshot(&state, selected_snapshot);
                }
                false
            }
            DarkroomPanelAction::RestoreSnapshot { id, .. } => {
                let (snapshot_id, cursor) = state
                    .snapshots()
                    .find(|snapshot| snapshot.id().get() == *id)
                    .map(|snapshot| (snapshot.id(), snapshot.cursor()))
                    .ok_or(DarkroomPanelControllerError::UnknownSnapshot(*id))?;
                restore_snapshot(&mut state, cursor, &mut history_repository)?;
                selected_snapshot = Some(snapshot_id);
                comparison = None;
                true
            }
            DarkroomPanelAction::ToggleSnapshotCompare { enabled, .. } => {
                comparison = if *enabled {
                    comparison_for_snapshot(&state, selected_snapshot)
                } else {
                    None
                };
                false
            }
            DarkroomPanelAction::SetExpanded { .. } | DarkroomPanelAction::SetScroll { .. } => {
                false
            }
        };

        drop(history_repository);
        if changes_preview {
            let mut catalog = RedbCatalogRepository::open(path)
                .map_err(|error| persistence(error.to_string()))?;
            persist_current_edit(&mut catalog, &state, target.photo_id())?;
        }

        self.router = candidate_router;
        self.history_state = Some(state);
        self.selected_snapshot = selected_snapshot;
        self.comparison = comparison;
        let projections = match action {
            DarkroomPanelAction::SetExpanded {
                panel, expanded, ..
            } => set_expanded(
                &current,
                *panel,
                target,
                *expanded,
                candidate_router.revision(),
            ),
            DarkroomPanelAction::SetScroll {
                panel,
                position_milli,
                ..
            } => set_scroll(
                &current,
                *panel,
                target,
                *position_milli,
                candidate_router.revision(),
            ),
            _ => self.project(candidate_router.revision())?,
        };
        self.target = Some(target);
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    fn project(
        &self,
        revision: Revision,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let target = self
            .target
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let state = self
            .history_state
            .as_ref()
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let history = project_history(state, revision)?;
        let snapshots = project_snapshots(state, self.selected_snapshot, self.comparison)?;
        Ok(DarkroomPanelProjections {
            history: DarkroomPanelProjection::ready(target, revision, history),
            snapshots: DarkroomPanelProjection::ready(target, revision, snapshots),
        })
    }

    fn catalog_path(&self) -> Result<&Path, DarkroomPanelControllerError> {
        self.catalog_path
            .as_deref()
            .ok_or(DarkroomPanelControllerError::NoCatalog)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomPanelControllerError {
    NoCatalog,
    NoSelection,
    Persistence(String),
    Canonical(String),
    History(DarkroomHistoryError),
    Panel(DarkroomPanelError),
    Presentation(String),
    UnknownSnapshot(u64),
}

impl fmt::Display for DarkroomPanelControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCatalog => formatter.write_str("catalog path is unavailable"),
            Self::NoSelection => formatter.write_str("no darkroom photo is selected"),
            Self::Persistence(message) | Self::Canonical(message) | Self::Presentation(message) => {
                formatter.write_str(message)
            }
            Self::History(error) => error.fmt(formatter),
            Self::Panel(error) => error.fmt(formatter),
            Self::UnknownSnapshot(id) => write!(formatter, "unknown snapshot {id}"),
        }
    }
}

impl std::error::Error for DarkroomPanelControllerError {}

impl From<DarkroomHistoryError> for DarkroomPanelControllerError {
    fn from(error: DarkroomHistoryError) -> Self {
        Self::History(error)
    }
}

fn persistence(message: impl Into<String>) -> DarkroomPanelControllerError {
    DarkroomPanelControllerError::Persistence(message.into())
}

fn action_target(action: &DarkroomPanelAction) -> DarkroomPanelTarget {
    match action {
        DarkroomPanelAction::SelectHistory { target, .. }
        | DarkroomPanelAction::NavigateHistory { target, .. }
        | DarkroomPanelAction::CreateBranch { target, .. }
        | DarkroomPanelAction::CreateSnapshot { target, .. }
        | DarkroomPanelAction::SelectSnapshot { target, .. }
        | DarkroomPanelAction::RestoreSnapshot { target, .. }
        | DarkroomPanelAction::ToggleSnapshotCompare { target, .. }
        | DarkroomPanelAction::SetExpanded { target, .. }
        | DarkroomPanelAction::SetScroll { target, .. } => *target,
    }
}

fn current_edit(
    repository: &RedbCatalogRepository,
    photo_id: PhotoId,
) -> Result<Option<Edit>, DarkroomPanelControllerError> {
    repository
        .list()
        .map_err(|error| persistence(error.to_string()))
        .map(|edits| {
            edits
                .into_iter()
                .filter(|edit| edit.photo_id() == photo_id)
                .max_by_key(|edit| (edit.revision(), edit.id()))
        })
}

fn load_or_seed_history(
    path: &Path,
    photo_id: PhotoId,
    edit: Option<&Edit>,
) -> Result<HistoryState, DarkroomPanelControllerError> {
    let mut repository = RedbHistoryRepository::open(path, photo_id)
        .map_err(|error| persistence(error.to_string()))?;
    let mut state = repository
        .load()
        .map_err(|error| persistence(error.to_string()))?
        .unwrap_or_else(|| HistoryState::new(photo_id));
    if let Some(edit) = edit
        && state
            .current_revision()
            .is_none_or(|revision| revision.payload().edit() != edit)
    {
        if state.revisions().next().is_none() {
            seed_operation_prefixes(&mut state, edit, &mut repository)?;
        } else {
            append_payload(
                &mut state,
                edit.clone(),
                HistoryOperationKind::Parameter,
                None,
                None,
                "Current edit",
                &mut repository,
            )?;
        }
    }
    Ok(state)
}

fn seed_operation_prefixes(
    state: &mut HistoryState,
    edit: &Edit,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    let operations = edit.operations().cloned().collect::<Vec<_>>();
    if operations.is_empty() {
        append_payload(
            state,
            edit.clone(),
            HistoryOperationKind::Reset,
            None,
            None,
            "Original edit",
            repository,
        )?;
        return Ok(());
    }
    for index in 0..operations.len() {
        let prefix = Edit::from_parts(
            edit.id(),
            edit.photo_id(),
            edit.base_photo_revision(),
            edit.revision(),
            operations[..=index].iter().cloned(),
        )
        .map_err(|error| DarkroomPanelControllerError::Canonical(error.to_string()))?;
        let operation = &operations[index];
        append_payload(
            state,
            prefix,
            operation_kind(operation),
            Some(operation.id()),
            Some(operation.key().clone()),
            operation.key().as_str(),
            repository,
        )?;
    }
    Ok(())
}

fn append_payload(
    state: &mut HistoryState,
    edit: Edit,
    kind: HistoryOperationKind,
    operation_id: Option<rusttable_core::OperationId>,
    operation_key: Option<rusttable_core::OperationKey>,
    label: &str,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    let summary = HistoryOperationSummary::new(kind, operation_id, operation_key, label)
        .map_err(|error| DarkroomPanelControllerError::Canonical(error.to_string()))?;
    apply_history_command(
        state,
        HistoryCommand::Append {
            payload: HistoryPayload::new(edit, Vec::new(), Vec::new(), summary),
        },
        repository,
    )?;
    Ok(())
}

fn apply_history_command(
    state: &mut HistoryState,
    command: HistoryCommand,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    let expected = state.version();
    DurableHistoryService::apply(state, expected, command, repository)
        .map(|_| ())
        .map_err(map_history_error)
}

fn map_history_error(error: DurableHistoryError) -> DarkroomPanelControllerError {
    match error {
        DurableHistoryError::Domain(error) => {
            DarkroomPanelControllerError::Canonical(error.to_string())
        }
        DurableHistoryError::Repository(error) => persistence(error.to_string()),
    }
}

fn move_to_history_index(
    state: &mut HistoryState,
    index: usize,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    let branch = state.branch(state.active_branch_id()).ok_or_else(|| {
        DarkroomPanelControllerError::Canonical("active branch is missing".to_owned())
    })?;
    let target = *branch.lineage().get(index).ok_or_else(|| {
        DarkroomPanelControllerError::History(DarkroomHistoryError::EntryIndexOutOfRange {
            index,
            entries: branch.lineage().len(),
        })
    })?;
    move_to_revision(state, target, repository)
}

fn move_to_revision(
    state: &mut HistoryState,
    target: HistoryRevisionId,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    loop {
        let branch = state.branch(state.active_branch_id()).ok_or_else(|| {
            DarkroomPanelControllerError::Canonical("active branch is missing".to_owned())
        })?;
        let current = branch.cursor();
        if current == Some(target) {
            return Ok(());
        }
        let current_index = current
            .and_then(|id| branch.lineage().iter().position(|revision| *revision == id))
            .unwrap_or(usize::MAX);
        let target_index = branch
            .lineage()
            .iter()
            .position(|revision| *revision == target)
            .ok_or_else(|| {
                DarkroomPanelControllerError::Canonical("target is not on branch".to_owned())
            })?;
        let command = if target_index < current_index {
            HistoryCommand::Undo
        } else {
            HistoryCommand::Redo
        };
        apply_history_command(state, command, repository)?;
    }
}

fn restore_snapshot(
    state: &mut HistoryState,
    cursor: rusttable_catalog::HistoryCursor,
    repository: &mut RedbHistoryRepository,
) -> Result<(), DarkroomPanelControllerError> {
    if state.active_branch_id() != cursor.branch() {
        apply_history_command(
            state,
            HistoryCommand::SwitchBranch {
                branch: cursor.branch(),
            },
            repository,
        )?;
    }
    let target = cursor.revision().ok_or_else(|| {
        DarkroomPanelControllerError::Canonical("snapshot has no revision".to_owned())
    })?;
    move_to_revision(state, target, repository)
}

fn persist_current_edit(
    repository: &mut RedbCatalogRepository,
    state: &HistoryState,
    photo_id: PhotoId,
) -> Result<(), DarkroomPanelControllerError> {
    let Some(desired) = state
        .current_revision()
        .map(|revision| revision.payload().edit())
    else {
        return Ok(());
    };
    let current = current_edit(repository, photo_id)?;
    match current {
        Some(current) if current != *desired => repository
            .commit_replacement(current.revision(), desired)
            .map_err(|error| persistence(error.to_string())),
        None => repository
            .commit_new(desired)
            .map_err(|error| persistence(error.to_string())),
        Some(_) => Ok(()),
    }
}

fn project_history(
    state: &HistoryState,
    revision: Revision,
) -> Result<DarkroomHistoryViewModel, DarkroomPanelControllerError> {
    let branch = state.branch(state.active_branch_id()).ok_or_else(|| {
        DarkroomPanelControllerError::Canonical("active branch is missing".to_owned())
    })?;
    let entries = branch
        .lineage()
        .iter()
        .map(|id| {
            let revision = state.revision(*id).ok_or_else(|| {
                DarkroomPanelControllerError::Canonical("history revision is missing".to_owned())
            })?;
            DarkroomHistoryEntry::new(
                HistoryEntryId::from_u64(id.get()),
                revision.payload().summary().label(),
            )
            .map_err(|error| DarkroomPanelControllerError::Presentation(format!("{error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selected = branch
        .cursor()
        .and_then(|cursor| branch.lineage().iter().position(|id| *id == cursor));
    DarkroomHistoryViewModel::new(revision, entries, selected).map_err(Into::into)
}

fn project_snapshots(
    state: &HistoryState,
    selected: Option<HistorySnapshotId>,
    comparison: Option<HistoryComparisonPair>,
) -> Result<DarkroomSnapshotsViewModel, DarkroomPanelControllerError> {
    let selected_snapshot = selected.and_then(|snapshot_id| {
        state
            .snapshots()
            .find(|snapshot| snapshot.id() == snapshot_id)
            .map(|snapshot| snapshot.id().get())
    });
    let entries = state
        .snapshots()
        .map(|snapshot| {
            let current = snapshot.cursor() == state.active_cursor();
            let branch = state
                .branch(snapshot.cursor().branch())
                .map_or("unknown", rusttable_catalog::HistoryBranch::name);
            let status = if current {
                format!("{branch} · current")
            } else {
                format!("{branch} · saved")
            };
            DarkroomSnapshotEntry::new(snapshot.id().get(), snapshot.name(), status)
                .map_err(|error| DarkroomPanelControllerError::Presentation(format!("{error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    DarkroomSnapshotsViewModel::new(entries, selected_snapshot, comparison.is_some())
        .map_err(DarkroomPanelControllerError::Panel)
}

fn comparison_for_snapshot(
    state: &HistoryState,
    selected: Option<HistorySnapshotId>,
) -> Option<HistoryComparisonPair> {
    let snapshot_id = selected?;
    let snapshot = state
        .snapshots()
        .find(|snapshot| snapshot.id() == snapshot_id)?;
    Some(HistoryComparisonPair::new(
        snapshot.cursor(),
        state.active_cursor(),
    ))
}

fn operation_kind(operation: &Operation) -> HistoryOperationKind {
    let key = operation.key().as_str().to_ascii_lowercase();
    if key.contains("mask") {
        HistoryOperationKind::Mask
    } else if key.contains("blend") {
        HistoryOperationKind::Blend
    } else if key.contains("enable") || key.contains("enabled") {
        HistoryOperationKind::Enable
    } else if key.contains("style") {
        HistoryOperationKind::Style
    } else if key.contains("order") {
        HistoryOperationKind::Order
    } else {
        HistoryOperationKind::Parameter
    }
}

fn rebind_projection<T: Clone>(
    projection: &DarkroomPanelProjection<T>,
    target: DarkroomPanelTarget,
) -> DarkroomPanelProjection<T> {
    match projection.state() {
        DarkroomPanelState::Empty => DarkroomPanelProjection::empty(),
        DarkroomPanelState::Loading => {
            DarkroomPanelProjection::loading(target, projection.revision())
        }
        DarkroomPanelState::Error(error) => {
            DarkroomPanelProjection::error(target, projection.revision(), error.as_str())
                .unwrap_or_else(|_| DarkroomPanelProjection::loading(target, projection.revision()))
        }
        DarkroomPanelState::Ready(value) => {
            DarkroomPanelProjection::ready(target, projection.revision(), value.clone())
                .with_view_state(projection.expanded(), projection.scroll_position_milli())
        }
    }
}

fn set_expanded(
    current: &DarkroomPanelProjections,
    panel: DarkroomPanelId,
    target: DarkroomPanelTarget,
    expanded: bool,
    revision: Revision,
) -> DarkroomPanelProjections {
    let mut result = current.clone();
    match panel {
        DarkroomPanelId::History => {
            result.history = rebind_projection(current.history(), target)
                .with_view_state(expanded, current.history().scroll_position_milli());
        }
        DarkroomPanelId::Snapshots => {
            result.snapshots = rebind_projection(current.snapshots(), target)
                .with_view_state(expanded, current.snapshots().scroll_position_milli());
        }
        DarkroomPanelId::ImageInformation => {}
    }
    set_projection_revision(result, revision)
}

fn set_scroll(
    current: &DarkroomPanelProjections,
    panel: DarkroomPanelId,
    target: DarkroomPanelTarget,
    position_milli: u32,
    revision: Revision,
) -> DarkroomPanelProjections {
    let mut result = current.clone();
    match panel {
        DarkroomPanelId::History => {
            result.history = rebind_projection(current.history(), target)
                .with_view_state(current.history().expanded(), position_milli);
        }
        DarkroomPanelId::Snapshots => {
            result.snapshots = rebind_projection(current.snapshots(), target)
                .with_view_state(current.snapshots().expanded(), position_milli);
        }
        DarkroomPanelId::ImageInformation => {}
    }
    set_projection_revision(result, revision)
}

fn set_projection_revision(
    projections: DarkroomPanelProjections,
    revision: Revision,
) -> DarkroomPanelProjections {
    let mut result = projections;
    result.history = replace_projection_revision(result.history, revision);
    result.snapshots = replace_projection_revision(result.snapshots, revision);
    result
}

fn replace_projection_revision<T: Clone>(
    projection: DarkroomPanelProjection<T>,
    revision: Revision,
) -> DarkroomPanelProjection<T> {
    match projection.state() {
        DarkroomPanelState::Empty => DarkroomPanelProjection::empty(),
        DarkroomPanelState::Loading => DarkroomPanelProjection::loading(
            projection.target().expect("loading projection target"),
            revision,
        ),
        DarkroomPanelState::Error(error) => DarkroomPanelProjection::error(
            projection.target().expect("error projection target"),
            revision,
            error.as_str(),
        )
        .unwrap_or(projection),
        DarkroomPanelState::Ready(value) => DarkroomPanelProjection::ready(
            projection.target().expect("ready projection target"),
            revision,
            value.clone(),
        )
        .with_view_state(projection.expanded(), projection.scroll_position_milli()),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rusttable_catalog::{HistoryCommand, HistoryOperationKind};
    use rusttable_catalog_store::RedbHistoryRepository;
    use rusttable_core::{
        EditId, OperationId, OperationKey, OperationOpacity, ParameterName, ParameterValue,
    };

    use super::*;

    static TEST_PATH: AtomicU64 = AtomicU64::new(0);

    fn test_path(label: &str) -> PathBuf {
        let id = TEST_PATH.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "rusttable-763-{label}-{}-{id}.redb",
            std::process::id()
        ))
    }

    fn edit(photo: PhotoId) -> Edit {
        Edit::from_parts(
            EditId::new(1).expect("edit id"),
            photo,
            Revision::ZERO,
            Revision::from_u64(3),
            [
                Operation::new_with_opacity(
                    OperationId::new(1).expect("operation id"),
                    OperationKey::new("darktable.exposure").expect("key"),
                    true,
                    OperationOpacity::ONE,
                    [(
                        ParameterName::new("stops").expect("name"),
                        ParameterValue::Integer(1),
                    )],
                )
                .expect("operation"),
                Operation::new_with_opacity(
                    OperationId::new(2).expect("operation id"),
                    OperationKey::new("darktable.colorbalance").expect("key"),
                    true,
                    OperationOpacity::ONE,
                    [],
                )
                .expect("operation"),
            ],
        )
        .expect("edit")
    }

    #[test]
    fn selected_photo_projects_persisted_history_and_snapshots() {
        let path = test_path("round-trip");
        let photo = PhotoId::new(763).expect("photo");
        let edit = edit(photo);
        let mut history = HistoryState::new(photo);
        let mut repository = RedbHistoryRepository::open(&path, photo).expect("history repo");
        append_payload(
            &mut history,
            edit.clone(),
            HistoryOperationKind::Parameter,
            None,
            None,
            "Exposure",
            &mut repository,
        )
        .expect("append");
        apply_history_command(
            &mut history,
            HistoryCommand::CreateSnapshot {
                name: "Before compare".to_owned(),
            },
            &mut repository,
        )
        .expect("snapshot");
        drop(repository);
        let mut controller = GtkDarkroomPanelController::new(Some(path.clone()));
        let target = DarkroomPanelTarget::new(
            photo,
            rusttable_ui::ViewportGeneration::new(11),
            Revision::from_u64(3),
        );
        let mut catalog = RedbCatalogRepository::open(&path).expect("catalog");
        catalog.commit_new(&edit).expect("catalog edit");
        drop(catalog);
        let projections = controller.select_photo(target).expect("select");
        assert_eq!(projections.history().state().ready_entries(), 1);
        assert_eq!(projections.snapshots().state().ready_entries(), 1);
        fs::remove_file(path).expect("cleanup");
    }

    #[test]
    fn stale_generation_is_rejected_before_canonical_dispatch() {
        let mut controller = GtkDarkroomPanelController::new(None);
        let target = DarkroomPanelTarget::new(
            PhotoId::new(1).expect("photo"),
            rusttable_ui::ViewportGeneration::new(1),
            Revision::ZERO,
        );
        let result = controller.apply(&DarkroomPanelAction::NavigateHistory {
            target,
            revision: Revision::ZERO,
            direction: HistoryDirection::Previous,
        });
        assert!(matches!(
            result,
            Err(DarkroomPanelControllerError::NoSelection)
        ));
    }

    trait ReadyEntries {
        fn ready_entries(&self) -> usize;
    }

    impl ReadyEntries for DarkroomPanelState<DarkroomHistoryViewModel> {
        fn ready_entries(&self) -> usize {
            match self {
                Self::Ready(value) => value.entries().count(),
                _ => 0,
            }
        }
    }

    impl ReadyEntries for DarkroomPanelState<DarkroomSnapshotsViewModel> {
        fn ready_entries(&self) -> usize {
            match self {
                Self::Ready(value) => value.entries().count(),
                _ => 0,
            }
        }
    }
}
