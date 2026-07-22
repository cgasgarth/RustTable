//! GTK-facing collection state backed by the imported catalog records.

mod ordering;
mod presentation;
mod service;
mod state;

pub use service::LibraryCollectionService;
pub use state::ActiveLighttableRestoreReport;

use rusttable_catalog::{
    ActiveLighttableState, CatalogCommand, CatalogState, ImportRecord, PhotoOrganizationState,
};
use rusttable_core::PhotoId;
use rusttable_i18n::{CollationProfile, LocaleCollator, LocaleTag};
use rusttable_ui::{
    CollectionItem, CollectionProperty, CollectionRule, LighttableColorLabel, LighttablePhotoState,
    LighttableRating, LighttableSort, LighttableSortDirection, LighttableToolbarState,
};
use std::collections::{BTreeMap, BTreeSet};

use ordering::{organization_matches, sort_items};
use presentation::{
    apply_fallback_organization, color_from_ui, photo_state, rating_from_ui, shared_labels,
    uniform_rating,
};
use state::{
    active_property, active_sort, active_sort_direction, collection_item, ui_property, ui_sort,
    ui_sort_direction,
};

/// A display-safe snapshot for refreshing the lighttable and filmstrip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSnapshot {
    property: CollectionProperty,
    search_text: String,
    total_count: usize,
    matching_photo_ids: Vec<PhotoId>,
    photo_states: Vec<LighttablePhotoState>,
    toolbar: LighttableToolbarState,
    generation: u64,
}

impl CollectionSnapshot {
    /// Returns the active property.
    #[must_use]
    pub const fn property(&self) -> CollectionProperty {
        self.property
    }

    /// Returns the search text used to produce this snapshot.
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }

    /// Returns the number of imported records before filtering.
    #[must_use]
    pub const fn total_count(&self) -> usize {
        self.total_count
    }

    /// Returns the number of records matching the current rule.
    #[must_use]
    pub const fn result_count(&self) -> usize {
        self.matching_photo_ids.len()
    }

    /// Returns matching photo IDs in deterministic locale-aware display order.
    #[must_use = "use the matching IDs to refresh the lighttable"]
    pub fn matching_photo_ids(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.matching_photo_ids.iter().copied()
    }

    #[must_use]
    pub fn photo_states(&self) -> impl ExactSizeIterator<Item = &LighttablePhotoState> {
        self.photo_states.iter()
    }

    #[must_use]
    pub const fn toolbar(&self) -> &LighttableToolbarState {
        &self.toolbar
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

/// Controller for one Darktable collection rule and its result set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionController {
    items: Vec<CollectionItem>,
    rule: CollectionRule,
    collation_profile: CollationProfile,
    organization: BTreeMap<PhotoId, PhotoOrganizationState>,
    catalog_state: Option<CatalogState>,
    selected: BTreeSet<PhotoId>,
    selection_anchor: Option<PhotoId>,
    sort: LighttableSort,
    sort_direction: LighttableSortDirection,
    generation: u64,
}

impl CollectionController {
    /// Creates a controller from display-ready searchable items.
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = CollectionItem>) -> Self {
        let items = items.into_iter().collect::<Vec<_>>();
        let organization = items
            .iter()
            .map(|item| {
                (
                    item.photo_id(),
                    PhotoOrganizationState::new(item.photo_id()),
                )
            })
            .collect();
        Self {
            items,
            rule: CollectionRule::new(CollectionProperty::Filename),
            collation_profile: CollationProfile::new(LocaleTag::default_locale()),
            organization,
            catalog_state: None,
            selected: BTreeSet::new(),
            selection_anchor: None,
            sort: LighttableSort::Filename,
            sort_direction: LighttableSortDirection::Ascending,
            generation: 0,
        }
    }

    /// Creates a controller with a locale-aware display ordering profile.
    #[must_use]
    pub fn with_locale(items: impl IntoIterator<Item = CollectionItem>, locale: LocaleTag) -> Self {
        let mut controller = Self::new(items);
        controller.collation_profile = CollationProfile::new(locale);
        controller
    }

    /// Projects imported records into a collection controller.
    #[must_use]
    pub fn from_import_records<'a>(records: impl IntoIterator<Item = &'a ImportRecord>) -> Self {
        Self::from_import_records_with_locale(records, LocaleTag::default_locale())
    }

    #[must_use]
    pub fn from_import_records_with_locale<'a>(
        records: impl IntoIterator<Item = &'a ImportRecord>,
        locale: LocaleTag,
    ) -> Self {
        let records = records.into_iter().collect::<Vec<_>>();
        Self::from_import_records_with_locale_and_organization(
            records.iter().copied(),
            locale,
            &BTreeMap::new(),
        )
    }

    #[must_use]
    pub fn from_import_records_with_locale_and_organization<'a>(
        records: impl IntoIterator<Item = &'a ImportRecord>,
        locale: LocaleTag,
        persisted: &BTreeMap<PhotoId, PhotoOrganizationState>,
    ) -> Self {
        let records = records.into_iter().collect::<Vec<_>>();
        let mut controller =
            Self::with_locale(records.iter().map(|record| collection_item(record)), locale);
        let mut catalog_state = CatalogState::new();
        for record in records {
            let expected = catalog_state.revision();
            if catalog_state
                .apply(
                    expected,
                    CatalogCommand::RegisterPhoto(record.photo().clone()),
                )
                .is_err()
            {
                return controller;
            }
        }
        controller.organization = catalog_state
            .photos()
            .map(|photo| {
                (
                    photo.id(),
                    persisted
                        .get(&photo.id())
                        .cloned()
                        .or_else(|| catalog_state.organization(photo.id()).cloned())
                        .unwrap_or_else(|| PhotoOrganizationState::new(photo.id())),
                )
            })
            .collect();
        controller.catalog_state = Some(catalog_state);
        controller
    }

    #[must_use]
    pub fn from_import_records_with_locale_and_organization_and_active_state<'a>(
        records: impl IntoIterator<Item = &'a ImportRecord>,
        locale: LocaleTag,
        persisted: &BTreeMap<PhotoId, PhotoOrganizationState>,
        active: &ActiveLighttableState,
    ) -> (Self, ActiveLighttableRestoreReport) {
        let mut controller =
            Self::from_import_records_with_locale_and_organization(records, locale, persisted);
        let report = controller.restore_active_state(active);
        (controller, report)
    }

    /// Returns the current rule.
    #[must_use]
    pub const fn rule(&self) -> &CollectionRule {
        &self.rule
    }

    /// Returns all searchable items in stable input order before display sorting.
    #[must_use = "refresh the collection controller from the catalog"]
    pub fn items(&self) -> impl ExactSizeIterator<Item = &CollectionItem> {
        self.items.iter()
    }

    /// Changes the active collection property.
    pub fn set_property(&mut self, property: CollectionProperty) {
        self.rule.set_property(property);
        self.reconcile_selection();
    }

    /// Changes the active search text.
    pub fn set_search_text(&mut self, search_text: impl Into<String>) {
        self.rule.set_search_text(search_text);
        self.reconcile_selection();
    }

    /// Clears the search text while preserving the selected property.
    pub fn clear(&mut self) {
        self.rule.set_search_text(String::new());
        self.reconcile_selection();
    }

    pub fn accept_generation(&mut self, generation: u64) -> bool {
        if generation < self.generation {
            return false;
        }
        self.generation = generation;
        true
    }

    pub fn set_sort(&mut self, sort: LighttableSort) {
        self.sort = sort;
    }

    pub fn set_sort_direction(&mut self, direction: LighttableSortDirection) {
        self.sort_direction = direction;
    }

    fn reconcile_selection(&mut self) {
        let visible = self
            .matching_photo_ids()
            .into_iter()
            .collect::<BTreeSet<_>>();
        self.selected.retain(|photo_id| visible.contains(photo_id));
        self.selection_anchor = self
            .selection_anchor
            .filter(|photo_id| self.selected.contains(photo_id))
            .or_else(|| self.selected.first().copied());
    }

    #[must_use]
    pub fn active_state(&self) -> ActiveLighttableState {
        ActiveLighttableState::new(
            active_property(self.rule.property()),
            self.rule.search_text(),
            active_sort(self.sort),
            active_sort_direction(self.sort_direction),
            self.selected.iter().map(|photo_id| photo_id.get()),
        )
    }

    pub fn restore_active_state(
        &mut self,
        active: &ActiveLighttableState,
    ) -> ActiveLighttableRestoreReport {
        self.rule.set_property(ui_property(active.property()));
        self.rule.set_search_text(active.search_text());
        self.sort = ui_sort(active.sort());
        self.sort_direction = ui_sort_direction(active.direction());

        let matching = self.matching_photo_ids();
        let matching = matching.iter().copied().collect::<BTreeSet<_>>();
        let available = self.organization.keys().copied().collect::<BTreeSet<_>>();
        let mut selected = BTreeSet::new();
        let mut discarded_missing = 0;
        let mut discarded_hidden = 0;
        for raw_id in active.selected_photo_ids() {
            let Some(photo_id) = PhotoId::new(*raw_id) else {
                discarded_missing += 1;
                continue;
            };
            if !available.contains(&photo_id) {
                discarded_missing += 1;
            } else if !matching.contains(&photo_id) {
                discarded_hidden += 1;
            } else {
                selected.insert(photo_id);
            }
        }
        self.selected = selected;
        self.selection_anchor = self
            .matching_photo_ids()
            .into_iter()
            .find(|photo_id| self.selected.contains(photo_id));
        ActiveLighttableRestoreReport {
            discarded_missing,
            discarded_hidden,
        }
    }

    pub fn select_only(&mut self, photo_id: PhotoId) -> bool {
        if !self.organization.contains_key(&photo_id) {
            return false;
        }
        let changed = self.selected.len() != 1 || !self.selected.contains(&photo_id);
        self.selected.clear();
        self.selected.insert(photo_id);
        self.selection_anchor = Some(photo_id);
        changed
    }

    pub fn toggle_selection(&mut self, photo_id: PhotoId) -> bool {
        if !self.organization.contains_key(&photo_id) {
            return false;
        }
        if !self.selected.insert(photo_id) {
            self.selected.remove(&photo_id);
        }
        self.selection_anchor = Some(photo_id);
        true
    }

    /// Selects the deterministic visible range from the last single selection.
    pub fn select_range(&mut self, photo_id: PhotoId, extend: bool) -> bool {
        if !self.organization.contains_key(&photo_id) {
            return false;
        }
        let anchor = self.selection_anchor.unwrap_or(photo_id);
        let order = self.snapshot().matching_photo_ids().collect::<Vec<_>>();
        let Some(left) = order.iter().position(|id| *id == anchor) else {
            return self.select_only(photo_id);
        };
        let Some(right) = order.iter().position(|id| *id == photo_id) else {
            return false;
        };
        let (start, end) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let before = self.selected.clone();
        if !extend {
            self.selected.clear();
        }
        self.selected.extend(order[start..=end].iter().copied());
        before != self.selected
    }

    #[must_use]
    pub fn selected_photo_ids(&self) -> impl ExactSizeIterator<Item = PhotoId> + '_ {
        self.selected.iter().copied()
    }

    #[must_use]
    pub fn organization_command_for_rating(
        &self,
        rating: LighttableRating,
    ) -> Option<CatalogCommand> {
        let photo_ids = self.selected_photo_ids().collect::<Vec<_>>();
        if photo_ids.is_empty() {
            return None;
        }
        match rating {
            LighttableRating::Rejected => Some(CatalogCommand::SetRejection {
                rejected: !photo_ids.iter().all(|photo_id| {
                    self.organization
                        .get(photo_id)
                        .is_some_and(|state| state.rejected)
                }),
                photo_ids,
            }),
            value => Some(CatalogCommand::SetRating {
                photo_ids,
                rating: rating_from_ui(value),
            }),
        }
    }

    #[must_use]
    pub fn organization_command_for_color_label(
        &self,
        label: LighttableColorLabel,
    ) -> Option<CatalogCommand> {
        let photo_ids = self.selected_photo_ids().collect::<Vec<_>>();
        (!photo_ids.is_empty()).then_some(CatalogCommand::ToggleColorLabel {
            photo_ids,
            label: color_from_ui(label),
        })
    }

    pub fn replace_organization(
        &mut self,
        states: impl IntoIterator<Item = PhotoOrganizationState>,
    ) {
        let persisted = states
            .into_iter()
            .map(|state| (state.photo_id, state))
            .collect::<BTreeMap<_, _>>();
        self.organization = self
            .items
            .iter()
            .map(|item| {
                let photo_id = item.photo_id();
                (
                    photo_id,
                    persisted
                        .get(&photo_id)
                        .cloned()
                        .unwrap_or_else(|| PhotoOrganizationState::new(photo_id)),
                )
            })
            .collect();
        self.catalog_state = None;
    }

    pub fn set_selected_rating(&mut self, rating: LighttableRating) {
        if let Some(command) = self.organization_command_for_rating(rating) {
            self.apply_catalog_command(command);
        }
    }

    pub fn toggle_selected_color_label(&mut self, label: LighttableColorLabel) {
        if let Some(command) = self.organization_command_for_color_label(label) {
            self.apply_catalog_command(command);
        }
    }

    pub fn clear_reset(&mut self) {
        self.rule = CollectionRule::new(CollectionProperty::Filename);
        self.sort = LighttableSort::Filename;
        self.sort_direction = LighttableSortDirection::Ascending;
        self.selected.clear();
        self.selection_anchor = None;
    }

    fn apply_catalog_command(&mut self, command: CatalogCommand) {
        if let Some(catalog) = self.catalog_state.as_mut() {
            let expected = catalog.revision();
            if catalog.apply(expected, command).is_ok() {
                self.organization = catalog
                    .photos()
                    .filter_map(|photo| {
                        catalog
                            .organization(photo.id())
                            .cloned()
                            .map(|state| (photo.id(), state))
                    })
                    .collect();
            }
            return;
        }
        apply_fallback_organization(&mut self.organization, command);
    }

    fn matching_photo_ids(&self) -> Vec<PhotoId> {
        self.snapshot().matching_photo_ids().collect()
    }

    /// Produces the typed result projection consumed by GTK refresh code.
    #[must_use]
    pub fn snapshot(&self) -> CollectionSnapshot {
        let mut matching_items = self
            .items
            .iter()
            .filter(|item| {
                self.rule.matches(item)
                    && organization_matches(&self.rule, item.photo_id(), &self.organization)
            })
            .collect::<Vec<_>>();
        let collator = LocaleCollator::new(self.collation_profile.clone()).ok();
        sort_items(
            &mut matching_items,
            self.sort,
            self.sort_direction,
            &self.organization,
            collator.as_ref(),
        );
        let matching_photo_ids: Vec<PhotoId> = matching_items
            .into_iter()
            .map(CollectionItem::photo_id)
            .collect();
        let photo_states = matching_photo_ids
            .iter()
            .copied()
            .map(|photo_id| photo_state(photo_id, &self.organization, &self.selected))
            .collect::<Vec<_>>();
        let selected_organization = self
            .selected
            .iter()
            .filter_map(|photo_id| self.organization.get(photo_id))
            .collect::<Vec<_>>();
        let selected_rating = uniform_rating(&selected_organization);
        let selected_labels = shared_labels(&selected_organization);
        let toolbar = LighttableToolbarState::new(self.items.len())
            .with_filter(
                self.rule.property(),
                self.rule.search_text(),
                matching_photo_ids.len(),
            )
            .with_sort(self.sort)
            .with_sort_direction(self.sort_direction)
            .with_selection(self.selected.len(), selected_rating, selected_labels);
        CollectionSnapshot {
            property: self.rule.property(),
            search_text: self.rule.search_text().to_owned(),
            total_count: self.items.len(),
            matching_photo_ids,
            photo_states,
            toolbar,
            generation: self.generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use rusttable_catalog::{
        ActiveLighttableProperty, ActiveLighttableSort, ActiveLighttableSortDirection,
        ActiveLighttableState,
    };
    use std::path::Path;

    use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, PhotoId};
    use rusttable_i18n::LocaleTag;
    use rusttable_ui::{
        CollectionItem, CollectionProperty, LighttableColorLabel, LighttableRating, LighttableSort,
        LighttableSortDirection,
    };

    use super::{ActiveLighttableRestoreReport, CollectionController};

    fn id(value: u128) -> PhotoId {
        PhotoId::new(value).expect("non-zero photo ID")
    }

    fn controller() -> CollectionController {
        CollectionController::new([
            CollectionItem::new(id(1), "/photos/2026/holiday/IMG_0001.CR3"),
            CollectionItem::new(id(2), "/photos/2026/portraits/portrait.jpg"),
            CollectionItem::new(id(3), "/photos/2025/archive/old.png"),
        ])
    }

    #[test]
    fn empty_snapshot_contains_every_imported_record() {
        let controller = controller();
        let snapshot = controller.snapshot();

        assert_eq!(snapshot.total_count(), 3);
        assert_eq!(snapshot.result_count(), 3);
        assert_eq!(
            snapshot.matching_photo_ids().collect::<Vec<_>>(),
            vec![id(1), id(3), id(2)]
        );
    }

    #[test]
    fn locale_profile_controls_collection_display_order() {
        let controller = CollectionController::with_locale(
            [
                CollectionItem::new(id(1), "/photos/IMG10.CR3"),
                CollectionItem::new(id(2), "/photos/IMG2.CR3"),
            ],
            LocaleTag::parse("en-US").expect("valid locale"),
        );

        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            vec![id(2), id(1)]
        );
    }

    fn item_with_capture(photo_id: u128, path: &str, capture_time: Option<&str>) -> CollectionItem {
        let item = CollectionItem::new(id(photo_id), path);
        let Some(capture_time) = capture_time else {
            return item;
        };
        let metadata = ImageMetadata::from_entries([MetadataEntry::CaptureDateTimeOriginal(
            MetadataText::new(capture_time).expect("capture time"),
        )])
        .expect("metadata");
        item.with_capture_metadata(&metadata)
    }

    #[test]
    fn filename_sort_uses_canonical_filename_and_path_keys_not_filter_property() {
        let mut controller = CollectionController::with_locale(
            [
                CollectionItem::new(id(90), "/photos/z/IMG10.CR3"),
                CollectionItem::new(id(2), "/photos/a/IMG2.CR3"),
                CollectionItem::new(id(77), "/photos/c/IMG2.CR3"),
                CollectionItem::new(id(1), "/photos/d/IMG1.CR3"),
            ],
            LocaleTag::parse("en-US").expect("valid locale"),
        );
        let expected = vec![id(1), id(2), id(77), id(90)];

        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            expected
        );
        controller.set_property(CollectionProperty::Folders);
        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            expected
        );
        controller.set_property(CollectionProperty::Rating);
        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn capture_sort_normalizes_timezones_and_orders_missing_values_deterministically() {
        let items = [
            item_with_capture(90, "/photos/z/late.jpg", Some("2025:01:01 00:00:00")),
            item_with_capture(
                5,
                "/photos/b/equal-b.jpg",
                Some("2024:01:02 05:04:05+02:00"),
            ),
            item_with_capture(42, "/photos/a/equal-a.jpg", Some("2024-01-02T03:04:05Z")),
            item_with_capture(1, "/photos/missing.jpg", None),
            item_with_capture(77, "/photos/invalid.jpg", Some("not-a-date")),
        ];
        let mut controller = CollectionController::with_locale(
            items,
            LocaleTag::parse("en-US").expect("valid locale"),
        );
        controller.set_sort(LighttableSort::CaptureTime);

        let ascending = controller
            .snapshot()
            .matching_photo_ids()
            .collect::<Vec<_>>();
        assert_eq!(ascending, vec![id(77), id(1), id(42), id(5), id(90)]);

        controller.set_sort_direction(LighttableSortDirection::Descending);
        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            vec![id(90), id(42), id(5), id(77), id(1)]
        );

        let state = controller.active_state();
        let mut rebuilt = CollectionController::new(controller.items().cloned());
        rebuilt.restore_active_state(&state);
        let restored = rebuilt.snapshot().matching_photo_ids().collect::<Vec<_>>();
        assert_eq!(restored, vec![id(90), id(42), id(5), id(77), id(1)]);
        assert_eq!(
            restored
                .chunks(2)
                .flat_map(|page| page.iter().copied())
                .collect::<Vec<_>>(),
            restored
        );
    }

    #[test]
    fn folder_rule_returns_matching_photo_ids_and_counts() {
        let mut controller = controller();
        controller.set_property(CollectionProperty::Folders);
        controller.set_search_text("/photos/2026");
        let snapshot = controller.snapshot();

        assert_eq!(snapshot.total_count(), 3);
        assert_eq!(snapshot.result_count(), 2);
        assert_eq!(
            snapshot.matching_photo_ids().collect::<Vec<_>>(),
            vec![id(1), id(2)]
        );
    }

    #[test]
    fn clear_preserves_property_and_restores_all_results() {
        let mut controller = controller();
        controller.set_property(CollectionProperty::Filename);
        controller.set_search_text("portrait");
        assert_eq!(controller.snapshot().result_count(), 1);

        controller.clear();
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.property(), CollectionProperty::Filename);
        assert_eq!(snapshot.search_text(), "");
        assert_eq!(snapshot.result_count(), 3);
    }

    #[test]
    fn selection_rating_and_color_label_transitions_project_to_toolbar_and_cards() {
        let mut controller = controller();
        assert!(controller.select_only(id(2)));
        controller.set_selected_rating(LighttableRating::Four);
        controller.toggle_selected_color_label(LighttableColorLabel::Blue);

        let snapshot = controller.snapshot();
        assert_eq!(snapshot.toolbar().selected_count(), 1);
        let selected = snapshot
            .photo_states()
            .find(|state| state.photo_id() == id(2))
            .expect("selected photo state");
        assert!(selected.selected());
        assert_eq!(selected.rating(), LighttableRating::Four);
        assert_eq!(
            selected.color_labels().collect::<Vec<_>>(),
            vec![LighttableColorLabel::Blue]
        );
    }

    #[test]
    fn selection_range_and_toggle_follow_the_same_visible_order_as_the_grid() {
        let mut controller = controller();
        assert!(controller.select_only(id(3)));
        assert!(controller.select_range(id(2), false));
        assert_eq!(
            controller
                .snapshot()
                .photo_states()
                .filter(|state| state.selected())
                .map(rusttable_ui::LighttablePhotoState::photo_id)
                .collect::<Vec<_>>(),
            vec![id(3), id(2)]
        );

        assert!(controller.toggle_selection(id(3)));
        assert_eq!(
            controller
                .snapshot()
                .photo_states()
                .filter(|state| state.selected())
                .map(rusttable_ui::LighttablePhotoState::photo_id)
                .collect::<Vec<_>>(),
            vec![id(2)]
        );
    }

    #[test]
    fn rating_sort_is_deterministic_and_reset_restores_filename_order() {
        let mut controller = controller();
        controller.select_only(id(3));
        controller.set_selected_rating(LighttableRating::Five);
        controller.set_sort(LighttableSort::Rating);
        assert_eq!(
            controller
                .snapshot()
                .matching_photo_ids()
                .collect::<Vec<_>>(),
            vec![id(3), id(1), id(2)]
        );

        controller.clear_reset();
        let snapshot = controller.snapshot();
        assert_eq!(snapshot.toolbar().sort(), LighttableSort::Filename);
        assert_eq!(snapshot.toolbar().selected_count(), 0);
        assert_eq!(
            snapshot.matching_photo_ids().collect::<Vec<_>>(),
            vec![id(1), id(3), id(2)]
        );
    }

    #[test]
    fn active_state_rebuild_restores_rule_sort_direction_and_visible_selection() {
        let mut controller = controller();
        controller.set_property(CollectionProperty::Folders);
        controller.set_search_text("/photos/2026");
        controller.set_sort(LighttableSort::Rating);
        controller.set_sort_direction(LighttableSortDirection::Descending);
        controller.select_only(id(1));
        controller.toggle_selection(id(2));
        let state = controller.active_state();

        let mut rebuilt = CollectionController::new(controller.items().cloned());
        let report = rebuilt.restore_active_state(&state);

        assert_eq!(report, ActiveLighttableRestoreReport::default());
        assert_eq!(rebuilt.active_state(), state);
        assert_eq!(rebuilt.rule().property(), CollectionProperty::Folders);
        assert_eq!(rebuilt.rule().search_text(), "/photos/2026");
        assert_eq!(rebuilt.snapshot().toolbar().sort(), LighttableSort::Rating);
        assert_eq!(
            rebuilt.snapshot().toolbar().sort_direction(),
            LighttableSortDirection::Descending
        );
        assert_eq!(
            rebuilt.selected_photo_ids().collect::<Vec<_>>(),
            vec![id(1), id(2)]
        );
    }

    #[test]
    fn restore_discards_missing_and_hidden_selection_deterministically() {
        let mut controller = controller();
        let state = ActiveLighttableState::new(
            ActiveLighttableProperty::Folders,
            "/photos/2026",
            ActiveLighttableSort::Filename,
            ActiveLighttableSortDirection::Ascending,
            [3, 2, 99],
        );

        let report = controller.restore_active_state(&state);

        assert_eq!(report.discarded_missing, 1);
        assert_eq!(report.discarded_hidden, 1);
        assert_eq!(
            controller.selected_photo_ids().collect::<Vec<_>>(),
            vec![id(2)]
        );
        assert_eq!(controller.active_state().selected_photo_ids(), &[2]);
    }

    #[test]
    fn filtering_reconciles_selection_and_does_not_restore_it_on_reentry() {
        let mut controller = controller();
        controller.select_only(id(1));
        controller.set_search_text("portrait");
        assert_eq!(controller.selected_photo_ids().count(), 0);

        controller.clear();
        assert_eq!(controller.selected_photo_ids().count(), 0);
    }

    #[test]
    fn reference_sources_are_projected_to_their_physical_filename() {
        use rusttable_catalog::{ImportCandidate, ImportRecord, SourcePath};
        use rusttable_core::{
            Asset, AssetId, AssetRole, ByteLength, ContentHash, ImageMetadata, Photo,
        };
        use rusttable_image::{ImageDimensions, ImageProbe, InputFormat};
        use rusttable_import::encode_reference_source;

        let path = Path::new("/photos/holiday/IMG_0007.CR3");
        let source = encode_reference_source(path, [7; 32]).expect("reference source");
        let candidate = ImportCandidate::new(
            id(7),
            AssetId::new(7).expect("asset ID"),
            source,
            ContentHash::Sha256([7; 32]),
            ByteLength::from_bytes(1),
            ImageProbe::new(
                InputFormat::Png,
                ImageDimensions::new(2, 2).expect("dimensions"),
            ),
            ImageMetadata::empty(),
        )
        .expect("candidate");
        let photo = Photo::new(
            id(7),
            [Asset::new(
                AssetId::new(7).expect("asset ID"),
                AssetRole::Primary,
                ContentHash::Sha256([7; 32]),
                ByteLength::from_bytes(1),
            )],
        )
        .expect("photo");
        let record = ImportRecord::new(&candidate, photo).expect("record");
        let controller = CollectionController::from_import_records([&record]);

        let mut controller = controller;
        controller.set_property(CollectionProperty::Filename);
        controller.set_search_text("IMG_0007");
        assert_eq!(controller.snapshot().result_count(), 1);

        let _ = SourcePath::new("logical/path.jpg").expect("source path API remains available");
    }
}
