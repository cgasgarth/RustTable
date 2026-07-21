//! Typed projections for Darktable's darkroom left-center panels.

use std::{fmt, rc::Rc};

use gtk4::accessible::Property;
use gtk4::prelude::*;
use rusttable_core::{PhotoId, Revision};

use crate::viewport_presentation::ViewportGeneration;

use super::{
    DarkroomHistoryViewModel, HistoryDirection, PhotoFactViewModel, PresentationText,
    PresentationTextError,
};

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

/// One immutable Darktable snapshot slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomSnapshotEntry {
    id: u64,
    label: PresentationText,
    status: PresentationText,
}

impl DarkroomSnapshotEntry {
    /// Creates a snapshot row with the status shown beside its label.
    ///
    /// # Errors
    ///
    /// Returns a presentation-text validation error for an invalid label or status.
    pub fn new(
        id: u64,
        label: impl Into<String>,
        status: impl Into<String>,
    ) -> Result<Self, PresentationTextError> {
        Ok(Self {
            id,
            label: PresentationText::new(label)?,
            status: PresentationText::new(status)?,
        })
    }

    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub fn label(&self) -> &PresentationText {
        &self.label
    }

    #[must_use]
    pub fn status(&self) -> &PresentationText {
        &self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomSnapshotsViewModel {
    entries: Vec<DarkroomSnapshotEntry>,
    selected: Option<u64>,
    side_by_side: bool,
}

impl DarkroomSnapshotsViewModel {
    /// Builds a snapshot list while rejecting duplicate stable ids.
    ///
    /// # Errors
    ///
    /// Returns a duplicate or unknown snapshot error.
    pub fn new(
        entries: Vec<DarkroomSnapshotEntry>,
        selected: Option<u64>,
        side_by_side: bool,
    ) -> Result<Self, DarkroomPanelError> {
        for (index, entry) in entries.iter().enumerate() {
            if entries[..index]
                .iter()
                .any(|previous| previous.id() == entry.id())
            {
                return Err(DarkroomPanelError::DuplicateSnapshot(entry.id()));
            }
        }
        if selected.is_some_and(|id| !entries.iter().any(|entry| entry.id() == id)) {
            return Err(DarkroomPanelError::UnknownSnapshot(
                selected.unwrap_or_default(),
            ));
        }
        Ok(Self {
            entries,
            selected,
            side_by_side,
        })
    }

    #[must_use]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &DarkroomSnapshotEntry> {
        self.entries.iter()
    }

    #[must_use]
    pub const fn selected(&self) -> Option<u64> {
        self.selected
    }

    #[must_use]
    pub const fn side_by_side(&self) -> bool {
        self.side_by_side
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomImageInformationViewModel {
    title: PresentationText,
    facts: Vec<PhotoFactViewModel>,
    unsupported: Vec<PresentationText>,
}

impl DarkroomImageInformationViewModel {
    /// Keeps unavailable metadata explicit instead of fabricating values.
    ///
    /// # Errors
    ///
    /// Returns a presentation-text validation error for an invalid title or field label.
    pub fn new(
        title: impl Into<String>,
        facts: Vec<PhotoFactViewModel>,
        unsupported: Vec<String>,
    ) -> Result<Self, PresentationTextError> {
        Ok(Self {
            title: PresentationText::new(title)?,
            facts,
            unsupported: unsupported
                .into_iter()
                .map(PresentationText::new)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }

    #[must_use]
    pub fn title(&self) -> &PresentationText {
        &self.title
    }

    #[must_use]
    pub fn facts(&self) -> impl ExactSizeIterator<Item = &PhotoFactViewModel> {
        self.facts.iter()
    }

    #[must_use]
    pub fn unsupported(&self) -> impl ExactSizeIterator<Item = &PresentationText> {
        self.unsupported.iter()
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

#[must_use]
pub fn build_history_panel(
    projection: &DarkroomPanelProjection<DarkroomHistoryViewModel>,
    handler: Option<DarkroomPanelActionHandler>,
) -> gtk4::Expander {
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    match projection.state() {
        DarkroomPanelState::Empty => append_status(&body, "select a photo to view edit history"),
        DarkroomPanelState::Loading => append_status(&body, "loading edit history…"),
        DarkroomPanelState::Error(error) => {
            append_status(&body, &format!("Error · {}", error.as_str()));
        }
        DarkroomPanelState::Ready(history) => {
            let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let previous = panel_button("history-previous", "Previous");
            let next = panel_button("history-next", "Next");
            actions.append(&previous);
            actions.append(&next);
            body.append(&actions);
            let entries = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
            for (index, entry) in history.entries().enumerate() {
                let button = panel_button(
                    &format!("history-entry-{}", entry.id().get()),
                    entry.label().as_str(),
                );
                if history.selected_index() == Some(index) {
                    button.add_css_class("selected");
                }
                if let (Some(handler), Some(target)) = (handler.clone(), projection.target()) {
                    let revision = projection.revision();
                    button.connect_clicked(move |_| {
                        handler(DarkroomPanelAction::SelectHistory {
                            target,
                            revision,
                            index,
                        });
                    });
                }
                entries.append(&button);
            }
            body.append(&entries);
            if let (Some(handler), Some(target)) = (handler, projection.target()) {
                connect_history_button(
                    &previous,
                    handler.clone(),
                    target,
                    projection.revision(),
                    true,
                );
                connect_history_button(&next, handler, target, projection.revision(), false);
            }
        }
    }
    panel_expander("darkroom-history", "history", projection.expanded(), &body)
}

#[must_use]
pub fn build_snapshots_panel(
    projection: &DarkroomPanelProjection<DarkroomSnapshotsViewModel>,
    handler: Option<DarkroomPanelActionHandler>,
) -> gtk4::Expander {
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    match projection.state() {
        DarkroomPanelState::Empty => append_status(&body, "select a photo to view snapshots"),
        DarkroomPanelState::Loading => append_status(&body, "loading snapshots…"),
        DarkroomPanelState::Error(error) => {
            append_status(&body, &format!("Error · {}", error.as_str()));
        }
        DarkroomPanelState::Ready(snapshots) => {
            let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
            let take = panel_button("snapshot-take", "Take snapshot");
            take.set_sensitive(false);
            let compare = gtk4::CheckButton::with_label("Side by side");
            compare.set_widget_name("snapshot-side-by-side");
            compare.set_active(snapshots.side_by_side());
            toolbar.append(&take);
            toolbar.append(&compare);
            body.append(&toolbar);
            for entry in snapshots.entries() {
                let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
                let select = panel_button(
                    &format!("snapshot-entry-{}", entry.id()),
                    entry.label().as_str(),
                );
                select.set_hexpand(true);
                let restore = panel_button(&format!("snapshot-restore-{}", entry.id()), "Restore");
                let status = gtk4::Label::new(Some(entry.status().as_str()));
                status.add_css_class("dim-label");
                row.append(&select);
                row.append(&status);
                row.append(&restore);
                if let (Some(handler), Some(target)) = (handler.clone(), projection.target()) {
                    let revision = projection.revision();
                    let id = entry.id();
                    select.connect_clicked({
                        let handler = handler.clone();
                        move |_| {
                            handler(DarkroomPanelAction::SelectSnapshot {
                                target,
                                revision,
                                id,
                            });
                        }
                    });
                    restore.connect_clicked(move |_| {
                        handler(DarkroomPanelAction::RestoreSnapshot {
                            target,
                            revision,
                            id,
                        });
                    });
                }
                body.append(&row);
            }
            if let (Some(handler), Some(target)) = (handler, projection.target()) {
                let revision = projection.revision();
                compare.connect_toggled(move |compare| {
                    handler(DarkroomPanelAction::ToggleSnapshotCompare {
                        target,
                        revision,
                        enabled: compare.is_active(),
                    });
                });
            }
        }
    }
    panel_expander(
        "darkroom-snapshots",
        "snapshots",
        projection.expanded(),
        &body,
    )
}

#[must_use]
pub fn build_image_information_panel(
    projection: &DarkroomPanelProjection<DarkroomImageInformationViewModel>,
) -> gtk4::Expander {
    let body = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    match projection.state() {
        DarkroomPanelState::Empty => append_status(&body, "image information unavailable"),
        DarkroomPanelState::Loading => append_status(&body, "loading image information…"),
        DarkroomPanelState::Error(error) => {
            append_status(&body, &format!("Error · {}", error.as_str()));
        }
        DarkroomPanelState::Ready(info) => {
            let title = gtk4::Label::new(Some(info.title().as_str()));
            title.set_halign(gtk4::Align::Start);
            title.add_css_class("title-4");
            body.append(&title);
            for fact in info.facts() {
                append_fact(&body, fact.label().as_str(), fact.value().as_str());
            }
            for unsupported in info.unsupported() {
                append_fact(&body, unsupported.as_str(), "Unavailable");
            }
        }
    }
    panel_expander(
        "darkroom-image-information",
        "image information",
        projection.expanded(),
        &body,
    )
}

fn connect_history_button(
    button: &gtk4::Button,
    handler: DarkroomPanelActionHandler,
    target: DarkroomPanelTarget,
    revision: Revision,
    previous: bool,
) {
    button.connect_clicked(move |_| {
        handler(DarkroomPanelAction::NavigateHistory {
            target,
            revision,
            direction: if previous {
                HistoryDirection::Previous
            } else {
                HistoryDirection::Next
            },
        });
    });
}

fn panel_expander(id: &str, title: &str, open: bool, body: &gtk4::Box) -> gtk4::Expander {
    let expander = gtk4::Expander::builder()
        .label(title)
        .expanded(open)
        .child(body)
        .build();
    expander.set_widget_name(id);
    expander.set_focusable(true);
    expander.set_accessible_role(gtk4::AccessibleRole::Group);
    expander.update_property(&[Property::Label(title)]);
    expander
}

fn panel_button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button.set_focusable(true);
    button.update_property(&[Property::Label(label)]);
    button
}

fn append_status(body: &gtk4::Box, text: &str) {
    let status = gtk4::Label::new(Some(text));
    status.set_halign(gtk4::Align::Start);
    status.add_css_class("dim-label");
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    body.append(&status);
}

fn append_fact(body: &gtk4::Box, label: &str, value: &str) {
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
    use super::super::{DarkroomHistoryEntry, HistoryEntryId};
    use super::*;

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
    fn snapshot_projection_rejects_duplicate_and_unknown_ids() {
        let one = DarkroomSnapshotEntry::new(1, "A", "ready").expect("snapshot");
        let duplicate = DarkroomSnapshotEntry::new(1, "B", "ready").expect("snapshot");
        assert!(matches!(
            DarkroomSnapshotsViewModel::new(vec![one.clone(), duplicate], None, false),
            Err(DarkroomPanelError::DuplicateSnapshot(1))
        ));
        assert!(matches!(
            DarkroomSnapshotsViewModel::new(vec![one], Some(9), false),
            Err(DarkroomPanelError::UnknownSnapshot(9))
        ));
    }
}
