//! Typed GTK4 controls for Darktable's visible lighttable toolbar.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use gtk4::accessible::{Property, State};
use gtk4::prelude::*;

use crate::CollectionProperty;

use crate::gui::darktable_components::{button as shared_button, dropdown as shared_dropdown};
use crate::gui::{LIGHTTABLE_TOOLBAR, ThemeRole, apply_theme_role};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LighttableColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

impl LighttableColorLabel {
    pub const ALL: [Self; 5] = [
        Self::Red,
        Self::Yellow,
        Self::Green,
        Self::Blue,
        Self::Purple,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Red => 0,
            Self::Yellow => 1,
            Self::Green => 2,
            Self::Blue => 3,
            Self::Purple => 4,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LighttableRating {
    Rejected,
    Zero,
    One,
    Two,
    Three,
    Four,
    Five,
}

impl LighttableRating {
    pub const STARS: [Self; 5] = [Self::One, Self::Two, Self::Three, Self::Four, Self::Five];

    #[must_use]
    pub const fn stars(self) -> Option<u8> {
        match self {
            Self::Rejected => None,
            Self::Zero => Some(0),
            Self::One => Some(1),
            Self::Two => Some(2),
            Self::Three => Some(3),
            Self::Four => Some(4),
            Self::Five => Some(5),
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rejected => "rejected",
            Self::Zero => "zero stars",
            Self::One => "one star",
            Self::Two => "two stars",
            Self::Three => "three stars",
            Self::Four => "four stars",
            Self::Five => "five stars",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LighttableSort {
    #[default]
    Filename,
    CaptureTime,
    Rating,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LighttableSortDirection {
    #[default]
    Ascending,
    Descending,
}

impl LighttableSort {
    pub const ALL: [Self; 3] = [Self::Filename, Self::CaptureTime, Self::Rating];

    const fn index(self) -> u32 {
        match self {
            Self::Filename => 0,
            Self::CaptureTime => 1,
            Self::Rating => 2,
        }
    }

    fn from_index(index: u32) -> Option<Self> {
        usize::try_from(index)
            .ok()
            .and_then(|index| Self::ALL.get(index).copied())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LighttableToolbarAction {
    SetProperty(CollectionProperty),
    SetSearchText(String),
    SetSort(LighttableSort),
    SetSortDirection(LighttableSortDirection),
    SetRating(LighttableRating),
    ToggleColorLabel(LighttableColorLabel),
    ClearReset,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LighttableToolbarState {
    property: CollectionProperty,
    search_text: String,
    sort: LighttableSort,
    sort_direction: LighttableSortDirection,
    selected_count: usize,
    visible_count: usize,
    total_count: usize,
    selected_rating: Option<LighttableRating>,
    selected_labels: BTreeSet<LighttableColorLabel>,
}

impl LighttableToolbarState {
    #[must_use]
    pub fn new(total_count: usize) -> Self {
        Self {
            property: CollectionProperty::Filename,
            search_text: String::new(),
            sort: LighttableSort::Filename,
            sort_direction: LighttableSortDirection::Ascending,
            selected_count: 0,
            visible_count: total_count,
            total_count,
            selected_rating: None,
            selected_labels: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn with_filter(
        mut self,
        property: CollectionProperty,
        search_text: impl Into<String>,
        visible_count: usize,
    ) -> Self {
        self.property = property;
        self.search_text = search_text.into();
        self.visible_count = visible_count.min(self.total_count);
        self
    }

    #[must_use]
    pub const fn with_sort(mut self, sort: LighttableSort) -> Self {
        self.sort = sort;
        self
    }

    #[must_use]
    pub const fn with_sort_direction(mut self, direction: LighttableSortDirection) -> Self {
        self.sort_direction = direction;
        self
    }

    #[must_use]
    pub fn with_selection(
        mut self,
        selected_count: usize,
        rating: Option<LighttableRating>,
        labels: impl IntoIterator<Item = LighttableColorLabel>,
    ) -> Self {
        self.selected_count = selected_count.min(self.total_count);
        self.selected_rating = rating;
        self.selected_labels = labels.into_iter().collect();
        self
    }

    #[must_use]
    pub const fn property(&self) -> CollectionProperty {
        self.property
    }
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }
    #[must_use]
    pub const fn sort(&self) -> LighttableSort {
        self.sort
    }
    #[must_use]
    pub const fn sort_direction(&self) -> LighttableSortDirection {
        self.sort_direction
    }
    #[must_use]
    pub const fn selected_count(&self) -> usize {
        self.selected_count
    }
    #[must_use]
    pub const fn visible_count(&self) -> usize {
        self.visible_count
    }
    #[must_use]
    pub const fn total_count(&self) -> usize {
        self.total_count
    }

    #[must_use]
    pub const fn selected_rating(&self) -> Option<LighttableRating> {
        self.selected_rating
    }

    #[must_use]
    pub fn selected_labels(&self) -> impl ExactSizeIterator<Item = LighttableColorLabel> + '_ {
        self.selected_labels.iter().copied()
    }

    #[must_use]
    pub fn has_active_filter(&self) -> bool {
        !self.search_text.is_empty() || self.visible_count != self.total_count
    }

    #[must_use]
    pub fn selection_summary(&self) -> String {
        format!(
            "{} selected · {} of {}",
            self.selected_count, self.visible_count, self.total_count
        )
    }
}

#[derive(Clone)]
pub struct LighttableToolbar {
    root: gtk4::Box,
    property: gtk4::DropDown,
    search: gtk4::SearchEntry,
    sort: gtk4::DropDown,
    sort_direction: gtk4::Button,
    count: gtk4::Label,
    rating_buttons: Vec<(LighttableRating, gtk4::Button)>,
    label_buttons: Vec<(LighttableColorLabel, gtk4::Button)>,
    reset: gtk4::Button,
    projecting: Rc<Cell<bool>>,
    state: Rc<RefCell<LighttableToolbarState>>,
}

impl LighttableToolbar {
    #[allow(clippy::too_many_lines)]
    #[must_use]
    pub fn new() -> Self {
        let root = gtk4::Box::new(gtk4::Orientation::Horizontal, 3);
        root.set_widget_name(LIGHTTABLE_TOOLBAR.widget_name);
        root.set_hexpand(true);
        root.set_valign(gtk4::Align::Center);
        root.set_accessible_role(gtk4::AccessibleRole::Toolbar);
        apply_theme_role(&root, ThemeRole::Toolbar);

        let property = shared_dropdown(
            "lighttable-filter-property",
            &["film roll", "folders", "filename"],
        );
        property.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        property.set_tooltip_text(Some("collection filter property"));
        root.append(&property);

        let search = gtk4::SearchEntry::new();
        search.set_widget_name(LIGHTTABLE_TOOLBAR.filter_entry_name);
        search.set_placeholder_text(Some("search collection"));
        search.set_accessible_role(gtk4::AccessibleRole::SearchBox);
        search.set_hexpand(true);
        search.set_max_width_chars(18);
        root.append(&search);
        root.append(&toolbar_separator("filter-labels"));

        let mut label_buttons = Vec::new();
        for label in LighttableColorLabel::ALL {
            let button = shared_button(&format!("lighttable-color-{}", label.index()), "●");
            button.set_accessible_role(gtk4::AccessibleRole::Button);
            button.add_css_class("dt_filter_button");
            button.add_css_class("dt_color_filter");
            let tooltip = format!("toggle {} label on selected images", label.label());
            button.update_property(&[Property::Label(&tooltip)]);
            button.set_tooltip_text(Some(&tooltip));
            root.append(&button);
            label_buttons.push((label, button));
        }

        root.append(&toolbar_separator("labels-rating"));

        let mut rating_buttons = Vec::new();
        let rejected = shared_button("lighttable-rating-rejected", "×");
        rejected.add_css_class("dt_filter_button");
        rejected.add_css_class("dt_rating_filter");
        rejected.set_accessible_role(gtk4::AccessibleRole::Button);
        rejected.update_property(&[Property::Label("set rejected rating on selected images")]);
        rejected.set_tooltip_text(Some("reject selected images"));
        root.append(&rejected);
        rating_buttons.push((LighttableRating::Rejected, rejected));

        let unrated = shared_button("lighttable-rating-zero", "0");
        unrated.add_css_class("dt_filter_button");
        unrated.add_css_class("dt_rating_filter");
        unrated.set_accessible_role(gtk4::AccessibleRole::Button);
        unrated.update_property(&[Property::Label("set zero-star rating on selected images")]);
        unrated.set_tooltip_text(Some("set zero stars on selected images"));
        root.append(&unrated);
        rating_buttons.push((LighttableRating::Zero, unrated));

        for rating in LighttableRating::STARS {
            let button = shared_button(
                &format!("lighttable-rating-{}", rating.stars().unwrap_or(0)),
                "★",
            );
            button.add_css_class("dt_filter_button");
            button.add_css_class("dt_rating_filter");
            button.set_accessible_role(gtk4::AccessibleRole::Button);
            let tooltip = format!("set {} on selected images", rating.label());
            button.update_property(&[Property::Label(&tooltip)]);
            button.set_tooltip_text(Some(&tooltip));
            root.append(&button);
            rating_buttons.push((rating, button));
        }

        root.append(&toolbar_separator("rating-sort"));

        let sort = shared_dropdown("lighttable-sort", &["filename", "capture time", "rating"]);
        sort.set_accessible_role(gtk4::AccessibleRole::ComboBox);
        sort.set_tooltip_text(Some("sort visible images"));
        root.append(&sort);
        let sort_direction = shared_button("lighttable-sort-direction", "↑");
        sort_direction.set_accessible_role(gtk4::AccessibleRole::Button);
        sort_direction.set_tooltip_text(Some("sort ascending; click to reverse"));
        root.append(&sort_direction);
        let count = gtk4::Label::new(Some("0 selected · 0 of 0"));
        count.set_widget_name("lighttable-selection-count");
        count.add_css_class("dim-label");
        count.set_accessible_role(gtk4::AccessibleRole::Status);
        root.append(&count);
        root.append(&toolbar_separator("count-reset"));
        let reset = shared_button("lighttable-reset", "reset");
        reset.add_css_class("dt_filter_button");
        reset.update_property(&[Property::Label(
            "clear collection filter, sort, and selection",
        )]);
        reset.set_tooltip_text(Some("clear filter, sort, and selection"));
        reset.set_sensitive(false);
        root.append(&reset);

        Self {
            root,
            property,
            search,
            sort,
            sort_direction,
            count,
            rating_buttons,
            label_buttons,
            reset,
            projecting: Rc::new(Cell::new(false)),
            state: Rc::new(RefCell::new(LighttableToolbarState::new(0))),
        }
    }

    #[must_use]
    pub const fn widget(&self) -> &gtk4::Box {
        &self.root
    }

    #[must_use]
    pub fn state(&self) -> LighttableToolbarState {
        self.state.borrow().clone()
    }

    pub fn set_state(&self, state: &LighttableToolbarState) {
        self.projecting.set(true);
        self.state.replace(state.clone());
        self.property.set_selected(property_index(state.property));
        self.search.set_text(&state.search_text);
        self.sort.set_selected(state.sort.index());
        let (symbol, tooltip) = match state.sort_direction {
            LighttableSortDirection::Ascending => ("↑", "sort ascending; click to reverse"),
            LighttableSortDirection::Descending => ("↓", "sort descending; click to reverse"),
        };
        self.sort_direction.set_label(symbol);
        self.sort_direction.set_tooltip_text(Some(tooltip));
        self.count.set_text(&format!(
            "{} selected · {} of {}",
            state.selected_count, state.visible_count, state.total_count
        ));
        self.reset.set_sensitive(
            state.has_active_filter()
                || state.selected_count() > 0
                || state.sort() != LighttableSort::Filename
                || state.sort_direction() != LighttableSortDirection::Ascending,
        );
        for (rating, button) in &self.rating_buttons {
            button.set_css_classes(&button_classes(
                "dt_rating_filter",
                state.selected_rating == Some(*rating),
            ));
            button.update_state(&[State::Selected(Some(
                state.selected_rating == Some(*rating),
            ))]);
        }
        for (label, button) in &self.label_buttons {
            button.set_css_classes(&button_classes(
                "dt_color_filter",
                state.selected_labels.contains(label),
            ));
            button.update_state(&[State::Selected(Some(state.selected_labels.contains(label)))]);
        }
        self.projecting.set(false);
    }

    pub fn connect_action<F>(&self, callback: F)
    where
        F: Fn(LighttableToolbarAction) + 'static,
    {
        let callback = Rc::new(callback);
        let projecting = Rc::clone(&self.projecting);
        let action = Rc::clone(&callback);
        self.property.connect_selected_notify(move |dropdown| {
            if !projecting.get()
                && let Some(property) = CollectionProperty::from_index(dropdown.selected())
            {
                action(LighttableToolbarAction::SetProperty(property));
            }
        });
        let projecting = Rc::clone(&self.projecting);
        let action = Rc::clone(&callback);
        self.search.connect_search_changed(move |entry| {
            if !projecting.get() {
                action(LighttableToolbarAction::SetSearchText(
                    entry.text().to_string(),
                ));
            }
        });
        let projecting = Rc::clone(&self.projecting);
        let action = Rc::clone(&callback);
        self.sort.connect_selected_notify(move |dropdown| {
            if !projecting.get()
                && let Some(sort) = LighttableSort::from_index(dropdown.selected())
            {
                action(LighttableToolbarAction::SetSort(sort));
            }
        });
        let projecting = Rc::clone(&self.projecting);
        let action = Rc::clone(&callback);
        let state = Rc::clone(&self.state);
        self.sort_direction.connect_clicked(move |_| {
            if !projecting.get() {
                let direction = match state.borrow().sort_direction() {
                    LighttableSortDirection::Ascending => LighttableSortDirection::Descending,
                    LighttableSortDirection::Descending => LighttableSortDirection::Ascending,
                };
                action(LighttableToolbarAction::SetSortDirection(direction));
            }
        });
        for (rating, button) in &self.rating_buttons {
            let action = Rc::clone(&callback);
            let rating = *rating;
            button.connect_clicked(move |_| action(LighttableToolbarAction::SetRating(rating)));
        }
        for (label, button) in &self.label_buttons {
            let action = Rc::clone(&callback);
            let label = *label;
            button
                .connect_clicked(move |_| action(LighttableToolbarAction::ToggleColorLabel(label)));
        }
        self.reset
            .connect_clicked(move |_| callback(LighttableToolbarAction::ClearReset));
    }
}

impl Default for LighttableToolbar {
    fn default() -> Self {
        Self::new()
    }
}

fn button_classes(kind: &'static str, selected: bool) -> Vec<&'static str> {
    let mut classes = vec!["dt_filter_button", kind];
    if selected {
        classes.push("dt_selected");
    }
    classes
}

fn toolbar_separator(id: &str) -> gtk4::Separator {
    let separator = gtk4::Separator::new(gtk4::Orientation::Vertical);
    separator.set_widget_name(&format!("lighttable-toolbar-separator-{id}"));
    separator.add_css_class("dt_toolbar_separator");
    separator.set_margin_start(2);
    separator.set_margin_end(2);
    separator
}

fn property_index(property: CollectionProperty) -> u32 {
    CollectionProperty::ALL
        .iter()
        .position(|candidate| *candidate == property)
        .and_then(|index| u32::try_from(index).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toolbar_state_preserves_typed_filter_sort_and_selection() {
        let state = LighttableToolbarState::new(12)
            .with_filter(CollectionProperty::Folders, "trip", 4)
            .with_sort(LighttableSort::Rating)
            .with_sort_direction(LighttableSortDirection::Descending)
            .with_selection(
                2,
                Some(LighttableRating::Four),
                [LighttableColorLabel::Blue],
            );
        assert_eq!(state.property(), CollectionProperty::Folders);
        assert_eq!(state.search_text(), "trip");
        assert_eq!(state.sort(), LighttableSort::Rating);
        assert_eq!(state.sort_direction(), LighttableSortDirection::Descending);
        assert_eq!(state.selected_count(), 2);
        assert_eq!(state.visible_count(), 4);
        assert_eq!(state.total_count(), 12);
        assert_eq!(state.selected_rating(), Some(LighttableRating::Four));
        assert_eq!(
            state.selected_labels().collect::<Vec<_>>(),
            vec![LighttableColorLabel::Blue]
        );
        assert!(state.has_active_filter());
        assert_eq!(state.selection_summary(), "2 selected · 4 of 12");
    }

    #[test]
    fn gtk_toolbar_contract_keeps_darktable_roles_and_typed_controls() {
        let source = include_str!("toolbar.rs");
        for widget in [
            "lighttable-filter-property",
            "lighttable-filter-entry",
            "lighttable-sort",
            "lighttable-sort-direction",
            "lighttable-selection-count",
            "lighttable-reset",
        ] {
            assert!(source.contains(widget));
        }
        assert!(source.contains("AccessibleRole::Toolbar"));
        assert!(source.contains("AccessibleRole::SearchBox"));
        assert!(source.contains("AccessibleRole::Status"));
        assert!(source.contains("clear collection filter"));
        assert_eq!(LighttableColorLabel::ALL.len(), 5);
        assert_eq!(LighttableRating::STARS.len(), 5);
    }

    #[test]
    fn toolbar_state_clamps_counts_to_the_catalog_boundary() {
        let state = LighttableToolbarState::new(3)
            .with_filter(CollectionProperty::Filename, "", 9)
            .with_selection(8, None, []);
        assert_eq!(state.visible_count(), 3);
        assert_eq!(state.selected_count(), 3);
        assert!(!state.has_active_filter());
    }
}
