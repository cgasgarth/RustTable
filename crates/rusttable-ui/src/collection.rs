//! Pure collection-rule models shared by the catalog controller and GTK controls.

use std::path::Path;

use rusttable_core::{ImageMetadata, MetadataEntry, MetadataField, PhotoId};
use rusttable_i18n::{I18n, MessageArgs, MessageId, normalize_search};

/// The first Darktable collection properties supported by `RustTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollectionProperty {
    /// The containing film-roll directory.
    Filmroll,
    /// The containing folder path.
    Folders,
    /// The current star rating.
    Rating,
    /// The current color label.
    ColorLabel,
    /// The source filename.
    #[default]
    Filename,
}

impl CollectionProperty {
    /// All supported properties in the same order used by the GTK dropdown.
    pub const ALL: [Self; 5] = [
        Self::Filmroll,
        Self::Folders,
        Self::Rating,
        Self::ColorLabel,
        Self::Filename,
    ];

    /// Returns the stable message ID used by the collection control.
    #[must_use]
    pub const fn message_id(self) -> MessageId {
        match self {
            Self::Filmroll => MessageId::CollectionPropertyFilmroll,
            Self::Folders => MessageId::CollectionPropertyFolders,
            Self::Rating => MessageId::CollectionPropertyRating,
            Self::ColorLabel => MessageId::CollectionPropertyColorLabel,
            Self::Filename => MessageId::CollectionPropertyFilename,
        }
    }

    /// Returns the localized collection-property label.
    #[must_use]
    pub fn localized_label(self, i18n: &I18n) -> String {
        i18n.text(self.message_id(), &MessageArgs::new())
    }

    /// Returns the dropdown index for this property.
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Filmroll => 0,
            Self::Folders => 1,
            Self::Rating => 2,
            Self::ColorLabel => 3,
            Self::Filename => 4,
        }
    }

    /// Converts a GTK dropdown index into a supported property.
    #[must_use]
    pub const fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(Self::Filmroll),
            1 => Some(Self::Folders),
            2 => Some(Self::Rating),
            3 => Some(Self::ColorLabel),
            4 => Some(Self::Filename),
            _ => None,
        }
    }
}

/// The searchable path projection for one imported photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionItem {
    photo_id: PhotoId,
    path: String,
    canonical_path: String,
    filmroll: String,
    folders: String,
    filename: String,
    canonical_filename: String,
    capture_time_sort_key: Option<i128>,
}

impl CollectionItem {
    /// Creates a searchable item from the physical or logical source path.
    #[must_use]
    pub fn new(photo_id: PhotoId, path: impl Into<String>) -> Self {
        let path = path.into();
        let path_ref = Path::new(&path);
        let folders = path_ref
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map_or_else(String::new, |parent| parent.to_string_lossy().into_owned());
        let filename = path_ref
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let canonical_path = normalize_search(&path);
        let canonical_filename = normalize_search(&filename);

        Self {
            photo_id,
            path,
            canonical_path,
            filmroll: folders.clone(),
            folders,
            canonical_filename,
            filename,
            capture_time_sort_key: None,
        }
    }

    /// Adds the normalized capture timestamp used by the lighttable sort.
    #[must_use]
    pub fn with_capture_metadata(mut self, metadata: &ImageMetadata) -> Self {
        self.capture_time_sort_key = metadata
            .get(MetadataField::CaptureDateTimeOriginal)
            .and_then(|entry| match entry {
                MetadataEntry::CaptureDateTimeOriginal(value) => parse_capture_time(value.as_str()),
                _ => None,
            });
        self
    }

    /// Returns the stable photo identity.
    #[must_use]
    pub const fn photo_id(&self) -> PhotoId {
        self.photo_id
    }

    /// Returns the original path used to derive the collection fields.
    #[must_use]
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Returns the canonical path key used for deterministic sorting.
    #[must_use]
    pub fn path_sort_key(&self) -> &str {
        &self.canonical_path
    }

    /// Returns the film-roll value.
    #[must_use]
    pub fn filmroll(&self) -> &str {
        &self.filmroll
    }

    /// Returns the folder value.
    #[must_use]
    pub fn folders(&self) -> &str {
        &self.folders
    }

    /// Returns the filename value.
    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    /// Returns the canonical filename key used for deterministic sorting.
    #[must_use]
    pub fn filename_sort_key(&self) -> &str {
        &self.canonical_filename
    }

    /// Returns the normalized capture timestamp, if metadata supplied a valid one.
    #[must_use]
    pub const fn capture_time_sort_key(&self) -> Option<i128> {
        self.capture_time_sort_key
    }

    /// Returns the searchable value for one collection property.
    #[must_use]
    pub fn value(&self, property: CollectionProperty) -> &str {
        match property {
            CollectionProperty::Filmroll => self.filmroll(),
            CollectionProperty::Folders => self.folders(),
            CollectionProperty::Rating | CollectionProperty::ColorLabel => "",
            CollectionProperty::Filename => self.filename(),
        }
    }
}

/// One Darktable-style collection rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRule {
    property: CollectionProperty,
    search_text: String,
}

impl CollectionRule {
    /// Creates an empty rule for the requested property.
    #[must_use]
    pub fn new(property: CollectionProperty) -> Self {
        Self {
            property,
            search_text: String::new(),
        }
    }

    /// Returns the active property.
    #[must_use]
    pub const fn property(&self) -> CollectionProperty {
        self.property
    }

    /// Returns the raw search text entered by the user.
    #[must_use]
    pub fn search_text(&self) -> &str {
        &self.search_text
    }

    /// Sets the active property.
    pub fn set_property(&mut self, property: CollectionProperty) {
        self.property = property;
    }

    /// Replaces the search text without applying UI-specific normalization.
    pub fn set_search_text(&mut self, search_text: impl Into<String>) {
        self.search_text = search_text.into();
    }

    /// Returns whether an item matches this rule.
    #[must_use]
    pub fn matches(&self, item: &CollectionItem) -> bool {
        if matches!(
            self.property,
            CollectionProperty::Rating | CollectionProperty::ColorLabel
        ) {
            return true;
        }
        self.matches_value(item.value(self.property))
    }

    /// Returns whether a display value matches this rule.
    #[must_use]
    pub fn matches_value(&self, haystack: &str) -> bool {
        let needle = self.search_text.trim();
        if needle.is_empty() {
            return true;
        }

        match self.property {
            CollectionProperty::Filename | CollectionProperty::Rating => {
                needle.split(',').any(|term| {
                    contains_case_insensitive(haystack, strip_prefix_wildcard(term.trim()))
                })
            }
            CollectionProperty::Filmroll
            | CollectionProperty::Folders
            | CollectionProperty::ColorLabel => {
                let needle = needle
                    .strip_suffix('*')
                    .unwrap_or(needle)
                    .trim_end_matches('/');
                contains_case_insensitive(haystack, strip_prefix_wildcard(needle))
            }
        }
    }
}

fn strip_prefix_wildcard(value: &str) -> &str {
    value.strip_prefix('%').unwrap_or(value)
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    needle.is_empty() || normalize_search(haystack).contains(&normalize_search(needle))
}

fn parse_capture_time(value: &str) -> Option<i128> {
    let value = value.trim();
    let (value, offset_minutes) = split_timezone(value)?;
    let (date, time) = value.split_once('T').or_else(|| value.split_once(' '))?;
    let year = parse_fixed(date.get(0..4)?)?;
    let month = parse_fixed(date.get(5..7)?)?;
    let day = parse_fixed(date.get(8..10)?)?;
    let date_separator = date.as_bytes().get(4).copied()?;
    let second_separator = date.as_bytes().get(7).copied()?;
    if !matches!(date_separator, b':' | b'-') || date_separator != second_separator {
        return None;
    }
    let (hour, minute, second, fraction) = parse_time(time)?;
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = i128::from(days) * 86_400
        + i128::from(hour) * 3_600
        + i128::from(minute) * 60
        + i128::from(second.min(59))
        - i128::from(offset_minutes) * 60;
    Some(seconds * 1_000_000 + i128::from(fraction) + i128::from(second / 60) * 1_000_000)
}

fn split_timezone(value: &str) -> Option<(&str, i32)> {
    if value.ends_with(['Z', 'z']) {
        return Some((&value[..value.len() - 1], 0));
    }
    let Some((index, sign)) = value.char_indices().rev().find_map(|(index, character)| {
        (index >= 10 && matches!(character, '+' | '-')).then_some((index, character))
    }) else {
        return Some((value, 0));
    };
    let timezone = &value[index + 1..];
    let (hours, minutes) = match timezone.split_once(':') {
        Some((hours, minutes)) => (parse_fixed(hours)?, parse_fixed(minutes)?),
        None if timezone.len() == 4 => (
            parse_fixed(timezone.get(..2)?)?,
            parse_fixed(timezone.get(2..)?)?,
        ),
        None => return None,
    };
    if hours > 23 || minutes > 59 {
        return None;
    }
    let offset = i32::try_from(hours).ok()?.saturating_mul(60) + i32::try_from(minutes).ok()?;
    Some((&value[..index], if sign == '+' { offset } else { -offset }))
}

fn parse_time(value: &str) -> Option<(u32, u32, u32, u32)> {
    let (hour, remainder) = value.split_once(':')?;
    let (minute, remainder) = remainder.split_once(':')?;
    let (second, fraction) = remainder
        .split_once(['.', ','])
        .map_or((remainder, ""), |(second, fraction)| (second, fraction));
    let fraction = if fraction.is_empty() {
        0
    } else {
        if !fraction.bytes().all(|byte| byte.is_ascii_digit()) || fraction.len() > 6 {
            return None;
        }
        let value = parse_fixed(fraction)?;
        value * 10_u32.pow(u32::try_from(6 - fraction.len()).ok()?)
    };
    Some((
        parse_fixed(hour)?,
        parse_fixed(minute)?,
        parse_fixed(second)?,
        fraction,
    ))
}

fn parse_fixed(value: &str) -> Option<u32> {
    (!value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())?
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        2 if year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400)) => {
            29
        }
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Returns days since 1970-01-01 in the proleptic Gregorian calendar.
fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = if year >= 0 {
        year / 400
    } else {
        (year - 399) / 400
    };
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, PhotoId};

    use super::{CollectionItem, CollectionProperty, CollectionRule};

    fn item(path: &str) -> CollectionItem {
        CollectionItem::new(PhotoId::new(7).expect("non-zero photo ID"), path)
    }

    #[test]
    fn derives_darktable_path_properties() {
        let item = item("/photos/2026/holiday/IMG_0001.CR3");

        assert_eq!(item.path(), "/photos/2026/holiday/IMG_0001.CR3");
        assert_eq!(item.filmroll(), "/photos/2026/holiday");
        assert_eq!(item.folders(), "/photos/2026/holiday");
        assert_eq!(item.filename(), "IMG_0001.CR3");
    }

    #[test]
    fn canonical_sort_keys_and_capture_metadata_are_display_independent() {
        let metadata = ImageMetadata::from_entries([MetadataEntry::CaptureDateTimeOriginal(
            MetadataText::new("2024:01:02 05:04:05+02:00").expect("capture time"),
        )])
        .expect("metadata");
        let item = CollectionItem::new(
            PhotoId::new(8).expect("non-zero photo ID"),
            "/photos/Été/IMG_02.CR3",
        )
        .with_capture_metadata(&metadata);
        let utc = CollectionItem::new(
            PhotoId::new(9).expect("non-zero photo ID"),
            "/photos/été/IMG_09.CR3",
        )
        .with_capture_metadata(
            &ImageMetadata::from_entries([MetadataEntry::CaptureDateTimeOriginal(
                MetadataText::new("2024-01-02T03:04:05Z").expect("capture time"),
            )])
            .expect("metadata"),
        );

        assert_eq!(item.filename_sort_key(), "img_02.cr3");
        assert_eq!(item.path_sort_key(), "/photos/été/img_02.cr3");
        assert_eq!(item.capture_time_sort_key(), utc.capture_time_sort_key());
    }

    #[test]
    fn invalid_or_missing_capture_metadata_has_no_sort_key() {
        let invalid = ImageMetadata::from_entries([MetadataEntry::CaptureDateTimeOriginal(
            MetadataText::new("not-a-date").expect("capture text"),
        )])
        .expect("metadata");
        assert_eq!(item("/photos/missing.jpg").capture_time_sort_key(), None);
        assert_eq!(
            item("/photos/invalid.jpg")
                .with_capture_metadata(&invalid)
                .capture_time_sort_key(),
            None
        );
    }

    #[test]
    fn property_indices_match_the_dropdown_order() {
        assert_eq!(CollectionProperty::ALL.len(), 5);
        for property in CollectionProperty::ALL {
            assert_eq!(
                CollectionProperty::from_index(property.index()),
                Some(property)
            );
        }
        assert_eq!(CollectionProperty::from_index(5), None);
    }

    #[test]
    fn empty_rule_matches_every_item() {
        let mut rule = CollectionRule::new(CollectionProperty::Filename);
        let photo = item("/photos/one.jpg");

        assert!(rule.matches(&photo));
        rule.set_search_text("   ");
        assert!(rule.matches(&photo));
    }

    #[test]
    fn filename_search_is_case_insensitive_and_supports_comma_alternatives() {
        let photo = item("/photos/holiday/IMG_0001.CR3");
        let mut rule = CollectionRule::new(CollectionProperty::Filename);

        rule.set_search_text("missing, %0001");
        assert!(rule.matches(&photo));
        rule.set_search_text("missing, portrait");
        assert!(!rule.matches(&photo));
    }

    #[test]
    fn folder_search_accepts_darktable_trailing_wildcards_and_slashes() {
        let photo = item("/photos/2026/holiday/IMG_0001.CR3");
        let mut rule = CollectionRule::new(CollectionProperty::Folders);

        rule.set_search_text("/PHOTOS/2026/");
        assert!(rule.matches(&photo));
        rule.set_search_text("/photos/2026/*");
        assert!(rule.matches(&photo));
        rule.set_search_text("/photos/2025");
        assert!(!rule.matches(&photo));
    }
}
