//! Darkroom history-stack presentation and GTK4 projection.
//!
//! History navigation is a snapshot operation: every widget callback carries
//! the revision it observed, and stale callbacks are rejected before they can
//! move the selected history entry.

use std::{cell::RefCell, fmt, rc::Rc};

use gtk4::prelude::*;
use rusttable_core::Revision;

use crate::libs::panel::{
    DarkroomPanelAction, DarkroomPanelActionHandler, DarkroomPanelProjection, DarkroomPanelState,
    append_status, panel_button, panel_expander,
};
use crate::presentation::{PresentationText, PresentationTextError};

/// Stable identity for one visible history entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HistoryEntryId(u64);

impl HistoryEntryId {
    #[must_use]
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// One named edit snapshot in the history rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomHistoryEntry {
    id: HistoryEntryId,
    label: PresentationText,
}

impl DarkroomHistoryEntry {
    /// Creates a history entry with a stable id and display-safe label.
    ///
    /// # Errors
    ///
    /// Returns the presentation-text validation error for an invalid label.
    pub fn new(
        id: HistoryEntryId,
        label: impl Into<String>,
    ) -> Result<Self, PresentationTextError> {
        Ok(Self {
            id,
            label: PresentationText::new(label)?,
        })
    }

    #[must_use]
    pub const fn id(&self) -> HistoryEntryId {
        self.id
    }

    #[must_use]
    pub const fn label(&self) -> &PresentationText {
        &self.label
    }
}

/// A revision-safe history action emitted by GTK callbacks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DarkroomHistoryAction {
    Select {
        expected_revision: Revision,
        index: usize,
    },
    Previous {
        expected_revision: Revision,
    },
    Next {
        expected_revision: Revision,
    },
}

impl DarkroomHistoryAction {
    #[must_use]
    pub const fn expected_revision(&self) -> Revision {
        match self {
            Self::Select {
                expected_revision, ..
            }
            | Self::Previous { expected_revision }
            | Self::Next { expected_revision } => *expected_revision,
        }
    }
}

/// Why a history-stack action could not be applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomHistoryError {
    StaleRevision {
        expected: Revision,
        actual: Revision,
    },
    EntryIndexOutOfRange {
        index: usize,
        entries: usize,
    },
    NavigationBoundary {
        direction: HistoryDirection,
    },
    DuplicateEntryId(HistoryEntryId),
    SnapshotRevisionRewind {
        current: Revision,
        replacement: Revision,
    },
    InvalidSelection {
        selected: usize,
        entries: usize,
    },
    RevisionOverflow,
    InvalidEntry(PresentationTextError),
}

impl fmt::Display for DarkroomHistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleRevision { expected, actual } => {
                write!(
                    formatter,
                    "stale history callback: expected {expected}, current {actual}"
                )
            }
            Self::EntryIndexOutOfRange { index, entries } => {
                write!(
                    formatter,
                    "history entry {index} is outside {entries} entries"
                )
            }
            Self::NavigationBoundary { direction } => {
                write!(
                    formatter,
                    "cannot navigate {direction:?} in the history stack"
                )
            }
            Self::DuplicateEntryId(id) => {
                write!(formatter, "duplicate history entry id {}", id.get())
            }
            Self::SnapshotRevisionRewind {
                current,
                replacement,
            } => write!(
                formatter,
                "history snapshot revision {replacement} is older than current {current}"
            ),
            Self::InvalidSelection { selected, entries } => {
                write!(
                    formatter,
                    "history selection {selected} is outside {entries} entries"
                )
            }
            Self::RevisionOverflow => formatter.write_str("history revision counter overflowed"),
            Self::InvalidEntry(error) => write!(formatter, "invalid history entry: {error:?}"),
        }
    }
}

impl std::error::Error for DarkroomHistoryError {}

/// Direction used for history navigation errors and headless contracts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryDirection {
    Previous,
    Next,
}

/// Last-known state of the history stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DarkroomHistoryStatus {
    Ready,
    Stale {
        expected: Revision,
        actual: Revision,
    },
    Error(DarkroomHistoryError),
}

/// Revisioned, ordered history snapshots for the darkroom rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DarkroomHistoryViewModel {
    revision: Revision,
    entries: Vec<DarkroomHistoryEntry>,
    selected: Option<usize>,
    status: DarkroomHistoryStatus,
}

impl DarkroomHistoryViewModel {
    /// Builds a history snapshot and preserves entry order.
    ///
    /// # Errors
    ///
    /// Returns an error for duplicate entry ids or an invalid selected index.
    pub fn new(
        revision: Revision,
        entries: Vec<DarkroomHistoryEntry>,
        selected: Option<usize>,
    ) -> Result<Self, DarkroomHistoryError> {
        validate_entries(&entries)?;
        validate_selection(selected, entries.len())?;
        Ok(Self {
            revision,
            entries,
            selected,
            status: DarkroomHistoryStatus::Ready,
        })
    }

    #[must_use]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    #[must_use]
    pub const fn status(&self) -> &DarkroomHistoryStatus {
        &self.status
    }

    #[must_use]
    pub const fn selected_index(&self) -> Option<usize> {
        self.selected
    }

    #[must_use]
    pub fn current(&self) -> Option<&DarkroomHistoryEntry> {
        self.selected.and_then(|index| self.entries.get(index))
    }

    #[must_use = "iterate over history entries in deterministic display order"]
    pub fn entries(&self) -> impl ExactSizeIterator<Item = &DarkroomHistoryEntry> {
        self.entries.iter()
    }

    /// Returns the stable widget names used for keyboard focus order.
    #[must_use]
    pub fn focus_order(&self) -> impl ExactSizeIterator<Item = String> + '_ {
        self.entries
            .iter()
            .map(|entry| format!("history-entry-{}", entry.id().get()))
    }

    /// Applies a revision-safe navigation action.
    ///
    /// # Errors
    ///
    /// Returns a stale, out-of-range, boundary, or revision-overflow error.
    pub fn apply(
        &mut self,
        action: DarkroomHistoryAction,
    ) -> Result<Revision, DarkroomHistoryError> {
        self.check_revision(action.expected_revision())?;
        match action {
            DarkroomHistoryAction::Select { index, .. } => self.select(index),
            DarkroomHistoryAction::Previous { .. } => self.navigate(HistoryDirection::Previous),
            DarkroomHistoryAction::Next { .. } => self.navigate(HistoryDirection::Next),
        }
    }

    /// Appends a new snapshot and discards redo entries after the current one.
    ///
    /// # Errors
    ///
    /// Returns a stale, duplicate-id, or revision-overflow error.
    pub fn push(
        &mut self,
        expected_revision: Revision,
        entry: DarkroomHistoryEntry,
    ) -> Result<Revision, DarkroomHistoryError> {
        self.check_revision(expected_revision)?;
        if self
            .entries
            .iter()
            .any(|existing| existing.id() == entry.id())
        {
            return Err(self.record_error(DarkroomHistoryError::DuplicateEntryId(entry.id())));
        }
        if let Some(selected) = self.selected {
            self.entries.truncate(selected.saturating_add(1));
        }
        self.entries.push(entry);
        self.selected = Some(self.entries.len().saturating_sub(1));
        self.advance_revision()
    }

    /// Reconciles a stale widget snapshot from the owning controller.
    ///
    /// # Errors
    ///
    /// Returns an error when the replacement revision moves backward, entries
    /// are duplicated, or the selected index is invalid.
    pub fn reconcile_snapshot(
        &mut self,
        revision: Revision,
        entries: Vec<DarkroomHistoryEntry>,
        selected: Option<usize>,
    ) -> Result<(), DarkroomHistoryError> {
        if revision < self.revision {
            return Err(
                self.record_error(DarkroomHistoryError::SnapshotRevisionRewind {
                    current: self.revision,
                    replacement: revision,
                }),
            );
        }
        validate_entries(&entries).map_err(|error| self.record_error(error))?;
        validate_selection(selected, entries.len()).map_err(|error| self.record_error(error))?;
        self.revision = revision;
        self.entries = entries;
        self.selected = selected;
        self.status = DarkroomHistoryStatus::Ready;
        Ok(())
    }

    #[must_use]
    pub fn status_text(&self) -> String {
        match &self.status {
            DarkroomHistoryStatus::Ready => format!("Ready · revision {}", self.revision),
            DarkroomHistoryStatus::Stale { expected, actual } => {
                format!("Stale callback · refresh required (expected {expected}, current {actual})")
            }
            DarkroomHistoryStatus::Error(error) => format!("History error · {error}"),
        }
    }

    fn select(&mut self, index: usize) -> Result<Revision, DarkroomHistoryError> {
        if index >= self.entries.len() {
            return Err(
                self.record_error(DarkroomHistoryError::EntryIndexOutOfRange {
                    index,
                    entries: self.entries.len(),
                }),
            );
        }
        self.selected = Some(index);
        self.advance_revision()
    }

    fn navigate(&mut self, direction: HistoryDirection) -> Result<Revision, DarkroomHistoryError> {
        let Some(selected) = self.selected else {
            return Err(self.record_error(DarkroomHistoryError::NavigationBoundary { direction }));
        };
        let target = match direction {
            HistoryDirection::Previous => selected.checked_sub(1),
            HistoryDirection::Next => selected
                .checked_add(1)
                .filter(|index| *index < self.entries.len()),
        };
        let Some(target) = target else {
            return Err(self.record_error(DarkroomHistoryError::NavigationBoundary { direction }));
        };
        self.selected = Some(target);
        self.advance_revision()
    }

    fn check_revision(&mut self, expected: Revision) -> Result<(), DarkroomHistoryError> {
        if expected != self.revision {
            let error = DarkroomHistoryError::StaleRevision {
                expected,
                actual: self.revision,
            };
            self.status = DarkroomHistoryStatus::Stale {
                expected,
                actual: self.revision,
            };
            return Err(error);
        }
        Ok(())
    }

    fn advance_revision(&mut self) -> Result<Revision, DarkroomHistoryError> {
        let next = self
            .revision
            .checked_increment()
            .map_err(|_| self.record_error(DarkroomHistoryError::RevisionOverflow))?;
        self.revision = next;
        self.status = DarkroomHistoryStatus::Ready;
        Ok(next)
    }

    fn record_error(&mut self, error: DarkroomHistoryError) -> DarkroomHistoryError {
        self.status = DarkroomHistoryStatus::Error(error.clone());
        error
    }
}

/// Callback type used by the GTK4 history-stack projection.
pub type DarkroomHistoryActionHandler =
    Rc<dyn Fn(DarkroomHistoryAction) -> Result<Revision, DarkroomHistoryError>>;

/// Builds the Darktable-style history stack with stable keyboard traversal.
#[must_use]
pub fn build_history_stack(
    history: &DarkroomHistoryViewModel,
    action_handler: Option<DarkroomHistoryActionHandler>,
) -> gtk4::Box {
    let current_revision = Rc::new(RefCell::new(history.revision()));
    let stack = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    stack.set_widget_name("history-stack");
    stack.set_accessible_role(gtk4::AccessibleRole::Group);
    let title = gtk4::Label::new(Some("History stack"));
    title.set_halign(gtk4::Align::Start);
    stack.append(&title);

    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    let previous = history_button("history-previous", "Previous");
    let next = history_button("history-next", "Next");
    actions.append(&previous);
    actions.append(&next);
    stack.append(&actions);

    let status = gtk4::Label::new(Some(&history.status_text()));
    status.set_widget_name("history-status");
    status.set_halign(gtk4::Align::Start);
    status.set_accessible_role(gtk4::AccessibleRole::Status);
    stack.append(&status);

    let entries = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    entries.set_widget_name("history-entries");
    for (index, entry) in history.entries().enumerate() {
        let button = history_button(
            &format!("history-entry-{}", entry.id().get()),
            entry.label().as_str(),
        );
        if history.selected_index() == Some(index) {
            button.add_css_class("selected");
        }
        if let Some(handler) = action_handler.clone() {
            let status_for_entry = status.clone();
            let revision_for_entry = current_revision.clone();
            button.connect_clicked(move |_| {
                dispatch_history_action(
                    &handler,
                    &status_for_entry,
                    &revision_for_entry,
                    DarkroomHistoryAction::Select {
                        expected_revision: *revision_for_entry.borrow(),
                        index,
                    },
                );
            });
        }
        entries.append(&button);
    }
    stack.append(&entries);

    if let Some(handler) = action_handler {
        connect_history_navigation(
            &previous,
            handler.clone(),
            status.clone(),
            current_revision.clone(),
            HistoryDirection::Previous,
        );
        connect_history_navigation(
            &next,
            handler,
            status,
            current_revision,
            HistoryDirection::Next,
        );
    }
    stack
}

fn history_button(id: &str, label: &str) -> gtk4::Button {
    let button = gtk4::Button::with_label(label);
    button.set_widget_name(id);
    button.set_focus_on_click(false);
    button
}

fn connect_history_navigation(
    button: &gtk4::Button,
    handler: DarkroomHistoryActionHandler,
    status: gtk4::Label,
    current_revision: Rc<RefCell<Revision>>,
    direction: HistoryDirection,
) {
    button.connect_clicked(move |_| {
        let expected_revision = *current_revision.borrow();
        let action = match direction {
            HistoryDirection::Previous => DarkroomHistoryAction::Previous { expected_revision },
            HistoryDirection::Next => DarkroomHistoryAction::Next { expected_revision },
        };
        dispatch_history_action(&handler, &status, &current_revision, action);
    });
}

fn dispatch_history_action(
    handler: &DarkroomHistoryActionHandler,
    status: &gtk4::Label,
    current_revision: &RefCell<Revision>,
    action: DarkroomHistoryAction,
) {
    match handler(action) {
        Ok(revision) => {
            *current_revision.borrow_mut() = revision;
            status.set_label(&format!("Ready · revision {revision}"));
        }
        Err(error) => status.set_label(&format!("History error · {error}")),
    }
}

/// Creates a revision-safe history handler backed by a history model.
#[must_use]
pub fn history_action_handler(
    model: Rc<RefCell<DarkroomHistoryViewModel>>,
) -> DarkroomHistoryActionHandler {
    Rc::new(move |action| model.borrow_mut().apply(action))
}

fn validate_entries(entries: &[DarkroomHistoryEntry]) -> Result<(), DarkroomHistoryError> {
    for (index, entry) in entries.iter().enumerate() {
        if entries[..index]
            .iter()
            .any(|previous| previous.id() == entry.id())
        {
            return Err(DarkroomHistoryError::DuplicateEntryId(entry.id()));
        }
    }
    Ok(())
}

fn validate_selection(selected: Option<usize>, entries: usize) -> Result<(), DarkroomHistoryError> {
    if let Some(selected) = selected
        && selected >= entries
    {
        return Err(DarkroomHistoryError::InvalidSelection { selected, entries });
    }
    Ok(())
}

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
            let previous = panel_button("history-previous", "Undo");
            let next = panel_button("history-next", "Redo");
            let branch = panel_button("history-branch", "New branch");
            actions.append(&previous);
            actions.append(&next);
            actions.append(&branch);
            body.append(&actions);
            let entries = gtk4::Box::new(gtk4::Orientation::Vertical, 1);
            if history.entries().next().is_none() {
                append_status(&entries, "no edit history for this photo");
            }
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
                connect_history_button(
                    &next,
                    handler.clone(),
                    target,
                    projection.revision(),
                    false,
                );
                let revision = projection.revision();
                let name = format!("branch-{}", revision.get().saturating_add(1));
                branch.connect_clicked(move |_| {
                    handler(DarkroomPanelAction::CreateBranch {
                        target,
                        revision,
                        name: name.clone(),
                    });
                });
            }
        }
    }
    panel_expander("darkroom-history", "history", projection.expanded(), &body)
}

fn connect_history_button(
    button: &gtk4::Button,
    handler: DarkroomPanelActionHandler,
    target: crate::libs::panel::DarkroomPanelTarget,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, label: &str) -> DarkroomHistoryEntry {
        DarkroomHistoryEntry::new(HistoryEntryId::from_u64(id), label).expect("valid entry")
    }

    fn history() -> DarkroomHistoryViewModel {
        DarkroomHistoryViewModel::new(
            Revision::from_u64(3),
            vec![
                entry(10, "Original"),
                entry(11, "Exposure"),
                entry(12, "Crop"),
            ],
            Some(2),
        )
        .expect("valid history")
    }

    #[test]
    fn navigation_and_push_truncate_redo_in_display_order() {
        let mut model = history();
        let revision = model
            .apply(DarkroomHistoryAction::Previous {
                expected_revision: Revision::from_u64(3),
            })
            .expect("previous entry");
        assert_eq!(
            model.current().expect("current").label().as_str(),
            "Exposure"
        );
        let revision = model
            .push(revision, entry(13, "Tone"))
            .expect("append entry");
        assert_eq!(
            model
                .entries()
                .map(|entry| entry.id().get())
                .collect::<Vec<_>>(),
            [10, 11, 13]
        );
        assert_eq!(model.selected_index(), Some(2));
        assert_eq!(revision, Revision::from_u64(5));
    }

    #[test]
    fn stale_navigation_is_visible_and_reconciles_only_newer_snapshots() {
        let mut model = history();
        model
            .apply(DarkroomHistoryAction::Previous {
                expected_revision: Revision::from_u64(3),
            })
            .expect("previous entry");
        let error = model
            .apply(DarkroomHistoryAction::Previous {
                expected_revision: Revision::from_u64(3),
            })
            .expect_err("old callback is stale");
        assert!(matches!(error, DarkroomHistoryError::StaleRevision { .. }));
        assert!(model.status_text().starts_with("Stale callback"));
        let error = model
            .reconcile_snapshot(Revision::from_u64(3), vec![entry(10, "Original")], Some(0))
            .expect_err("older snapshot cannot recover stale state");
        assert!(matches!(
            error,
            DarkroomHistoryError::SnapshotRevisionRewind { .. }
        ));
        model
            .reconcile_snapshot(Revision::from_u64(5), vec![entry(10, "Original")], Some(0))
            .expect("newer snapshot recovers stale state");
        assert!(matches!(model.status(), DarkroomHistoryStatus::Ready));
    }

    #[test]
    fn focus_order_is_stable_and_matches_history_order() {
        let model = history();
        assert_eq!(
            model.focus_order().collect::<Vec<_>>(),
            ["history-entry-10", "history-entry-11", "history-entry-12"]
        );
    }
}
