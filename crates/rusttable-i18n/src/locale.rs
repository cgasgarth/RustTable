use std::fmt;

use icu_locale::Locale;

const DEFAULT_LOCALE: &str = "en-US";

/// Text direction used by GTK containers and directional controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Left-to-right layout.
    Ltr,
    /// Right-to-left layout.
    Rtl,
}

impl Direction {
    /// Returns the GTK4 direction value without making GTK a dependency of this crate.
    #[must_use]
    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// A canonical, well-formed BCP-47 locale tag.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LocaleTag(String);

impl LocaleTag {
    /// Parses and canonicalizes a BCP-47 tag using ICU4X locale data.
    ///
    /// # Errors
    ///
    /// Returns an error when the tag is malformed or does not contain an explicit language.
    pub fn parse(value: &str) -> Result<Self, LocaleError> {
        let locale = value
            .parse::<Locale>()
            .map_err(|error| LocaleError::Malformed(value.to_owned(), error.to_string()))?;
        let canonical = locale.to_string();
        if canonical == "und" {
            return Err(LocaleError::Malformed(
                value.to_owned(),
                "an explicit language is required".to_owned(),
            ));
        }
        Ok(Self(canonical))
    }

    /// Returns the application's fallback locale.
    #[must_use]
    pub fn default_locale() -> Self {
        Self(DEFAULT_LOCALE.to_owned())
    }

    /// Returns the canonical BCP-47 representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the ICU4X locale value.
    pub(crate) fn as_icu(&self) -> Result<Locale, LocaleError> {
        self.0
            .parse::<Locale>()
            .map_err(|error| LocaleError::Malformed(self.0.clone(), error.to_string()))
    }

    /// Returns the locale's language subtag.
    #[must_use]
    pub fn language(&self) -> &str {
        self.0.split('-').next().unwrap_or("und")
    }

    /// Returns the direction prescribed for the locale's primary script.
    #[must_use]
    pub fn direction(&self) -> Direction {
        match self.language() {
            "ar" | "fa" | "he" | "ku" | "ps" | "sd" | "ur" | "yi" => Direction::Rtl,
            _ => Direction::Ltr,
        }
    }

    /// Returns progressively less-specific locale candidates, ending in English.
    #[must_use]
    pub fn fallback_chain(&self) -> Vec<Self> {
        let mut candidates = Vec::new();
        let mut current = self.0.clone();
        loop {
            if !candidates
                .iter()
                .any(|candidate: &Self| candidate.0 == current)
            {
                candidates.push(Self(current.clone()));
            }
            let Some((prefix, _)) = current.rsplit_once('-') else {
                break;
            };
            current = prefix.to_owned();
        }
        let english = Self::default_locale();
        if !candidates.iter().any(|candidate| candidate == &english) {
            candidates.push(english);
        }
        candidates
    }

    pub(crate) fn catalog_supported(&self) -> bool {
        matches!(
            self.as_str(),
            "en-US" | "de-DE" | "fr-FR" | "en-XA" | "ar-XB"
        )
    }
}

impl fmt::Display for LocaleTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Locale selection failure and fallback information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocaleError {
    /// The tag was malformed or had no explicit language.
    Malformed(String, String),
}

impl fmt::Display for LocaleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(value, reason) => {
                write!(formatter, "invalid locale {value:?}: {reason}")
            }
        }
    }
}

impl std::error::Error for LocaleError {}

/// The source that selected the active locale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocaleSource {
    /// An explicit user preference.
    UserPreference,
    /// The first usable operating-system language.
    OperatingSystem,
    /// The stable product fallback.
    Default,
}

/// The result of resolving user and operating-system locale preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLocale {
    locale: LocaleTag,
    source: LocaleSource,
    rejected_user_value: Option<String>,
}

impl ResolvedLocale {
    /// Returns the selected locale.
    #[must_use]
    pub const fn locale(&self) -> &LocaleTag {
        &self.locale
    }

    /// Returns the selection source.
    #[must_use]
    pub const fn source(&self) -> LocaleSource {
        self.source
    }

    /// Returns a rejected explicit value for diagnostics, without persisting it as the locale.
    #[must_use]
    pub fn rejected_user_value(&self) -> Option<&str> {
        self.rejected_user_value.as_deref()
    }
}

/// Explicit-user → OS-language-list → English locale resolution.
#[derive(Debug, Clone, Default)]
pub struct LocaleSelection {
    user_preference: Option<String>,
    operating_system_languages: Vec<String>,
}

impl LocaleSelection {
    /// Creates a selection request from explicit and OS candidates.
    #[must_use]
    pub fn new(user_preference: Option<String>, operating_system_languages: Vec<String>) -> Self {
        Self {
            user_preference,
            operating_system_languages,
        }
    }

    /// Resolves the first well-formed, shipped locale, preserving rejected user input in memory.
    #[must_use]
    pub fn resolve(&self) -> ResolvedLocale {
        let mut rejected_user_value = None;
        if let Some(value) = self.user_preference.as_deref() {
            match LocaleTag::parse(value) {
                Ok(locale) if locale.catalog_supported() => {
                    return ResolvedLocale {
                        locale,
                        source: LocaleSource::UserPreference,
                        rejected_user_value,
                    };
                }
                _ => rejected_user_value = Some(value.to_owned()),
            }
        }
        for value in &self.operating_system_languages {
            if let Ok(locale) = LocaleTag::parse(value)
                && locale.catalog_supported()
            {
                return ResolvedLocale {
                    locale,
                    source: LocaleSource::OperatingSystem,
                    rejected_user_value,
                };
            }
        }
        ResolvedLocale {
            locale: LocaleTag::default_locale(),
            source: LocaleSource::Default,
            rejected_user_value,
        }
    }

    /// Reads the conventional process language list without depending on libc/gettext.
    #[must_use]
    pub fn from_environment() -> Self {
        let user_preference = std::env::var("RUSTTABLE_LOCALE").ok();
        let operating_system_languages = ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .into_iter()
            .filter_map(|name| std::env::var(name).ok())
            .flat_map(|value| value.split(':').map(str::to_owned).collect::<Vec<_>>())
            .map(|value| value.trim_end_matches(".UTF-8").to_owned())
            .filter(|value| !value.is_empty())
            .collect();
        Self::new(user_preference, operating_system_languages)
    }
}

#[cfg(test)]
mod tests {
    use super::{Direction, LocaleSelection, LocaleSource, LocaleTag};

    #[test]
    fn canonicalizes_tags_and_builds_fallbacks() {
        let locale = LocaleTag::parse("de-de").expect("ICU4X canonicalizes locale tags");
        assert_eq!(locale.as_str(), "de-DE");
        assert_eq!(locale.fallback_chain()[1].as_str(), "de");
    }

    #[test]
    fn explicit_user_locale_wins_and_invalid_value_is_diagnostic_only() {
        let selected = LocaleSelection::new(
            Some("not a locale".to_owned()),
            vec!["fr-fr".to_owned(), "de-DE".to_owned()],
        )
        .resolve();
        assert_eq!(selected.locale().as_str(), "fr-FR");
        assert_eq!(selected.source(), LocaleSource::OperatingSystem);
        assert_eq!(selected.rejected_user_value(), Some("not a locale"));
    }

    #[test]
    fn rtl_direction_is_metadata_not_a_coordinate_transform() {
        assert_eq!(
            LocaleTag::parse("ar-XB").unwrap().direction(),
            Direction::Rtl
        );
        assert_eq!(
            LocaleTag::parse("en-US").unwrap().direction(),
            Direction::Ltr
        );
    }
}
