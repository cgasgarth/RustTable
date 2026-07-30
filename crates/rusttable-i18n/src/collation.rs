use std::cmp::Ordering;
use std::fmt;

use icu_collator::options::{AlternateHandling, CaseLevel, CollatorOptions, Strength};
use icu_collator::{Collator, CollatorBorrowed};
use icu_locale::Locale;

use crate::{ICU4X_DATA_VERSION, LocaleError, LocaleTag, normalize_search};

/// Versioned description of a display-order policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollationProfile {
    locale: LocaleTag,
    strength: StrengthName,
    numeric: bool,
    case_insensitive: bool,
    alternate_handling: AlternateName,
}

/// The supported user-facing collation strengths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrengthName {
    /// Base letters only.
    Primary,
    /// Base letters plus diacritics; case remains a deterministic tie-break.
    Secondary,
    /// Full ICU4X tertiary distinction.
    Tertiary,
}

/// The handling of spaces and punctuation in a sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlternateName {
    /// Spaces and punctuation participate in the primary ordering.
    NonIgnorable,
    /// Spaces and punctuation are shifted below letter differences.
    Shifted,
}

impl CollationProfile {
    /// Creates the Darktable-style library default.
    #[must_use]
    pub fn new(locale: LocaleTag) -> Self {
        Self {
            locale,
            strength: StrengthName::Secondary,
            numeric: true,
            case_insensitive: true,
            alternate_handling: AlternateName::NonIgnorable,
        }
    }

    /// Returns the locale used for ordering.
    #[must_use]
    pub const fn locale(&self) -> &LocaleTag {
        &self.locale
    }

    /// Returns a stable identity for persisted cache invalidation.
    #[must_use]
    pub fn identity(&self) -> String {
        format!(
            "{}|locale={}|strength={:?}|numeric={}|case-insensitive={}|alternate={:?}",
            ICU4X_DATA_VERSION,
            self.locale,
            self.strength,
            self.numeric,
            self.case_insensitive,
            self.alternate_handling
        )
    }

    fn options(&self) -> CollatorOptions {
        let mut options = CollatorOptions::default();
        options.strength = Some(match self.strength {
            StrengthName::Primary => Strength::Primary,
            StrengthName::Secondary => Strength::Secondary,
            StrengthName::Tertiary => Strength::Tertiary,
        });
        options.case_level = Some(if self.case_insensitive {
            CaseLevel::Off
        } else {
            CaseLevel::On
        });
        options.alternate_handling = Some(match self.alternate_handling {
            AlternateName::NonIgnorable => AlternateHandling::NonIgnorable,
            AlternateName::Shifted => AlternateHandling::Shifted,
        });
        options
    }

    fn icu_locale(&self) -> Result<Locale, LocaleError> {
        let tag = if self.numeric && !self.locale.as_str().contains("-u-") {
            format!("{}-u-kn-true", self.locale)
        } else {
            self.locale.to_string()
        };
        tag.parse::<Locale>()
            .map_err(|error| LocaleError::Malformed(tag, error.to_string()))
    }
}

/// ICU4X collation service for catalog and library projections.
pub struct LocaleCollator {
    profile: CollationProfile,
    collator: CollatorBorrowed<'static>,
}

impl fmt::Debug for LocaleCollator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocaleCollator")
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl LocaleCollator {
    /// Creates an ICU4X collator backed exclusively by compiled Rust data.
    ///
    /// # Errors
    ///
    /// Returns an error when the locale tag is invalid or the compiled ICU4X collation data is
    /// unavailable.
    pub fn new(profile: CollationProfile) -> Result<Self, LocaleError> {
        let preferences = profile.icu_locale()?.into();
        let collator = Collator::try_new(preferences, profile.options()).map_err(|error| {
            LocaleError::Malformed(profile.locale.to_string(), error.to_string())
        })?;
        Ok(Self { profile, collator })
    }

    /// Returns the immutable collation profile.
    #[must_use]
    pub const fn profile(&self) -> &CollationProfile {
        &self.profile
    }

    /// Compares two display strings according to the selected locale.
    #[must_use]
    pub fn compare(&self, left: &str, right: &str) -> Ordering {
        self.collator.compare(left, right)
    }

    /// Generates an ICU4X sort key. Callers must invalidate durable copies by profile identity.
    #[must_use]
    pub fn sort_key(&self, value: &str) -> SortKey {
        let mut bytes = Vec::new();
        let _ = self.collator.write_sort_key_to(value, &mut bytes);
        SortKey(bytes)
    }

    /// Sorts values with deterministic normalized-text and stable-ID tie-breaks.
    pub fn sort_items(&self, items: &mut [CollationItem]) {
        items.sort_by(|left, right| {
            self.compare(&left.value, &right.value)
                .then_with(|| normalize_search(&left.value).cmp(&normalize_search(&right.value)))
                .then_with(|| left.stable_id.cmp(&right.stable_id))
        });
    }
}

/// An ICU4X-produced opaque key for in-memory or versioned derived caches.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SortKey(Vec<u8>);

impl SortKey {
    /// Returns the key bytes for a cache layer that already stores the profile identity.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// One source value plus a locale-independent deterministic tie-break identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollationItem {
    value: String,
    stable_id: u64,
}

impl CollationItem {
    /// Creates an item; `value` is never modified or persisted in normalized form.
    #[must_use]
    pub fn new(value: impl Into<String>, stable_id: u64) -> Self {
        Self {
            value: value.into(),
            stable_id,
        }
    }

    /// Returns the original display value.
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Returns the stable identity used after collation ties.
    #[must_use]
    pub const fn stable_id(&self) -> u64 {
        self.stable_id
    }
}

#[cfg(test)]
mod tests {
    use super::{CollationItem, CollationProfile, LocaleCollator};
    use crate::LocaleTag;

    #[test]
    fn numeric_collation_and_tie_breaks_match_library_expectations() {
        let collator =
            LocaleCollator::new(CollationProfile::new(LocaleTag::parse("en-US").unwrap())).unwrap();
        let mut items = vec![
            CollationItem::new("IMG 10", 10),
            CollationItem::new("IMG 2", 2),
            CollationItem::new("IMG 2", 1),
        ];
        collator.sort_items(&mut items);
        assert_eq!(items[0].stable_id(), 1);
        assert_eq!(items[1].stable_id(), 2);
        assert_eq!(items[2].value(), "IMG 10");
        assert!(!collator.sort_key("IMG 2").as_bytes().is_empty());
    }

    #[test]
    fn locale_tailoring_is_not_a_byte_order_sort() {
        let german =
            LocaleCollator::new(CollationProfile::new(LocaleTag::parse("de-DE").unwrap())).unwrap();
        let swedish =
            LocaleCollator::new(CollationProfile::new(LocaleTag::parse("sv-SE").unwrap())).unwrap();
        assert_ne!(german.compare("ä", "z"), swedish.compare("ä", "z"));
    }
}
