//! GTK-facing collection state backed by the imported catalog records.

use std::path::PathBuf;

use rusttable_catalog::ImportRecord;
use rusttable_core::PhotoId;
use rusttable_i18n::{CollationProfile, LocaleCollator, LocaleTag};
use rusttable_import::decode_reference_source;
use rusttable_ui::{CollectionItem, CollectionProperty, CollectionRule};

/// A display-safe snapshot for refreshing the lighttable and filmstrip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionSnapshot {
    property: CollectionProperty,
    search_text: String,
    total_count: usize,
    matching_photo_ids: Vec<PhotoId>,
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
}

/// Controller for one Darktable collection rule and its result set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionController {
    items: Vec<CollectionItem>,
    rule: CollectionRule,
    collation_profile: CollationProfile,
}

impl CollectionController {
    /// Creates a controller from display-ready searchable items.
    #[must_use]
    pub fn new(items: impl IntoIterator<Item = CollectionItem>) -> Self {
        Self {
            items: items.into_iter().collect(),
            rule: CollectionRule::new(CollectionProperty::Filename),
            collation_profile: CollationProfile::new(LocaleTag::default_locale()),
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
        Self::new(records.into_iter().map(collection_item))
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

    /// Produces the typed result projection consumed by GTK refresh code.
    #[must_use]
    pub fn snapshot(&self) -> CollectionSnapshot {
        let mut matching_items = self
            .items
            .iter()
            .filter(|item| self.rule.matches(item))
            .collect::<Vec<_>>();
        if let Ok(collator) = LocaleCollator::new(self.collation_profile.clone()) {
            matching_items.sort_by(|left, right| {
                collator
                    .compare(
                        left.value(self.rule.property()),
                        right.value(self.rule.property()),
                    )
                    .then_with(|| left.photo_id().cmp(&right.photo_id()))
            });
        }
        let matching_photo_ids = matching_items
            .into_iter()
            .map(CollectionItem::photo_id)
            .collect();
        CollectionSnapshot {
            property: self.rule.property(),
            search_text: self.rule.search_text().to_owned(),
            total_count: self.items.len(),
            matching_photo_ids,
        }
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
    use rusttable_ui::{CollectionItem, CollectionProperty};

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
