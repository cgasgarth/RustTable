use std::collections::BTreeMap;
use std::fmt;
use std::time::Duration;

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource};
use unic_langid::LanguageIdentifier;

use crate::formatter::{
    DateValue, FormattingError, format_byte_count, format_date, format_decimal, format_duration,
    format_integer, format_list, format_percent, plural_category,
};
use crate::{Direction, LocaleError, LocaleTag};

const ENGLISH_CATALOG: &str = include_str!("../../../locales/en-US/messages.ftl");

/// Stable semantic IDs used by UI code instead of raw English lookup keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MessageId {
    /// Application title.
    AppTitle,
    /// Import command.
    ToolbarImport,
    /// Preferences command.
    ToolbarPreferences,
    /// Lighttable mode.
    WorkspaceLighttable,
    /// Darkroom mode.
    WorkspaceDarkroom,
    /// Grid tool.
    WorkspaceGrid,
    /// Zoom tool.
    WorkspaceZoomable,
    /// Culling tool.
    WorkspaceCulling,
    /// Overlay tool.
    WorkspaceOverlay,
    /// Fit tool.
    WorkspaceFit,
    /// Before/after tool.
    WorkspaceBeforeAfter,
    /// Soft-proof tool.
    WorkspaceSoftProof,
    /// Navigation panel.
    PanelNavigation,
    /// Background-jobs panel.
    PanelBackgroundJobs,
    /// Module-groups panel.
    PanelModuleGroups,
    /// Filmstrip panel.
    PanelFilmstrip,
    /// Favorites module group.
    PanelFavorites,
    /// Active module group.
    PanelActive,
    /// Tone module group.
    PanelTone,
    /// Color module group.
    PanelColor,
    /// Filmroll collection property.
    CollectionPropertyFilmroll,
    /// Folders collection property.
    CollectionPropertyFolders,
    /// Rating collection property.
    CollectionPropertyRating,
    /// Color-label collection property.
    CollectionPropertyColorLabel,
    /// Filename collection property.
    CollectionPropertyFilename,
    /// Collection search placeholder.
    CollectionSearch,
    /// Collection clear action.
    CollectionClear,
    /// Collection result count.
    CollectionResults,
    /// Collection pluralized photo count.
    CollectionPhotos,
    /// Export dialog title.
    ExportSave,
    /// Export dialog action.
    ExportSaveAction,
    /// PNG filter name.
    ExportPngImage,
    /// Export default filename.
    ExportDefaultName,
}

impl MessageId {
    /// Returns the stable catalog key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AppTitle => "app-title",
            Self::ToolbarImport => "toolbar-import",
            Self::ToolbarPreferences => "toolbar-preferences",
            Self::WorkspaceLighttable => "workspace-lighttable",
            Self::WorkspaceDarkroom => "workspace-darkroom",
            Self::WorkspaceGrid => "workspace-grid",
            Self::WorkspaceZoomable => "workspace-zoomable",
            Self::WorkspaceCulling => "workspace-culling",
            Self::WorkspaceOverlay => "workspace-overlay",
            Self::WorkspaceFit => "workspace-fit",
            Self::WorkspaceBeforeAfter => "workspace-before-after",
            Self::WorkspaceSoftProof => "workspace-soft-proof",
            Self::PanelNavigation => "panel-navigation",
            Self::PanelBackgroundJobs => "panel-background-jobs",
            Self::PanelModuleGroups => "panel-module-groups",
            Self::PanelFilmstrip => "panel-filmstrip",
            Self::PanelFavorites => "panel-favorites",
            Self::PanelActive => "panel-active",
            Self::PanelTone => "panel-tone",
            Self::PanelColor => "panel-color",
            Self::CollectionPropertyFilmroll => "collection-property-filmroll",
            Self::CollectionPropertyFolders => "collection-property-folders",
            Self::CollectionPropertyRating => "collection-property-rating",
            Self::CollectionPropertyColorLabel => "collection-property-color-label",
            Self::CollectionPropertyFilename => "collection-property-filename",
            Self::CollectionSearch => "collection-search",
            Self::CollectionClear => "collection-clear",
            Self::CollectionResults => "collection-results",
            Self::CollectionPhotos => "collection-photos",
            Self::ExportSave => "export-save",
            Self::ExportSaveAction => "export-save-action",
            Self::ExportPngImage => "export-png-image",
            Self::ExportDefaultName => "export-default-name",
        }
    }

    /// Returns every message ID in schema order.
    pub const ALL: [Self; 33] = [
        Self::AppTitle,
        Self::ToolbarImport,
        Self::ToolbarPreferences,
        Self::WorkspaceLighttable,
        Self::WorkspaceDarkroom,
        Self::WorkspaceGrid,
        Self::WorkspaceZoomable,
        Self::WorkspaceCulling,
        Self::WorkspaceOverlay,
        Self::WorkspaceFit,
        Self::WorkspaceBeforeAfter,
        Self::WorkspaceSoftProof,
        Self::PanelNavigation,
        Self::PanelBackgroundJobs,
        Self::PanelModuleGroups,
        Self::PanelFilmstrip,
        Self::PanelFavorites,
        Self::PanelActive,
        Self::PanelTone,
        Self::PanelColor,
        Self::CollectionPropertyFilmroll,
        Self::CollectionPropertyFolders,
        Self::CollectionPropertyRating,
        Self::CollectionPropertyColorLabel,
        Self::CollectionPropertyFilename,
        Self::CollectionSearch,
        Self::CollectionClear,
        Self::CollectionResults,
        Self::CollectionPhotos,
        Self::ExportSave,
        Self::ExportSaveAction,
        Self::ExportPngImage,
        Self::ExportDefaultName,
    ];
}

/// Typed values accepted by localized messages.
#[derive(Debug, Clone, PartialEq)]
pub enum MessageValue {
    /// User-provided or catalog-owned text.
    Text(String),
    /// An integer used for plural selection and number formatting.
    Integer(i64),
    /// A decimal source value represented without floating-point persistence.
    Decimal(String),
    /// A fractional value formatted as a percent.
    Percent(String),
    /// A byte count.
    ByteCount(u64),
    /// A locale-independent duration.
    Duration(Duration),
    /// A locale-independent Gregorian date.
    Date(DateValue),
    /// A list of user-visible values.
    List(Vec<String>),
    /// An enum/select key controlled by catalog authors.
    Select(String),
}

/// Named typed arguments for a Fluent message.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageArgs(BTreeMap<String, MessageValue>);

impl MessageArgs {
    /// Creates an empty argument set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a typed argument.
    pub fn set(&mut self, name: impl Into<String>, value: MessageValue) {
        self.0.insert(name.into(), value);
    }

    /// Adds an integer argument.
    #[must_use]
    pub fn integer(mut self, name: impl Into<String>, value: i64) -> Self {
        self.set(name, MessageValue::Integer(value));
        self
    }

    /// Adds text.
    #[must_use]
    pub fn text(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.set(name, MessageValue::Text(value.into()));
        self
    }

    /// Adds a list.
    #[must_use]
    pub fn list(mut self, name: impl Into<String>, value: Vec<String>) -> Self {
        self.set(name, MessageValue::List(value));
        self
    }
}

/// A bounded formatting diagnostic retained for telemetry or developer receipts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatDiagnostic {
    /// Stable diagnostic category.
    pub code: &'static str,
    /// Bounded implementation detail.
    pub detail: String,
}

/// A successfully formatted message and its fallback metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedText {
    text: String,
    locale: LocaleTag,
    fallback_used: bool,
    diagnostics: Vec<FormatDiagnostic>,
}

impl LocalizedText {
    /// Returns the rendered message.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the active locale.
    #[must_use]
    pub const fn locale(&self) -> &LocaleTag {
        &self.locale
    }

    /// Returns whether catalog fallback or safe English recovery was used.
    #[must_use]
    pub const fn fallback_used(&self) -> bool {
        self.fallback_used
    }

    /// Returns bounded diagnostics.
    #[must_use]
    pub fn diagnostics(&self) -> &[FormatDiagnostic] {
        &self.diagnostics
    }
}

impl fmt::Display for LocalizedText {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.text.fmt(formatter)
    }
}

/// Catalog construction or message lookup failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// Locale could not be parsed.
    Locale(LocaleError),
    /// A catalog could not be parsed.
    Parse(String),
    /// A message has no value.
    MissingValue(MessageId),
    /// An English source message is missing.
    MissingMessage(MessageId),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locale(error) => error.fmt(formatter),
            Self::Parse(error) => write!(formatter, "catalog parse failed: {error}"),
            Self::MissingValue(id) => {
                write!(formatter, "catalog message has no value: {}", id.as_str())
            }
            Self::MissingMessage(id) => {
                write!(formatter, "catalog message is missing: {}", id.as_str())
            }
        }
    }
}

impl std::error::Error for CatalogError {}

/// Rust-owned Fluent and ICU4X localization service.
pub struct I18n {
    locale: LocaleTag,
    catalog_locale: LocaleTag,
    direction: Direction,
    bundle: FluentBundle<FluentResource>,
    english_bundle: FluentBundle<FluentResource>,
}

impl I18n {
    /// Builds a service for a canonical locale, falling back to the nearest bundled catalog.
    ///
    /// # Errors
    ///
    /// Returns an error when the locale cannot be represented by Fluent or a bundled catalog is
    /// malformed.
    pub fn new(locale: LocaleTag) -> Result<Self, CatalogError> {
        let locale_language_id = language_id(&locale)?;
        let mut english_bundle = FluentBundle::new(vec![locale_language_id]);
        let english_resource = resource(ENGLISH_CATALOG)?;
        english_bundle
            .add_resource(english_resource)
            .map_err(|errors| CatalogError::Parse(format!("{errors:?}")))?;

        let catalog_locale = locale
            .fallback_chain()
            .into_iter()
            .find(LocaleTag::catalog_supported)
            .unwrap_or_else(LocaleTag::default_locale);
        let mut bundle = FluentBundle::new(vec![language_id(&catalog_locale)?]);
        bundle
            .add_resource(resource(ENGLISH_CATALOG)?)
            .map_err(|errors| CatalogError::Parse(format!("{errors:?}")))?;
        if catalog_locale.as_str() != "en-US" {
            bundle.add_resource_overriding(resource(catalog_source(&catalog_locale))?);
        }
        Ok(Self {
            direction: locale.direction(),
            locale,
            catalog_locale,
            bundle,
            english_bundle,
        })
    }

    /// Returns the active locale.
    #[must_use]
    pub const fn locale(&self) -> &LocaleTag {
        &self.locale
    }

    /// Returns the active text direction.
    #[must_use]
    pub const fn direction(&self) -> Direction {
        self.direction
    }

    /// Returns the catalog that supplied the translated message.
    #[must_use]
    pub const fn catalog_locale(&self) -> &LocaleTag {
        &self.catalog_locale
    }

    /// Formats a message using Fluent structure and ICU4X-backed typed arguments.
    ///
    /// # Errors
    ///
    /// Returns an error when the requested message or its value is absent from the validated
    /// English source catalog.
    pub fn format(
        &self,
        id: MessageId,
        values: &MessageArgs,
    ) -> Result<LocalizedText, CatalogError> {
        let fluent_args = self.fluent_args(values);
        let mut errors = Vec::new();
        let message = self
            .bundle
            .get_message(id.as_str())
            .ok_or(CatalogError::MissingMessage(id))?;
        let pattern = message.value().ok_or(CatalogError::MissingValue(id))?;
        let text = self
            .bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors)
            .into_owned();
        if errors.is_empty() {
            return Ok(LocalizedText {
                text,
                locale: self.locale.clone(),
                fallback_used: self.catalog_locale != self.locale,
                diagnostics: Vec::new(),
            });
        }
        let mut english_errors = Vec::new();
        let english_message = self
            .english_bundle
            .get_message(id.as_str())
            .ok_or(CatalogError::MissingMessage(id))?;
        let english_pattern = english_message
            .value()
            .ok_or(CatalogError::MissingValue(id))?;
        let safe_text = self
            .english_bundle
            .format_pattern(english_pattern, Some(&fluent_args), &mut english_errors)
            .into_owned();
        Ok(LocalizedText {
            text: safe_text,
            locale: self.locale.clone(),
            fallback_used: true,
            diagnostics: vec![FormatDiagnostic {
                code: "fluent-format-fallback",
                detail: format!("{} errors", errors.len())
                    .chars()
                    .take(160)
                    .collect(),
            }],
        })
    }

    /// Formats a message and returns a bounded safe marker if a catalog is unexpectedly broken.
    #[must_use]
    pub fn text(&self, id: MessageId, values: &MessageArgs) -> String {
        self.format(id, values)
            .map_or_else(|_| "⚠".to_owned(), |localized| localized.text)
    }

    /// Formats a typed value through ICU4X for use in Fluent arguments or GTK labels.
    ///
    /// # Errors
    ///
    /// Returns an error when ICU4X data cannot be loaded or the source value is malformed.
    pub fn format_value(&self, value: &MessageValue) -> Result<String, FormattingError> {
        match value {
            MessageValue::Text(value) | MessageValue::Select(value) => Ok(value.clone()),
            MessageValue::Integer(value) => format_integer(&self.locale, *value),
            MessageValue::Decimal(value) => format_decimal(&self.locale, value),
            MessageValue::Percent(value) => format_percent(&self.locale, value),
            MessageValue::ByteCount(value) => format_byte_count(&self.locale, *value),
            MessageValue::Duration(value) => format_duration(&self.locale, *value),
            MessageValue::Date(value) => format_date(&self.locale, *value),
            MessageValue::List(value) => format_list(&self.locale, value),
        }
    }

    /// Selects a locale-specific plural category through ICU4X.
    ///
    /// # Errors
    ///
    /// Returns an error when ICU4X plural data is unavailable for the locale.
    pub fn plural_category(
        &self,
        value: i64,
    ) -> Result<icu_plurals::PluralCategory, FormattingError> {
        plural_category(&self.locale, value)
    }

    fn fluent_args(&self, values: &MessageArgs) -> FluentArgs<'static> {
        let mut args = FluentArgs::new();
        for (name, value) in &values.0 {
            match value {
                MessageValue::Text(value) | MessageValue::Select(value) => {
                    args.set(name.clone(), value.clone());
                }
                MessageValue::Integer(value) => {
                    args.set(name.clone(), *value);
                    args.set(
                        format!("{name}_text"),
                        format_integer(&self.locale, *value).unwrap_or_else(|_| value.to_string()),
                    );
                }
                MessageValue::Decimal(value) => {
                    args.set(name.clone(), value.parse::<f64>().unwrap_or(0.0));
                    args.set(
                        format!("{name}_text"),
                        format_decimal(&self.locale, value).unwrap_or_else(|_| value.clone()),
                    );
                }
                MessageValue::Percent(value) => {
                    args.set(name.clone(), value.parse::<f64>().unwrap_or(0.0));
                    args.set(
                        format!("{name}_text"),
                        format_percent(&self.locale, value).unwrap_or_else(|_| value.clone()),
                    );
                }
                MessageValue::ByteCount(value) => {
                    args.set(name.clone(), i64::try_from(*value).unwrap_or(i64::MAX));
                    args.set(
                        format!("{name}_text"),
                        format_byte_count(&self.locale, *value)
                            .unwrap_or_else(|_| value.to_string()),
                    );
                }
                MessageValue::Duration(value) => {
                    args.set(
                        name.clone(),
                        format_duration(&self.locale, *value).unwrap_or_default(),
                    );
                }
                MessageValue::Date(value) => {
                    args.set(
                        name.clone(),
                        format_date(&self.locale, *value).unwrap_or_default(),
                    );
                }
                MessageValue::List(value) => {
                    args.set(
                        name.clone(),
                        format_list(&self.locale, value).unwrap_or_default(),
                    );
                }
            }
        }
        args
    }
}

impl Default for I18n {
    fn default() -> Self {
        Self::new(LocaleTag::default_locale())
            .unwrap_or_else(|_| unreachable!("bundled English catalog is valid"))
    }
}

fn language_id(locale: &LocaleTag) -> Result<LanguageIdentifier, CatalogError> {
    locale
        .as_str()
        .parse::<LanguageIdentifier>()
        .map_err(|error| CatalogError::Parse(format!("{}: {error}", locale.as_str())))
}

fn resource(source: &str) -> Result<FluentResource, CatalogError> {
    FluentResource::try_new(source.to_owned())
        .map_err(|errors| CatalogError::Parse(format!("{errors:?}")))
}

fn catalog_source(locale: &LocaleTag) -> &'static str {
    match locale.as_str() {
        "de-DE" => include_str!("../../../locales/de-DE/messages.ftl"),
        "fr-FR" => include_str!("../../../locales/fr-FR/messages.ftl"),
        "en-XA" => include_str!("../../../locales/en-XA/messages.ftl"),
        "ar-XB" => include_str!("../../../locales/ar-XB/messages.ftl"),
        _ => ENGLISH_CATALOG,
    }
}

#[cfg(test)]
mod tests {
    use super::{I18n, MessageArgs, MessageId, MessageValue};
    use crate::LocaleTag;

    #[test]
    fn fluent_plural_and_typed_icu4x_arguments_are_localized() {
        let german = I18n::new(LocaleTag::parse("de-DE").unwrap()).unwrap();
        let one = german
            .format(
                MessageId::CollectionPhotos,
                &MessageArgs::new().integer("count", 1),
            )
            .unwrap();
        assert_eq!(one.text(), "\u{2068}1\u{2069} Foto");
        let number = german
            .format_value(&MessageValue::Integer(1_234_567))
            .unwrap();
        assert_eq!(number, "1.234.567");
    }

    #[test]
    fn pseudo_locales_keep_ids_stable_and_expose_direction() {
        let pseudo = I18n::new(LocaleTag::parse("ar-XB").unwrap()).unwrap();
        assert!(pseudo.direction().is_rtl());
        assert_eq!(
            pseudo.text(MessageId::ToolbarImport, &MessageArgs::new()),
            "استيراد"
        );
    }
}
