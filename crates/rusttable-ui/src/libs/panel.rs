//! Typed projections for Darktable's darkroom left-center panels.

use std::{fmt, rc::Rc};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};

use crate::gui::darktable_components::module_title;
use crate::libs::history::HistoryDirection;
use crate::viewport_presentation::ViewportGeneration;

use crate::presentation::{PresentationText, PresentationTextError};

pub const DARKROOM_LEFT_PANEL_ORDER: [&str; 4] = [
    "darkroom-navigation",
    "darkroom-snapshots",
    "darkroom-history",
    "darkroom-image-information",
];

pub const DARKROOM_LEFT_PANEL_FOCUS_ORDER: [&str; 7] = [
    "darkroom-navigation",
    "darkroom-snapshots",
    "snapshot-take",
    "darkroom-history",
    "history-previous",
    "history-next",
    "darkroom-image-information",
];

/// The photo/edit generation a left-rail projection belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DarkroomPanelTarget {
    photo_id: PhotoId,
    generation: ViewportGeneration,
    edit_revision: Revision,
}

impl DarkroomPanelTarget {
    #[must_use]
    pub const fn new(
        photo_id: PhotoId,
        generation: ViewportGeneration,
        edit_revision: Revision,
    ) -> Self {
        Self {
            photo_id,
            generation,
            edit_revision,
        }
    }

    #[must_use]
    pub const fn photo_id(self) -> PhotoId {
        self.photo_id
    }

    #[must_use]
    pub const fn generation(self) -> ViewportGeneration {
        self.generation
    }

    #[must_use]
    pub const fn edit_revision(self) -> Revision {
        self.edit_revision
    }
}

/// A typed async projection state shared by all three panels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomPanelState<T> {
    Empty,
    Loading,
    Error(PresentationText),
    Ready(T),
}

/// Immutable projection plus the UI-only scroll and expansion state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomPanelProjection<T> {
    target: Option<DarkroomPanelTarget>,
    revision: Revision,
    expanded: bool,
    scroll_position_milli: u32,
    state: DarkroomPanelState<T>,
}

impl<T> DarkroomPanelProjection<T> {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            target: None,
            revision: Revision::ZERO,
            expanded: false,
            scroll_position_milli: 0,
            state: DarkroomPanelState::Empty,
        }
    }

    #[must_use]
    pub const fn loading(target: DarkroomPanelTarget, revision: Revision) -> Self {
        Self {
            target: Some(target),
            revision,
            expanded: false,
            scroll_position_milli: 0,
            state: DarkroomPanelState::Loading,
        }
    }

    /// Creates an error projection with bounded display text.
    ///
    /// # Errors
    ///
    /// Returns a presentation-text validation error for unsafe or oversized detail text.
    pub fn error(
        target: DarkroomPanelTarget,
        revision: Revision,
        detail: impl Into<String>,
    ) -> Result<Self, PresentationTextError> {
        Ok(Self {
            target: Some(target),
            revision,
            expanded: true,
            scroll_position_milli: 0,
            state: DarkroomPanelState::Error(PresentationText::new(detail)?),
        })
    }

    #[must_use]
    pub const fn ready(target: DarkroomPanelTarget, revision: Revision, value: T) -> Self {
        Self {
            target: Some(target),
            revision,
            expanded: true,
            scroll_position_milli: 0,
            state: DarkroomPanelState::Ready(value),
        }
    }

    #[must_use]
    pub const fn target(&self) -> Option<DarkroomPanelTarget> {
        self.target
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn expanded(&self) -> bool {
        self.expanded
    }

    #[must_use]
    pub const fn scroll_position_milli(&self) -> u32 {
        self.scroll_position_milli
    }

    #[must_use]
    pub const fn state(&self) -> &DarkroomPanelState<T> {
        &self.state
    }

    #[must_use]
    pub fn with_view_state(mut self, expanded: bool, scroll_position_milli: u32) -> Self {
        self.expanded = expanded;
        self.scroll_position_milli = scroll_position_milli;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomPanelError {
    DuplicateSnapshot(u64),
    UnknownSnapshot(u64),
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    StaleTarget {
        expected: DarkroomPanelTarget,
        actual: Option<DarkroomPanelTarget>,
    },
    RevisionOverflow,
    InvalidScrollPosition(u32),
}

impl fmt::Display for DarkroomPanelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateSnapshot(id) => write!(formatter, "duplicate snapshot {id}"),
            Self::UnknownSnapshot(id) => write!(formatter, "unknown snapshot {id}"),
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale panel revision: expected {expected}, current {actual}"
                )
            }
            Self::StaleTarget { .. } => formatter.write_str("stale photo generation"),
            Self::RevisionOverflow => formatter.write_str("panel revision overflowed"),
            Self::InvalidScrollPosition(value) => {
                write!(formatter, "invalid scroll position {value}")
            }
        }
    }
}

impl std::error::Error for DarkroomPanelError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomPanelId {
    Snapshots,
    History,
    ImageInformation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomPanelAction {
    SelectHistory {
        target: DarkroomPanelTarget,
        revision: Revision,
        index: usize,
    },
    NavigateHistory {
        target: DarkroomPanelTarget,
        revision: Revision,
        direction: HistoryDirection,
    },
    CreateBranch {
        target: DarkroomPanelTarget,
        revision: Revision,
        name: String,
    },
    CreateSnapshot {
        target: DarkroomPanelTarget,
        revision: Revision,
        name: String,
    },
    SelectSnapshot {
        target: DarkroomPanelTarget,
        revision: Revision,
        id: u64,
    },
    RestoreSnapshot {
        target: DarkroomPanelTarget,
        revision: Revision,
        id: u64,
    },
    ToggleSnapshotCompare {
        target: DarkroomPanelTarget,
        revision: Revision,
        enabled: bool,
    },
    SetExpanded {
        panel: DarkroomPanelId,
        target: DarkroomPanelTarget,
        revision: Revision,
        expanded: bool,
    },
    SetScroll {
        panel: DarkroomPanelId,
        target: DarkroomPanelTarget,
        revision: Revision,
        position_milli: u32,
    },
}

impl DarkroomPanelAction {
    fn target(&self) -> DarkroomPanelTarget {
        match self {
            Self::SelectHistory { target, .. }
            | Self::NavigateHistory { target, .. }
            | Self::CreateBranch { target, .. }
            | Self::CreateSnapshot { target, .. }
            | Self::SelectSnapshot { target, .. }
            | Self::RestoreSnapshot { target, .. }
            | Self::ToggleSnapshotCompare { target, .. }
            | Self::SetExpanded { target, .. }
            | Self::SetScroll { target, .. } => *target,
        }
    }

    fn revision(&self) -> Revision {
        match self {
            Self::SelectHistory { revision, .. }
            | Self::NavigateHistory { revision, .. }
            | Self::CreateBranch { revision, .. }
            | Self::CreateSnapshot { revision, .. }
            | Self::SelectSnapshot { revision, .. }
            | Self::RestoreSnapshot { revision, .. }
            | Self::ToggleSnapshotCompare { revision, .. }
            | Self::SetExpanded { revision, .. }
            | Self::SetScroll { revision, .. } => *revision,
        }
    }
}

/// Application-boundary guard for panel commands; it never mutates catalog state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DarkroomPanelRouter {
    target: Option<DarkroomPanelTarget>,
    revision: Revision,
}

impl Default for DarkroomPanelRouter {
    fn default() -> Self {
        Self {
            target: None,
            revision: Revision::ZERO,
        }
    }
}

impl DarkroomPanelRouter {
    #[must_use]
    pub const fn target(&self) -> Option<DarkroomPanelTarget> {
        self.target
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Replaces the current selected-photo projection with a same-or-newer revision.
    ///
    /// # Errors
    ///
    /// Returns a stale-revision error when a same-target older projection arrives.
    pub fn reconcile(
        &mut self,
        target: DarkroomPanelTarget,
        revision: Revision,
    ) -> Result<(), DarkroomPanelError> {
        if self.target == Some(target) && revision < self.revision {
            return Err(DarkroomPanelError::StaleRevision {
                expected: self.revision,
                actual: revision,
            });
        }
        self.target = Some(target);
        self.revision = revision;
        Ok(())
    }

    /// Accepts only actions for the currently selected photo generation.
    /// Accepts a panel action only for the current target and revision.
    ///
    /// # Errors
    ///
    /// Returns stale-target, stale-revision, invalid-scroll, or overflow errors without routing.
    pub fn route(&mut self, action: &DarkroomPanelAction) -> Result<Revision, DarkroomPanelError> {
        if self.target != Some(action.target()) {
            return Err(DarkroomPanelError::StaleTarget {
                expected: action.target(),
                actual: self.target,
            });
        }
        if action.revision() != self.revision {
            return Err(DarkroomPanelError::StaleRevision {
                expected: action.revision(),
                actual: self.revision,
            });
        }
        if let DarkroomPanelAction::SetScroll { position_milli, .. } = action
            && *position_milli > 1000
        {
            return Err(DarkroomPanelError::InvalidScrollPosition(*position_milli));
        }
        self.revision = self
            .revision
            .checked_increment()
            .map_err(|_| DarkroomPanelError::RevisionOverflow)?;
        Ok(self.revision)
    }
}

pub type DarkroomPanelActionHandler = Rc<dyn Fn(DarkroomPanelAction)>;

pub(crate) fn panel_expander(
    id: &str,
    title: &str,
    open: bool,
    body: &gtk4::Box,
) -> gtk4::Expander {
    let expander = gtk4::Expander::builder()
        .label(title)
        .expanded(open)
        .child(body)
        .build();
    expander.set_widget_name(id);
    expander.set_focusable(true);
    expander.set_accessible_role(gtk4::AccessibleRole::Group);
    expander.add_css_class("dt_module_expander");
    expander.set_label_widget(Some(&panel_title(id, title)));
    expander.update_property(&[Property::Label(title)]);
    expander
}

fn panel_title(id: &str, title: &str) -> gtk4::Box {
    module_title(id, title)
}

pub(crate) fn panel_button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button.set_focusable(true);
    button.update_property(&[Property::Label(label)]);
    button
}

pub(crate) fn append_status(body: &gtk4::Box, text: &str) {
    let status = gtk4::Label::new(Some(text));
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    body.append(&status);
}

pub(crate) fn append_fact(body: &gtk4::Box, label: &str, value: &str) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let label_widget = gtk4::Label::new(Some(label));
    label_widget.set_halign(gtk4::Align::Start);
    label_widget.set_hexpand(true);
    let value_widget = gtk4::Label::new(Some(value));
    value_widget.set_halign(gtk4::Align::End);
    row.append(&label_widget);
    row.append(&value_widget);
    body.append(&row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libs::history::{DarkroomHistoryEntry, DarkroomHistoryViewModel, HistoryEntryId};

    fn target(generation: u64) -> DarkroomPanelTarget {
        DarkroomPanelTarget::new(
            PhotoId::new(7).expect("photo"),
            ViewportGeneration::new(generation),
            Revision::from_u64(3),
        )
    }

    fn history() -> DarkroomHistoryViewModel {
        DarkroomHistoryViewModel::new(
            Revision::from_u64(3),
            vec![
                DarkroomHistoryEntry::new(HistoryEntryId::from_u64(1), "Original").expect("entry"),
            ],
            Some(0),
        )
        .expect("history")
    }

    #[test]
    fn every_async_state_is_explicit_and_ready_preserves_view_state() {
        let target = target(11);
        let projection = DarkroomPanelProjection::ready(target, Revision::from_u64(4), history())
            .with_view_state(true, 375);
        assert!(matches!(projection.state(), DarkroomPanelState::Ready(_)));
        assert_eq!(projection.scroll_position_milli(), 375);
        assert!(
            DarkroomPanelProjection::<DarkroomHistoryViewModel>::empty()
                .target()
                .is_none()
        );
    }

    #[test]
    fn stale_generation_and_revision_cannot_route_panel_actions() {
        let current = target(11);
        let mut router = DarkroomPanelRouter::default();
        router
            .reconcile(current, Revision::from_u64(4))
            .expect("current");
        let stale = DarkroomPanelAction::SelectHistory {
            target: target(10),
            revision: Revision::from_u64(4),
            index: 0,
        };
        assert!(matches!(
            router.route(&stale),
            Err(DarkroomPanelError::StaleTarget { .. })
        ));
        let stale_revision = DarkroomPanelAction::SelectHistory {
            target: current,
            revision: Revision::from_u64(3),
            index: 0,
        };
        assert!(matches!(
            router.route(&stale_revision),
            Err(DarkroomPanelError::StaleRevision { .. })
        ));
    }

    #[test]
    fn history_branch_and_snapshot_actions_share_the_stale_guard() {
        let current = target(11);
        let mut router = DarkroomPanelRouter::default();
        router
            .reconcile(current, Revision::from_u64(4))
            .expect("current");
        let branch = DarkroomPanelAction::CreateBranch {
            target: current,
            revision: Revision::from_u64(4),
            name: "branch-1".to_owned(),
        };
        assert!(router.route(&branch).is_ok());
        let snapshot = DarkroomPanelAction::CreateSnapshot {
            target: current,
            revision: Revision::from_u64(5),
            name: "Snapshot 1".to_owned(),
        };
        assert!(router.route(&snapshot).is_ok());
    }
}
