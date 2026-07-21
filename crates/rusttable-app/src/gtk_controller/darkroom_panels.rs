//! Controller-owned projections for the darkroom history and snapshot rails.

use std::fmt;
use std::path::PathBuf;

use rusttable_catalog::EditRepository;
use rusttable_catalog_store::RedbCatalogRepository;
use rusttable_core::{Edit, Revision};
use rusttable_ui::presentation::{
    DarkroomHistoryAction, DarkroomHistoryEntry, DarkroomHistoryError, DarkroomHistoryViewModel,
    DarkroomPanelAction, DarkroomPanelError, DarkroomPanelId, DarkroomPanelProjection,
    DarkroomPanelRouter, DarkroomPanelState, DarkroomPanelTarget, DarkroomSnapshotEntry,
    DarkroomSnapshotsViewModel, HistoryDirection, HistoryEntryId,
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

/// Application-side owner of the selected photo's history/snapshot projection.
#[derive(Debug, Clone)]
pub struct GtkDarkroomPanelController {
    catalog_path: Option<PathBuf>,
    target: Option<DarkroomPanelTarget>,
    router: DarkroomPanelRouter,
    projections: Option<DarkroomPanelProjections>,
}

impl GtkDarkroomPanelController {
    #[must_use]
    pub fn new(catalog_path: Option<PathBuf>) -> Self {
        Self {
            catalog_path,
            target: None,
            router: DarkroomPanelRouter::default(),
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

    /// Loads the selected photo's persisted edit and projects its operation stack.
    ///
    /// Named snapshot persistence is owned by the history backend tracked in #285. Until that
    /// backend exists, a selected photo receives a real, controller-owned empty snapshot model;
    /// it is never represented as an unavailable/error placeholder.
    ///
    /// # Errors
    ///
    /// Returns a typed persistence or presentation error and leaves the previous projection
    /// untouched so a caller can render a recoverable error for the requested target.
    pub fn select_photo(
        &mut self,
        target: DarkroomPanelTarget,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let repository = self.open_repository()?;
        let edit = repository
            .list()
            .map_err(|error| DarkroomPanelControllerError::Persistence(error.to_string()))?
            .into_iter()
            .filter(|edit| edit.photo_id() == target.photo_id())
            .max_by_key(|edit| (edit.revision(), edit.id()));
        let edit_revision = edit.as_ref().map_or(Revision::ZERO, Edit::revision);
        let history = project_history(edit.as_ref(), edit_revision)?;
        let snapshots = DarkroomSnapshotsViewModel::new(Vec::new(), None, false)
            .map_err(DarkroomPanelControllerError::Panel)?;
        self.router = DarkroomPanelRouter::default();
        self.router
            .reconcile(target, edit_revision)
            .map_err(DarkroomPanelControllerError::Panel)?;
        let projections = DarkroomPanelProjections {
            history: DarkroomPanelProjection::ready(target, edit_revision, history),
            snapshots: DarkroomPanelProjection::ready(target, edit_revision, snapshots),
        };
        self.target = Some(target);
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    /// Rebinds the current models to a new preview generation without losing selection.
    ///
    /// Preview refreshes intentionally create a new generation. Rebinding keeps callbacks from
    /// the old generation stale while preserving the selected history/snapshot row.
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
        let projections = self
            .projections
            .clone()
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let revision = self.router.revision();
        self.router = DarkroomPanelRouter::default();
        self.router
            .reconcile(target, revision)
            .map_err(DarkroomPanelControllerError::Panel)?;
        let projections = DarkroomPanelProjections {
            history: rebind_projection(projections.history(), target),
            snapshots: rebind_projection(projections.snapshots(), target),
        };
        self.target = Some(target);
        self.projections = Some(projections.clone());
        Ok(projections)
    }

    /// Applies one typed rail action against a cloned model and commits it only after the router
    /// accepts the target and revision. This prevents a stale callback from mutating UI state.
    ///
    /// # Errors
    ///
    /// Returns a stale, boundary, persistence, or projection error without changing state.
    #[allow(clippy::too_many_lines)]
    pub fn apply(
        &mut self,
        action: &DarkroomPanelAction,
    ) -> Result<DarkroomPanelProjections, DarkroomPanelControllerError> {
        let current = self
            .projections
            .clone()
            .ok_or(DarkroomPanelControllerError::NoSelection)?;
        let mut candidate_router = self.router;
        match action {
            DarkroomPanelAction::SelectHistory {
                target,
                revision,
                index,
            } => {
                let DarkroomPanelState::Ready(mut history) = current.history().state().clone()
                else {
                    return Err(DarkroomPanelControllerError::NotReady("history"));
                };
                let next = history.apply(DarkroomHistoryAction::Select {
                    expected_revision: *revision,
                    index: *index,
                })?;
                let projection = DarkroomPanelProjection::ready(*target, next, history)
                    .with_view_state(
                        current.history().expanded(),
                        current.history().scroll_position_milli(),
                    );
                candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = DarkroomPanelProjections {
                    history: projection,
                    snapshots: set_projection_revision(
                        rebind_projection(current.snapshots(), *target),
                        next,
                    ),
                };
                Ok(self.commit(candidate_router, projections))
            }
            DarkroomPanelAction::NavigateHistory {
                target,
                revision,
                direction,
            } => {
                let DarkroomPanelState::Ready(mut history) = current.history().state().clone()
                else {
                    return Err(DarkroomPanelControllerError::NotReady("history"));
                };
                let next = history.apply(history_action_for_direction(*revision, *direction))?;
                let projection = DarkroomPanelProjection::ready(*target, next, history)
                    .with_view_state(
                        current.history().expanded(),
                        current.history().scroll_position_milli(),
                    );
                candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = DarkroomPanelProjections {
                    history: projection,
                    snapshots: set_projection_revision(
                        rebind_projection(current.snapshots(), *target),
                        next,
                    ),
                };
                Ok(self.commit(candidate_router, projections))
            }
            DarkroomPanelAction::SelectSnapshot {
                target,
                revision: _,
                id,
            }
            | DarkroomPanelAction::RestoreSnapshot {
                target,
                revision: _,
                id,
            } => {
                let DarkroomPanelState::Ready(snapshots) = current.snapshots().state() else {
                    return Err(DarkroomPanelControllerError::NotReady("snapshots"));
                };
                if !snapshots.entries().any(|entry| entry.id() == *id) {
                    return Err(DarkroomPanelControllerError::Panel(
                        DarkroomPanelError::UnknownSnapshot(*id),
                    ));
                }
                let snapshots = DarkroomSnapshotsViewModel::new(
                    snapshots.entries().cloned().collect::<Vec<_>>(),
                    Some(*id),
                    snapshots.side_by_side(),
                )
                .map_err(DarkroomPanelControllerError::Panel)?;
                let next = candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = DarkroomPanelProjections {
                    history: set_projection_revision(
                        rebind_projection(current.history(), *target),
                        next,
                    ),
                    snapshots: DarkroomPanelProjection::ready(*target, next, snapshots)
                        .with_view_state(
                            current.snapshots().expanded(),
                            current.snapshots().scroll_position_milli(),
                        ),
                };
                Ok(self.commit(candidate_router, projections))
            }
            DarkroomPanelAction::ToggleSnapshotCompare {
                target,
                revision: _,
                enabled,
            } => {
                let DarkroomPanelState::Ready(snapshots) = current.snapshots().state() else {
                    return Err(DarkroomPanelControllerError::NotReady("snapshots"));
                };
                let next = snapshots
                    .entries()
                    .cloned()
                    .collect::<Vec<DarkroomSnapshotEntry>>();
                let snapshots =
                    DarkroomSnapshotsViewModel::new(next, snapshots.selected(), *enabled)
                        .map_err(DarkroomPanelControllerError::Panel)?;
                candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = DarkroomPanelProjections {
                    history: set_projection_revision(
                        rebind_projection(current.history(), *target),
                        candidate_router.revision(),
                    ),
                    snapshots: DarkroomPanelProjection::ready(
                        *target,
                        candidate_router.revision(),
                        snapshots,
                    )
                    .with_view_state(
                        current.snapshots().expanded(),
                        current.snapshots().scroll_position_milli(),
                    ),
                };
                Ok(self.commit(candidate_router, projections))
            }
            DarkroomPanelAction::SetExpanded {
                panel,
                target,
                revision: _,
                expanded,
            } => {
                candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = set_expanded(
                    &current,
                    *panel,
                    *target,
                    *expanded,
                    candidate_router.revision(),
                );
                Ok(self.commit(candidate_router, projections))
            }
            DarkroomPanelAction::SetScroll {
                panel,
                target,
                revision: _,
                position_milli,
            } => {
                candidate_router
                    .route(action)
                    .map_err(DarkroomPanelControllerError::Panel)?;
                let projections = set_scroll(
                    &current,
                    *panel,
                    *target,
                    *position_milli,
                    candidate_router.revision(),
                );
                Ok(self.commit(candidate_router, projections))
            }
        }
    }

    fn commit(
        &mut self,
        router: DarkroomPanelRouter,
        projections: DarkroomPanelProjections,
    ) -> DarkroomPanelProjections {
        self.router = router;
        self.target = router.target();
        self.projections = Some(projections.clone());
        projections
    }

    fn open_repository(&self) -> Result<RedbCatalogRepository, DarkroomPanelControllerError> {
        let path = self
            .catalog_path
            .as_deref()
            .ok_or(DarkroomPanelControllerError::NoCatalog)?;
        RedbCatalogRepository::open(path)
            .map_err(|error| DarkroomPanelControllerError::Persistence(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomPanelControllerError {
    NoCatalog,
    NoSelection,
    NotReady(&'static str),
    Persistence(String),
    History(DarkroomHistoryError),
    Panel(DarkroomPanelError),
    Presentation(String),
}

impl fmt::Display for DarkroomPanelControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCatalog => formatter.write_str("catalog path is unavailable"),
            Self::NoSelection => formatter.write_str("no darkroom photo is selected"),
            Self::NotReady(panel) => write!(formatter, "{panel} projection is not ready"),
            Self::Persistence(message) | Self::Presentation(message) => {
                formatter.write_str(message)
            }
            Self::History(error) => error.fmt(formatter),
            Self::Panel(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DarkroomPanelControllerError {}

impl From<DarkroomHistoryError> for DarkroomPanelControllerError {
    fn from(error: DarkroomHistoryError) -> Self {
        Self::History(error)
    }
}

fn project_history(
    edit: Option<&Edit>,
    revision: Revision,
) -> Result<DarkroomHistoryViewModel, DarkroomPanelControllerError> {
    let entries = edit
        .into_iter()
        .flat_map(Edit::operations)
        .enumerate()
        .map(|(index, operation)| {
            let label = operation.key().as_str().to_owned();
            let id = u64::try_from(index)
                .ok()
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| {
                    DarkroomPanelControllerError::Presentation(
                        "edit history contains too many operations".to_owned(),
                    )
                })?;
            DarkroomHistoryEntry::new(HistoryEntryId::from_u64(id), label)
                .map_err(|error| DarkroomPanelControllerError::Presentation(format!("{error:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let selected = entries.len().checked_sub(1);
    DarkroomHistoryViewModel::new(revision, entries, selected)
        .map_err(DarkroomPanelControllerError::History)
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
            result.history = set_projection_revision(result.history, revision);
        }
        DarkroomPanelId::Snapshots => {
            result.snapshots = rebind_projection(current.snapshots(), target)
                .with_view_state(expanded, current.snapshots().scroll_position_milli());
            result.snapshots = set_projection_revision(result.snapshots, revision);
        }
        DarkroomPanelId::ImageInformation => {}
    }
    result
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
            result.history = set_projection_revision(result.history, revision);
        }
        DarkroomPanelId::Snapshots => {
            result.snapshots = rebind_projection(current.snapshots(), target)
                .with_view_state(current.snapshots().expanded(), position_milli);
            result.snapshots = set_projection_revision(result.snapshots, revision);
        }
        DarkroomPanelId::ImageInformation => {}
    }
    result
}

fn set_projection_revision<T: Clone>(
    projection: DarkroomPanelProjection<T>,
    revision: Revision,
) -> DarkroomPanelProjection<T> {
    match projection.state() {
        DarkroomPanelState::Empty => projection,
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

fn history_action_for_direction(
    expected_revision: Revision,
    direction: HistoryDirection,
) -> DarkroomHistoryAction {
    match direction {
        HistoryDirection::Previous => DarkroomHistoryAction::Previous { expected_revision },
        HistoryDirection::Next => DarkroomHistoryAction::Next { expected_revision },
    }
}

#[cfg(test)]
mod tests {
    use rusttable_core::{
        EditId, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
        ParameterValue, PhotoId,
    };

    use super::*;

    fn edit() -> Edit {
        Edit::from_parts(
            EditId::new(1).expect("edit id"),
            PhotoId::new(2).expect("photo id"),
            Revision::ZERO,
            Revision::from_u64(4),
            [
                Operation::new_with_opacity(
                    OperationId::new(1).expect("operation id"),
                    OperationKey::new("darktable.exposure").expect("operation key"),
                    true,
                    OperationOpacity::ONE,
                    [(
                        ParameterName::new("stops").expect("parameter"),
                        ParameterValue::Integer(1),
                    )],
                )
                .expect("operation"),
                Operation::new_with_opacity(
                    OperationId::new(2).expect("operation id"),
                    OperationKey::new("darktable.colorbalance").expect("operation key"),
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
    fn project_history_preserves_operation_order_and_current_selection() {
        let history = project_history(Some(&edit()), Revision::from_u64(4)).expect("history");
        assert_eq!(history.entries().count(), 2);
        assert_eq!(
            history
                .entries()
                .map(|entry| entry.label().as_str())
                .collect::<Vec<_>>(),
            ["darktable.exposure", "darktable.colorbalance"]
        );
        assert_eq!(history.selected_index(), Some(1));
        assert_eq!(history.revision(), Revision::from_u64(4));
    }

    #[test]
    fn photos_without_edits_have_an_explicit_empty_history_model() {
        let history = project_history(None, Revision::ZERO).expect("history");
        assert_eq!(history.entries().count(), 0);
        assert_eq!(history.selected_index(), None);
    }

    #[test]
    fn stale_history_action_is_rejected_without_changing_the_projection() {
        let target = DarkroomPanelTarget::new(
            PhotoId::new(2).expect("photo id"),
            rusttable_ui::ViewportGeneration::new(7),
            Revision::from_u64(4),
        );
        let history = project_history(Some(&edit()), Revision::from_u64(4)).expect("history");
        let snapshots =
            DarkroomSnapshotsViewModel::new(Vec::new(), None, false).expect("snapshots");
        let projections = DarkroomPanelProjections {
            history: DarkroomPanelProjection::ready(target, Revision::from_u64(4), history),
            snapshots: DarkroomPanelProjection::ready(target, Revision::from_u64(4), snapshots),
        };
        let mut controller = GtkDarkroomPanelController {
            catalog_path: None,
            target: Some(target),
            router: {
                let mut router = DarkroomPanelRouter::default();
                router
                    .reconcile(target, Revision::from_u64(4))
                    .expect("target");
                router
            },
            projections: Some(projections.clone()),
        };
        let stale = DarkroomPanelAction::SelectHistory {
            target,
            revision: Revision::from_u64(3),
            index: 0,
        };
        assert!(matches!(
            controller.apply(&stale),
            Err(
                DarkroomPanelControllerError::History(DarkroomHistoryError::StaleRevision { .. },)
                    | DarkroomPanelControllerError::Panel(DarkroomPanelError::StaleRevision { .. },)
            )
        ));
        assert_eq!(controller.projections(), Some(&projections));
    }
}
