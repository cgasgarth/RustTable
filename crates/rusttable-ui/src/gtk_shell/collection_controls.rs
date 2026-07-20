//! GTK4 controls for a single Darktable collection rule.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use gtk4::prelude::*;
use rusttable_core::PhotoId;
use rusttable_i18n::{I18n, MessageArgs, MessageId};

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
    photo_states: BTreeMap<PhotoId, LighttablePhotoState>,
    toolbar: LighttableToolbarState,
}

use super::{LighttableColorLabel, LighttableRating, LighttableToolbarState};

/// Controller-owned organization and selection state for one visible thumbnail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LighttablePhotoState {
    photo_id: PhotoId,
    selected: bool,
    rating: LighttableRating,
    color_labels: BTreeSet<LighttableColorLabel>,
}

impl LighttablePhotoState {
    #[must_use]
    pub fn new(
        photo_id: PhotoId,
        selected: bool,
        rating: LighttableRating,
        color_labels: impl IntoIterator<Item = LighttableColorLabel>,
    ) -> Self {
        Self {
            photo_id,
            selected,
            rating,
            color_labels: color_labels.into_iter().collect(),
        }
    }

    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }
    #[must_use]
    pub const fn selected(&self) -> bool {
        self.selected
    }
    #[must_use]
    pub const fn rating(&self) -> LighttableRating {
        self.rating
    }
    #[must_use]
    pub fn color_labels(&self) -> impl ExactSizeIterator<Item = LighttableColorLabel> + '_ {
        self.color_labels.iter().copied()
    }
}

impl CollectionFilterState {
    /// Creates a filter projection from its control state and matching catalog IDs.
    #[must_use]
    pub fn new(controls: CollectionControlState, matching_photo_ids: Vec<PhotoId>) -> Self {
        let toolbar = LighttableToolbarState::new(controls.total_count()).with_filter(
            controls.property(),
            controls.search_text(),
            controls.result_count(),
        );
        Self {
            controls,
            matching_photo_ids,
            photo_states: BTreeMap::new(),
            toolbar,
        }
    }

    #[must_use]
    pub fn with_lighttable_state(
        mut self,
        photo_states: impl IntoIterator<Item = LighttablePhotoState>,
        toolbar: LighttableToolbarState,
    ) -> Self {
        self.photo_states = photo_states
            .into_iter()
            .map(|state| (state.photo_id(), state))
            .collect();
        self.toolbar = toolbar;
        self
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

    #[must_use]
    pub fn photo_state(&self, photo_id: PhotoId) -> Option<&LighttablePhotoState> {
        self.photo_states.get(&photo_id)
    }

    #[must_use]
    pub const fn toolbar(&self) -> &LighttableToolbarState {
        &self.toolbar
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
    property_model: gtk4::StringList,
    search_entry: gtk4::SearchEntry,
    clear_button: gtk4::Button,
    result_count: gtk4::Label,
    locale: Rc<RefCell<I18n>>,
    state: Rc<RefCell<CollectionControlState>>,
    projecting: Rc<Cell<bool>>,
}

impl CollectionControls {
    /// Builds one Darktable-shaped collection rule row.
    #[must_use]
    pub fn new() -> Self {
        Self::with_i18n(I18n::default())
    }

    /// Builds collection controls with an explicit initial locale.
    #[must_use]
    pub fn with_i18n(i18n: I18n) -> Self {
        let locale = Rc::new(RefCell::new(i18n));
        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
        root.set_widget_name("collection-controls");
        root.set_width_request(0);

        let rule_row = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
        rule_row.set_widget_name("collection-rule");

        let property_model = gtk4::StringList::new(&[]);
        let property_labels = CollectionProperty::ALL.map(|property| {
            locale
                .borrow()
                .text(property.message_id(), &MessageArgs::new())
        });
        let property_label_refs = property_labels
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        property_model.splice(0, 0, &property_label_refs);
        let property_dropdown =
            gtk4::DropDown::new(Some(property_model.clone()), None::<&gtk4::Expression>);
        property_dropdown.set_widget_name("collection-property");
        property_dropdown.set_selected(CollectionProperty::default().index());
        property_dropdown.set_hexpand(true);
        property_dropdown.set_width_request(0);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_widget_name("collection-search");
        search_entry.set_hexpand(true);
        search_entry.set_width_chars(1);
        search_entry.set_max_width_chars(9);
        search_entry.set_placeholder_text(Some(
            &locale
                .borrow()
                .text(MessageId::CollectionSearch, &MessageArgs::new()),
        ));

        let clear_button = gtk4::Button::with_label("×");
        clear_button.set_widget_name("collection-clear");
        clear_button.set_tooltip_text(Some(
            &locale
                .borrow()
                .text(MessageId::CollectionClear, &MessageArgs::new()),
        ));

        rule_row.append(&property_dropdown);
        let search_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        search_row.set_widget_name("collection-search-row");
        search_row.append(&search_entry);
        search_row.append(&clear_button);
        rule_row.append(&search_row);

        let result_count = gtk4::Label::new(Some(&locale.borrow().text(
            MessageId::CollectionResults,
            &MessageArgs::new().integer("result", 0).integer("total", 0),
        )));
        result_count.set_widget_name("collection-result-count");
        result_count.set_xalign(0.0);

        root.append(&rule_row);
        root.append(&result_count);

        Self {
            root,
            property_dropdown,
            property_model,
            search_entry,
            clear_button,
            result_count,
            locale: Rc::clone(&locale),
            state: Rc::new(RefCell::new(CollectionControlState::new(
                CollectionProperty::default(),
                0,
            ))),
            projecting: Rc::new(Cell::new(false)),
        }
    }

    /// Returns the root widget for insertion into a GTK panel.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Projects controller state into the GTK controls.
    pub fn set_state(&self, state: &CollectionControlState) {
        self.projecting.set(true);
        self.state.replace(state.clone());
        self.property_dropdown
            .set_selected(state.property().index());
        self.search_entry.set_text(state.search_text());
        self.result_count.set_text(
            &self.locale.borrow().text(
                MessageId::CollectionResults,
                &MessageArgs::new()
                    .integer(
                        "result",
                        i64::try_from(state.result_count()).unwrap_or(i64::MAX),
                    )
                    .integer(
                        "total",
                        i64::try_from(state.total_count()).unwrap_or(i64::MAX),
                    ),
            ),
        );
        self.projecting.set(false);
    }

    /// Applies a locale change to ordinary collection UI state without changing the rule.
    pub fn set_locale(&self, i18n: I18n) {
        self.locale.replace(i18n);
        let i18n = self.locale.borrow();
        let labels = CollectionProperty::ALL.map(|property| property.localized_label(&i18n));
        let label_refs = labels.iter().map(String::as_str).collect::<Vec<_>>();
        self.property_model
            .splice(0, self.property_model.n_items(), &label_refs);
        self.search_entry.set_placeholder_text(Some(
            &i18n.text(MessageId::CollectionSearch, &MessageArgs::new()),
        ));
        self.clear_button.set_tooltip_text(Some(
            &i18n.text(MessageId::CollectionClear, &MessageArgs::new()),
        ));
        self.set_state(&self.state.borrow().clone());
    }

    /// Connects all user actions to one typed callback.
    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(CollectionControlAction) + 'static,
    {
        let callback = Rc::new(callback);

        let property_callback = Rc::clone(&callback);
        let projecting = Rc::clone(&self.projecting);
        self.property_dropdown
            .connect_selected_notify(move |dropdown| {
                if !projecting.get()
                    && let Some(property) = CollectionProperty::from_index(dropdown.selected())
                {
                    property_callback(CollectionControlAction::SetProperty(property));
                }
            });

        let search_callback = Rc::clone(&callback);
        let projecting = Rc::clone(&self.projecting);
        self.search_entry.connect_search_changed(move |entry| {
            if !projecting.get() {
                search_callback(CollectionControlAction::SetSearchText(
                    entry.text().to_string(),
                ));
            }
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
