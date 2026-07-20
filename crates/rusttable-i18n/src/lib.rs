#![forbid(unsafe_code)]
#![doc = "Rust-owned localization, ICU4X formatting, and locale-aware collation services."]

mod catalog;
mod collation;
mod formatter;
mod locale;
mod search;

pub use catalog::{
    CatalogError, FormatDiagnostic, I18n, LocalizedText, MessageArgs, MessageId, MessageValue,
};
pub use collation::{
    AlternateName, CollationItem, CollationProfile, LocaleCollator, SortKey, StrengthName,
};
pub use formatter::{DateValue, FormattingError};
pub use locale::{
    Direction, LocaleError, LocaleSelection, LocaleSource, LocaleTag, ResolvedLocale,
};
pub use search::{SEARCH_PIPELINE_VERSION, normalize_search};

/// The ICU4X data and algorithm identity used by derived presentation values.
pub const ICU4X_DATA_VERSION: &str = "icu4x-2.2.0-compiled-data";
