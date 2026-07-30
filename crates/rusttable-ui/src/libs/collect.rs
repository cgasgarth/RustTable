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
    status: CollectionStatus,
    generation: u64,
}

/// Bounded states the navigator can show while a collection projection is refreshed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectionStatus {
    Ready,
    Empty,
    Loading,
    Error,
}

impl CollectionStatus {
    const fn for_counts(total_count: usize, result_count: usize) -> Self {
        if total_count == 0 || result_count == 0 {
            Self::Empty
        } else {
            Self::Ready
        }
    }

    const fn message(self) -> Option<&'static str> {
        match self {
            Self::Ready => None,
            Self::Empty => Some("no images match this collection"),
            Self::Loading => Some("loading collection…"),
            Self::Error => Some("unable to load this collection"),
        }
    }
}

/// Complete collection projection used to refresh controls and the lighttable together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionFilterState {
    controls: CollectionControlState,
    matching_photo_ids: Vec<PhotoId>,
    photo_states: BTreeMap<PhotoId, LighttablePhotoState>,
    toolbar: LighttableToolbarState,
}

use crate::gui::{LighttableColorLabel, LighttableRating, LighttableToolbarState};

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
        let selected_count = self
            .photo_states
            .values()
            .filter(|state| state.selected())
            .count();
        let selected_rating = toolbar.selected_rating();
        let selected_labels = toolbar.selected_labels().collect::<Vec<_>>();
        self.toolbar = toolbar.with_selection(selected_count, selected_rating, selected_labels);
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

    /// Returns organization and selection state for every projected photo.
    #[must_use]
    pub fn photo_states(&self) -> impl ExactSizeIterator<Item = &LighttablePhotoState> {
        self.photo_states.values()
    }

    /// Returns selected IDs from the same projection used to render the grid and filmstrip.
    pub fn selected_photo_ids(&self) -> impl Iterator<Item = PhotoId> + '_ {
        self.photo_states
            .values()
            .filter(|state| state.selected())
            .map(LighttablePhotoState::photo_id)
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
            status: CollectionStatus::for_counts(total_count, total_count),
            generation: 0,
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
        self.status = CollectionStatus::for_counts(self.total_count, result_count);
        self
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// Returns a loading projection without exposing catalog internals to GTK.
    #[must_use]
    pub fn loading(mut self) -> Self {
        self.status = CollectionStatus::Loading;
        self
    }

    /// Returns an error projection with a bounded user-facing message.
    #[must_use]
    pub fn failed(mut self) -> Self {
        self.status = CollectionStatus::Error;
        self
    }

    #[must_use]
    fn status_message(&self) -> Option<&'static str> {
        self.status.message()
    }
}

/// Typed events emitted by the GTK collection controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollectionControlAction {
    /// The user selected a different collection property.
    SetProperty {
        property: CollectionProperty,
        generation: u64,
    },
    /// The user changed the collection search text.
    SetSearchText {
        search_text: String,
        generation: u64,
    },
    /// The user pressed the clear action.
    Clear { generation: u64 },
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
    status: gtk4::Label,
    locale: Rc<RefCell<I18n>>,
    state: Rc<RefCell<CollectionControlState>>,
    projecting: Rc<Cell<bool>>,
    generation: Rc<Cell<u64>>,
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
        property_dropdown.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        property_dropdown.set_tooltip_text(Some("choose the collection property"));
        property_dropdown.set_selected(CollectionProperty::default().index());
        property_dropdown.set_hexpand(true);
        property_dropdown.set_width_request(0);

        let search_entry = gtk4::SearchEntry::new();
        search_entry.set_widget_name("collection-search");
        search_entry.set_accessible_role(gtk4::AccessibleRole::SearchBox);
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
        clear_button.set_accessible_role(gtk4::AccessibleRole::Button);
        clear_button.update_property(&[gtk4::accessible::Property::Label("clear collection rule")]);
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
        result_count.add_css_class("dim-label");
        result_count.set_accessible_role(gtk4::AccessibleRole::Status);

        let status = gtk4::Label::new(None);
        status.set_widget_name("collection-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.add_css_class("dt_collection_status");
        status.set_visible(false);

        root.append(&rule_row);
        root.append(&result_count);
        root.append(&status);

        Self {
            root,
            property_dropdown,
            property_model,
            search_entry,
            clear_button,
            result_count,
            status,
            locale: Rc::clone(&locale),
            state: Rc::new(RefCell::new(CollectionControlState::new(
                CollectionProperty::default(),
                0,
            ))),
            projecting: Rc::new(Cell::new(false)),
            generation: Rc::new(Cell::new(0)),
        }
    }

    /// Returns the root widget for insertion into a GTK panel.
    #[must_use]
    pub fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    /// Projects controller state into the GTK controls.
    pub fn set_state(&self, state: &CollectionControlState) {
        if state.generation() < self.generation.get() {
            return;
        }
        self.projecting.set(true);
        self.state.replace(state.clone());
        self.generation.set(state.generation());
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
        if let Some(message) = state.status_message() {
            self.status.set_text(message);
            self.status.set_visible(true);
        } else {
            self.status.set_visible(false);
        }
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
        let generation = Rc::clone(&self.generation);
        self.property_dropdown
            .connect_selected_notify(move |dropdown| {
                if !projecting.get()
                    && let Some(property) = CollectionProperty::from_index(dropdown.selected())
                {
                    let next = generation.get().saturating_add(1);
                    generation.set(next);
                    property_callback(CollectionControlAction::SetProperty {
                        property,
                        generation: next,
                    });
                }
            });

        let search_callback = Rc::clone(&callback);
        let projecting = Rc::clone(&self.projecting);
        let generation = Rc::clone(&self.generation);
        self.search_entry.connect_search_changed(move |entry| {
            if !projecting.get() {
                let next = generation.get().saturating_add(1);
                generation.set(next);
                search_callback(CollectionControlAction::SetSearchText {
                    search_text: entry.text().to_string(),
                    generation: next,
                });
            }
        });

        let generation = Rc::clone(&self.generation);
        self.clear_button.connect_clicked(move |_| {
            let next = generation.get().saturating_add(1);
            generation.set(next);
            callback(CollectionControlAction::Clear { generation: next });
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

    use rusttable_core::PhotoId;

    use super::{
        CollectionControlAction, CollectionControlState, CollectionFilterState,
        LighttablePhotoState,
    };
    use crate::gtk_shell::{LighttableColorLabel, LighttableRating, LighttableToolbarState};

    #[test]
    fn state_preserves_counts_and_rule_values() {
        let state =
            CollectionControlState::new(CollectionProperty::Folders, 12).with_results("2026", 5);

        assert_eq!(state.property(), CollectionProperty::Folders);
        assert_eq!(state.search_text(), "2026");
        assert_eq!(state.total_count(), 12);
        assert_eq!(state.result_count(), 5);
        assert_eq!(state.status_message(), None);
    }

    #[test]
    fn collection_state_is_bounded_for_empty_loading_and_error_projections() {
        let empty =
            CollectionControlState::new(CollectionProperty::Filename, 4).with_results("missing", 0);
        assert_eq!(
            empty.status_message(),
            Some("no images match this collection")
        );

        let loading = CollectionControlState::new(CollectionProperty::Filename, 4).loading();
        assert_eq!(loading.status_message(), Some("loading collection…"));

        let failed = CollectionControlState::new(CollectionProperty::Filename, 4).failed();
        assert_eq!(
            failed.status_message(),
            Some("unable to load this collection")
        );
        assert!(!failed.status_message().unwrap_or_default().contains('/'));
    }

    #[test]
    fn actions_are_typed_for_runtime_integration() {
        assert_eq!(
            CollectionControlAction::SetProperty {
                property: CollectionProperty::Filmroll,
                generation: 1,
            },
            CollectionControlAction::SetProperty {
                property: CollectionProperty::Filmroll,
                generation: 1,
            }
        );
        assert_eq!(
            CollectionControlAction::SetSearchText {
                search_text: "holiday".to_owned(),
                generation: 2,
            },
            CollectionControlAction::SetSearchText {
                search_text: "holiday".to_owned(),
                generation: 2,
            }
        );
        assert_eq!(
            CollectionControlAction::Clear { generation: 3 },
            CollectionControlAction::Clear { generation: 3 }
        );
    }

    #[test]
    fn collection_projection_generation_is_monotonic() {
        let newer = CollectionControlState::new(CollectionProperty::Filename, 2).with_generation(4);
        let older = CollectionControlState::new(CollectionProperty::Filename, 1).with_generation(3);
        assert_eq!(newer.generation(), 4);
        assert!(older.generation() < newer.generation());
    }

    #[test]
    fn filter_projection_reconciles_selection_from_photo_states() {
        let first = PhotoId::new(1).expect("non-zero photo ID");
        let second = PhotoId::new(2).expect("non-zero photo ID");
        let controls = CollectionControlState::new(CollectionProperty::Filename, 2);
        let state = CollectionFilterState::new(controls, vec![first, second])
            .with_lighttable_state(
                [
                    LighttablePhotoState::new(
                        first,
                        true,
                        LighttableRating::Four,
                        [LighttableColorLabel::Blue],
                    ),
                    LighttablePhotoState::new(second, false, LighttableRating::Zero, []),
                ],
                LighttableToolbarState::new(2).with_selection(2, Some(LighttableRating::Five), []),
            );

        assert_eq!(state.selected_photo_ids().collect::<Vec<_>>(), vec![first]);
        assert_eq!(state.toolbar().selected_count(), 1);
        assert_eq!(
            state.toolbar().selected_rating(),
            Some(LighttableRating::Five)
        );
    }
}
