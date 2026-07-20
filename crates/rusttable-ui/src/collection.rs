//! Pure collection-rule models shared by the catalog controller and GTK controls.

use std::path::Path;

use rusttable_core::PhotoId;

/// The first Darktable collection properties supported by `RustTable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CollectionProperty {
    /// The containing film-roll directory.
    Filmroll,
    /// The containing folder path.
    Folders,
    /// The source filename.
    #[default]
    Filename,
}

impl CollectionProperty {
    /// All supported properties in the same order used by the GTK dropdown.
    pub const ALL: [Self; 3] = [Self::Filmroll, Self::Folders, Self::Filename];

    /// Returns the display label used by the collection control.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Filmroll => "filmroll",
            Self::Folders => "folders",
            Self::Filename => "filename",
        }
    }

    /// Returns the dropdown index for this property.
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Filmroll => 0,
            Self::Folders => 1,
            Self::Filename => 2,
        }
    }

    /// Converts a GTK dropdown index into a supported property.
    #[must_use]
    pub const fn from_index(index: u32) -> Option<Self> {
        match index {
            0 => Some(Self::Filmroll),
            1 => Some(Self::Folders),
            2 => Some(Self::Filename),
            _ => None,
        }
    }
}

/// The searchable path projection for one imported photo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionItem {
    photo_id: PhotoId,
    path: String,
    filmroll: String,
    folders: String,
    filename: String,
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

        Self {
            photo_id,
            path,
            filmroll: folders.clone(),
            folders,
            filename,
        }
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

    /// Returns the searchable value for one collection property.
    #[must_use]
    pub fn value(&self, property: CollectionProperty) -> &str {
        match property {
            CollectionProperty::Filmroll => self.filmroll(),
            CollectionProperty::Folders => self.folders(),
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
        let needle = self.search_text.trim();
        if needle.is_empty() {
            return true;
        }

        let haystack = item.value(self.property);
        match self.property {
            CollectionProperty::Filename => needle.split(',').any(|term| {
                contains_case_insensitive(haystack, strip_prefix_wildcard(term.trim()))
            }),
            CollectionProperty::Filmroll | CollectionProperty::Folders => {
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
    needle.is_empty() || haystack.to_lowercase().contains(&needle.to_lowercase())
}

#[cfg(test)]
mod tests {
    use rusttable_core::PhotoId;

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
    fn property_indices_match_the_dropdown_order() {
        assert_eq!(CollectionProperty::ALL.len(), 3);
        for property in CollectionProperty::ALL {
            assert_eq!(
                CollectionProperty::from_index(property.index()),
                Some(property)
            );
        }
        assert_eq!(CollectionProperty::from_index(3), None);
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
