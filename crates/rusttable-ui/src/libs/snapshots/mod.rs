//! Darkroom snapshot models and their GTK4 left-rail projection.

use gtk4::prelude::*;

use crate::libs::panel::{
    DarkroomPanelAction, DarkroomPanelActionHandler, DarkroomPanelError, DarkroomPanelProjection,
    DarkroomPanelState, append_status, panel_button, panel_expander,
};
use crate::presentation::{PresentationText, PresentationTextError};

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
            let compare = gtk4::CheckButton::with_label("Before / after");
            compare.set_widget_name("snapshot-side-by-side");
            compare.set_active(snapshots.side_by_side());
            toolbar.append(&take);
            toolbar.append(&compare);
            body.append(&toolbar);
            if snapshots.entries().next().is_none() {
                append_status(&body, "no saved snapshots for this photo");
            }
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
                let name = format!("Snapshot {}", revision.get().saturating_add(1));
                take.set_sensitive(true);
                take.connect_clicked({
                    let handler = handler.clone();
                    move |_| {
                        handler(DarkroomPanelAction::CreateSnapshot {
                            target,
                            revision,
                            name: name.clone(),
                        });
                    }
                });
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

#[cfg(test)]
mod tests {
    use super::*;

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
