//! GTK-facing collection state backed by the imported catalog records.

mod service;

pub use service::LibraryCollectionService;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use rusttable_catalog::{
    CatalogCommand, CatalogState, ColorLabel, ImportRecord, PhotoOrganizationState, Rating,
};
use rusttable_core::PhotoId;
use rusttable_i18n::{CollationProfile, LocaleCollator, LocaleTag};
use rusttable_import::decode_reference_source;
use rusttable_ui::{
    CollectionItem, CollectionProperty, CollectionRule, LighttableColorLabel, LighttablePhotoState,
    LighttableRating, LighttableSort, LighttableToolbarState,
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
            .filter_map(|photo| {
                catalog_state
                    .organization(photo.id())
                    .cloned()
                    .map(|state| (photo.id(), state))
            })
            .collect();
        controller.catalog_state = Some(catalog_state);
        controller
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
    }

    /// Changes the active search text.
    pub fn set_search_text(&mut self, search_text: impl Into<String>) {
        self.rule.set_search_text(search_text);
    }

    /// Clears the search text while preserving the selected property.
    pub fn clear(&mut self) {
        self.rule.set_search_text(String::new());
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

    pub fn set_selected_rating(&mut self, rating: LighttableRating) {
        let photo_ids = self.selected.iter().copied().collect::<Vec<_>>();
        if photo_ids.is_empty() {
            return;
        }
        match rating {
            LighttableRating::Rejected => {
                self.apply_catalog_command(CatalogCommand::SetRejection {
                    photo_ids,
                    rejected: true,
                });
            }
            value => {
                self.apply_catalog_command(CatalogCommand::SetRejection {
                    photo_ids: photo_ids.clone(),
                    rejected: false,
                });
                self.apply_catalog_command(CatalogCommand::SetRating {
                    photo_ids,
                    rating: rating_from_ui(value),
                });
            }
        }
    }

    pub fn toggle_selected_color_label(&mut self, label: LighttableColorLabel) {
        let photo_ids = self.selected.iter().copied().collect::<Vec<_>>();
        if photo_ids.is_empty() {
            return;
        }
        self.apply_catalog_command(CatalogCommand::ToggleColorLabel {
            photo_ids,
            label: color_from_ui(label),
        });
    }

    pub fn clear_reset(&mut self) {
        self.rule = CollectionRule::new(CollectionProperty::Filename);
        self.sort = LighttableSort::Filename;
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
        if let Ok(collator) = LocaleCollator::new(self.collation_profile.clone()) {
            matching_items.sort_by(|left, right| {
                match self.sort {
                    LighttableSort::Filename => collator.compare(
                        left.value(self.rule.property()),
                        right.value(self.rule.property()),
                    ),
                    LighttableSort::CaptureTime => left.photo_id().cmp(&right.photo_id()),
                    LighttableSort::Rating => {
                        organization_rating(&self.organization, right.photo_id())
                            .cmp(&organization_rating(&self.organization, left.photo_id()))
                    }
                }
                .then_with(|| left.photo_id().cmp(&right.photo_id()))
            });
        }
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

fn rating_from_ui(rating: LighttableRating) -> Rating {
    Rating::from_u8(rating.stars().unwrap_or(0)).unwrap_or(Rating::Zero)
}

const fn color_from_ui(label: LighttableColorLabel) -> ColorLabel {
    match label {
        LighttableColorLabel::Red => ColorLabel::Red,
        LighttableColorLabel::Yellow => ColorLabel::Yellow,
        LighttableColorLabel::Green => ColorLabel::Green,
        LighttableColorLabel::Blue => ColorLabel::Blue,
        LighttableColorLabel::Purple => ColorLabel::Purple,
    }
}

const fn color_to_ui(label: ColorLabel) -> LighttableColorLabel {
    match label {
        ColorLabel::Red => LighttableColorLabel::Red,
        ColorLabel::Yellow => LighttableColorLabel::Yellow,
        ColorLabel::Green => LighttableColorLabel::Green,
        ColorLabel::Blue => LighttableColorLabel::Blue,
        ColorLabel::Purple => LighttableColorLabel::Purple,
    }
}

fn organization_rating(
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
    photo_id: PhotoId,
) -> u8 {
    organization
        .get(&photo_id)
        .map_or(0, |state| state.rating.as_u8())
}

fn organization_matches(
    rule: &CollectionRule,
    photo_id: PhotoId,
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
) -> bool {
    if rule.search_text().trim().is_empty() {
        return true;
    }
    let Some(state) = organization.get(&photo_id) else {
        return true;
    };
    match rule.property() {
        CollectionProperty::Rating => {
            let rating = if state.rejected {
                "rejected".to_owned()
            } else {
                state.rating.as_u8().to_string()
            };
            rule.matches_value(&rating)
        }
        CollectionProperty::ColorLabel => state.color_labels.iter().any(|label| {
            rule.matches_value(match label {
                ColorLabel::Red => "red",
                ColorLabel::Yellow => "yellow",
                ColorLabel::Green => "green",
                ColorLabel::Blue => "blue",
                ColorLabel::Purple => "purple",
            })
        }),
        CollectionProperty::Filmroll
        | CollectionProperty::Folders
        | CollectionProperty::Filename => true,
    }
}

fn photo_state(
    photo_id: PhotoId,
    organization: &BTreeMap<PhotoId, PhotoOrganizationState>,
    selected: &BTreeSet<PhotoId>,
) -> LighttablePhotoState {
    let state = organization.get(&photo_id);
    let rating = state.map_or(LighttableRating::Zero, |state| {
        if state.rejected {
            LighttableRating::Rejected
        } else {
            ui_rating(state.rating)
        }
    });
    let labels = state
        .into_iter()
        .flat_map(|state| state.color_labels.iter().copied().map(color_to_ui));
    LighttablePhotoState::new(photo_id, selected.contains(&photo_id), rating, labels)
}

fn ui_rating(rating: Rating) -> LighttableRating {
    match rating {
        Rating::Zero => LighttableRating::Zero,
        Rating::One => LighttableRating::One,
        Rating::Two => LighttableRating::Two,
        Rating::Three => LighttableRating::Three,
        Rating::Four => LighttableRating::Four,
        Rating::Five => LighttableRating::Five,
    }
}

fn uniform_rating(states: &[&PhotoOrganizationState]) -> Option<LighttableRating> {
    let first = states.first()?;
    let rating = if first.rejected {
        LighttableRating::Rejected
    } else {
        ui_rating(first.rating)
    };
    states
        .iter()
        .all(|state| {
            if state.rejected {
                rating == LighttableRating::Rejected
            } else {
                rating == ui_rating(state.rating)
            }
        })
        .then_some(rating)
}

fn shared_labels(states: &[&PhotoOrganizationState]) -> BTreeSet<LighttableColorLabel> {
    let Some(first) = states.first() else {
        return BTreeSet::new();
    };
    first
        .color_labels
        .iter()
        .copied()
        .filter(|label| {
            states
                .iter()
                .all(|state| state.color_labels.contains(label))
        })
        .map(color_to_ui)
        .collect()
}

fn apply_fallback_organization(
    organization: &mut BTreeMap<PhotoId, PhotoOrganizationState>,
    command: CatalogCommand,
) {
    match command {
        CatalogCommand::SetRating { photo_ids, rating } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    state.rating = rating;
                }
            }
        }
        CatalogCommand::SetRejection {
            photo_ids,
            rejected,
        } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    state.rejected = rejected;
                }
            }
        }
        CatalogCommand::SetColorLabel {
            photo_ids,
            label,
            enabled,
        } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id) {
                    if enabled {
                        state.color_labels.insert(label);
                    } else {
                        state.color_labels.remove(&label);
                    }
                }
            }
        }
        CatalogCommand::ToggleColorLabel { photo_ids, label } => {
            for photo_id in photo_ids {
                if let Some(state) = organization.get_mut(&photo_id)
                    && !state.color_labels.insert(label)
                {
                    state.color_labels.remove(&label);
                }
            }
        }
        CatalogCommand::RegisterPhoto(_)
        | CatalogCommand::CreateEdit(_)
        | CatalogCommand::ReplaceEdit { .. } => {}
    }
}

pub(super) fn collection_item(record: &ImportRecord) -> CollectionItem {
    let path = decode_reference_source(record.source())
        .map_or_else(|_| record.source().as_str().to_owned(), path_string);
    CollectionItem::new(record.photo().id(), path)
}

fn path_string(path: PathBuf) -> String {
    path.into_os_string().to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use rusttable_core::PhotoId;
    use rusttable_i18n::LocaleTag;
    use rusttable_ui::{
        CollectionItem, CollectionProperty, LighttableColorLabel, LighttableRating, LighttableSort,
    };

    use super::CollectionController;

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
