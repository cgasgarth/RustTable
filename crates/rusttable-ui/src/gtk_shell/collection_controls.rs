//! GTK4 controls for a single Darktable collection rule.

use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::PhotoId;

use crate::collection::CollectionProperty;

/// State projected into the collection controls after a catalog refresh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionControlState {
    property: CollectionProperty,
    search_text: String,
    total_count: usize,
    result_count: usize,
}

/// Complete collection projection used to refresh controls and the lighttable together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionFilterState {
    controls: CollectionControlState,
    matching_photo_ids: Vec<PhotoId>,
}

impl CollectionFilterState {
    /// Creates a filter projection from its control state and matching catalog IDs.
    #[must_use]
    pub fn new(controls: CollectionControlState, matching_photo_ids: Vec<PhotoId>) -> Self {
        Self {
            controls,
            matching_photo_ids,
        }
    }

    /// Returns the values shown by the collection controls.
    #[must_use]
    pub const fn controls(&self) -> &CollectionControlState {
        &self.controls
    }

    /// Returns matching photo IDs in catalog order.
    #[must_use]
    pub fn matching_photo_ids(&self) -> &[PhotoId] {
        &self.matching_photo_ids
    }
}

impl CollectionControlState {
    /// Creates control state with an empty search.
    #[must_use]
    pub const fn new(property: CollectionProperty, total_count: usize) -> Self {
        Self {
            property,
            search_text: String::new(),
            total_count,
            result_count: total_count,
        }
    }

    /// Returns the active property.
    #[must_use]
    pub const fn property(&self) -> CollectionProperty {
        self.property
    }

    /// Returns the search text.
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }

    /// Returns the imported-record count before filtering.
    #[must_use]
    pub const fn total_count(&self) -> usize {
        self.total_count
    }

    /// Returns the filtered-record count.
    #[must_use]
    pub const fn result_count(&self) -> usize {
        self.result_count
    }

    /// Returns a copy with updated search and result counts.
    #[must_use]
    pub fn with_results(mut self, search_text: impl Into<String>, result_count: usize) -> Self {
        self.search_text = search_text.into();
        self.result_count = result_count;
        self
    }
}

/// Typed events emitted by the GTK collection controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionControlAction {
    /// The user selected a different collection property.
    SetProperty(CollectionProperty),
    /// The user changed the collection search text.
    SetSearchText(String),
    /// The user pressed the clear action.
    Clear,
}

/// GTK4 property dropdown, search entry, clear action, and result count.
#[derive(Clone)]
pub struct CollectionControls {
    root: gtk4::Box,
    property_dropdown: gtk4::DropDown,
    search_entry: gtk4::SearchEntry,
    clear_button: gtk4::Button,
    result_count: gtk4::Label,
}

impl CollectionControls {
    /// Builds one Darktable-shaped collection rule row.
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        root.set_widget_name("collection-controls");

        let rule_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
        rule_row.set_widget_name("collection-rule");

        let property_dropdown =
            gtk4::DropDown::from_strings(&CollectionProperty::ALL.map(CollectionProperty::label));
        property_dropdown.set_widget_name("collection-property");
        property_dropdown.set_selected(CollectionProperty::default().index());

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_widget_name("collection-search");
        search_entry.set_hexpand(true);
        search_entry.set_placeholder_text(Some("search collection"));

        let clear_button = gtk4::Button::with_label("clear");
        clear_button.set_widget_name("collection-clear");

        rule_row.append(&property_dropdown);
        rule_row.append(&search_entry);
        rule_row.append(&clear_button);

        let result_count = gtk4::Label::new(Some("0 of 0"));
        result_count.set_widget_name("collection-result-count");
        result_count.set_xalign(0.0);

        root.append(&rule_row);
        root.append(&result_count);

        Self {
            root,
            property_dropdown,
            search_entry,
            clear_button,
            result_count,
        }
    }

    /// Returns the root widget for insertion into a GTK panel.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Projects controller state into the GTK controls.
    pub fn set_state(&self, state: &CollectionControlState) {
        self.property_dropdown
            .set_selected(state.property().index());
        self.search_entry.set_text(state.search_text());
        self.result_count.set_text(&format!(
            "{} of {}",
            state.result_count(),
            state.total_count()
        ));
    }

    /// Connects all user actions to one typed callback.
    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(CollectionControlAction) + 'static,
    {
        let callback = Rc::new(callback);

        let property_callback = Rc::clone(&callback);
        self.property_dropdown
            .connect_selected_notify(move |dropdown| {
                if let Some(property) = CollectionProperty::from_index(dropdown.selected()) {
                    property_callback(CollectionControlAction::SetProperty(property));
                }
            });

        let search_callback = Rc::clone(&callback);
        self.search_entry.connect_search_changed(move |entry| {
            search_callback(CollectionControlAction::SetSearchText(
                entry.text().to_string(),
            ));
        });

        self.clear_button.connect_clicked(move |_| {
            callback(CollectionControlAction::Clear);
        });
    }
}

impl Default for CollectionControls {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::collection::CollectionProperty;

    use super::{CollectionControlAction, CollectionControlState};

    #[test]
    fn state_preserves_counts_and_rule_values() {
        let state =
            CollectionControlState::new(CollectionProperty::Folders, 12).with_results("2026", 5);

        assert_eq!(state.property(), CollectionProperty::Folders);
        assert_eq!(state.search_text(), "2026");
        assert_eq!(state.total_count(), 12);
        assert_eq!(state.result_count(), 5);
    }

    #[test]
    fn actions_are_typed_for_runtime_integration() {
        assert_eq!(
            CollectionControlAction::SetProperty(CollectionProperty::Filmroll),
            CollectionControlAction::SetProperty(CollectionProperty::Filmroll)
        );
        assert_eq!(
            CollectionControlAction::SetSearchText("holiday".to_owned()),
            CollectionControlAction::SetSearchText("holiday".to_owned())
        );
        assert_eq!(
            CollectionControlAction::Clear,
            CollectionControlAction::Clear
        );
    }
}
