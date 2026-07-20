use std::fmt;
use std::time::Duration;

use fixed_decimal::Decimal;
use icu_datetime::FixedCalendarDateTimeFormatter;
use icu_datetime::fieldsets::YMD;
use icu_datetime::input::Date;
use icu_decimal::DecimalFormatter;
use icu_decimal::options::DecimalFormatterOptions;
use icu_list::ListFormatter;
use icu_list::options::ListFormatterOptions;
use icu_locale::Locale;
use icu_plurals::PluralRulesOptions;
use icu_plurals::{PluralCategory, PluralRules};

use crate::LocaleTag;

/// A locale-independent Gregorian date supplied by domain code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateValue {
    /// Gregorian year.
    pub year: i32,
    /// Gregorian month, one-based.
    pub month: u8,
    /// Gregorian day, one-based.
    pub day: u8,
}

/// Formatting failures are bounded and safe to surface as a diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormattingError {
    /// ICU4X could not load compiled data for the locale.
    Data(String),
    /// The decimal input was not valid.
    Decimal(String),
    /// The date was outside the Gregorian calendar's valid range.
    Date(String),
}

impl fmt::Display for FormattingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Data(error) => write!(formatter, "ICU4X data unavailable: {error}"),
            Self::Decimal(error) => write!(formatter, "invalid decimal: {error}"),
            Self::Date(error) => write!(formatter, "invalid date: {error}"),
        }
    }
}

impl std::error::Error for FormattingError {}

pub(crate) fn format_integer(locale: &LocaleTag, value: i64) -> Result<String, FormattingError> {
    let formatter = DecimalFormatter::try_new(
        locale_for_formatting(locale)?.into(),
        DecimalFormatterOptions::default(),
    )
    .map_err(|error| FormattingError::Data(error.to_string()))?;
    Ok(formatter.format_to_string(&Decimal::from(value)))
}

pub(crate) fn format_decimal(locale: &LocaleTag, value: &str) -> Result<String, FormattingError> {
    let decimal = value
        .parse::<Decimal>()
        .map_err(|error| FormattingError::Decimal(error.to_string()))?;
    let formatter = DecimalFormatter::try_new(
        locale_for_formatting(locale)?.into(),
        DecimalFormatterOptions::default(),
    )
    .map_err(|error| FormattingError::Data(error.to_string()))?;
    Ok(formatter.format_to_string(&decimal))
}

pub(crate) fn format_percent(locale: &LocaleTag, value: &str) -> Result<String, FormattingError> {
    let decimal = value
        .parse::<f64>()
        .map_err(|error| FormattingError::Decimal(error.to_string()))?;
    let percent = format_decimal(locale, &(decimal * 100.0).to_string())?;
    let suffix = if locale.language() == "ar" { "٪" } else { "%" };
    Ok(format!("{percent}{suffix}"))
}

pub(crate) fn format_byte_count(locale: &LocaleTag, value: u64) -> Result<String, FormattingError> {
    Ok(format!(
        "{} B",
        format_integer(locale, i64::try_from(value).unwrap_or(i64::MAX))?
    ))
}

pub(crate) fn format_duration(
    locale: &LocaleTag,
    value: Duration,
) -> Result<String, FormattingError> {
    let seconds = value.as_secs();
    let minutes = seconds / 60;
    let remaining_seconds = seconds % 60;
    if minutes == 0 {
        return Ok(format!(
            "{} s",
            format_integer(locale, i64::try_from(remaining_seconds).unwrap_or(i64::MAX))?
        ));
    }
    Ok(format!(
        "{} m {} s",
        format_integer(locale, i64::try_from(minutes).unwrap_or(i64::MAX))?,
        format_integer(locale, i64::try_from(remaining_seconds).unwrap_or(i64::MAX))?
    ))
}

pub(crate) fn format_date(locale: &LocaleTag, value: DateValue) -> Result<String, FormattingError> {
    let date = Date::try_new_gregorian(value.year, value.month, value.day)
        .map_err(|error| FormattingError::Date(error.to_string()))?;
    let formatter = FixedCalendarDateTimeFormatter::try_new(
        locale_for_formatting(locale)?.into(),
        YMD::medium(),
    )
    .map_err(|error| FormattingError::Data(error.to_string()))?;
    Ok(format!("{}", formatter.format(&date)))
}

pub(crate) fn format_list(
    locale: &LocaleTag,
    values: &[String],
) -> Result<String, FormattingError> {
    let formatter = ListFormatter::try_new_and(
        locale_for_formatting(locale)?.into(),
        ListFormatterOptions::default(),
    )
    .map_err(|error| FormattingError::Data(error.to_string()))?;
    Ok(formatter.format_to_string(values.iter()))
}

pub(crate) fn plural_category(
    locale: &LocaleTag,
    value: i64,
) -> Result<PluralCategory, FormattingError> {
    let rules = PluralRules::try_new(
        locale_for_formatting(locale)?.into(),
        PluralRulesOptions::default(),
    )
    .map_err(|error| FormattingError::Data(error.to_string()))?;
    Ok(rules.category_for(value))
}

fn locale_for_formatting(locale: &LocaleTag) -> Result<Locale, FormattingError> {
    locale
        .as_icu()
        .map_err(|error| FormattingError::Data(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        DateValue, format_date, format_decimal, format_duration, format_integer, format_list,
        format_percent,
    };
    use crate::LocaleTag;

    #[test]
    fn icu4x_formats_numbers_for_the_selected_locale() {
        let german = LocaleTag::parse("de-DE").unwrap();
        assert_eq!(format_integer(&german, 1_234_567).unwrap(), "1.234.567");
        assert_eq!(format_decimal(&german, "12.5").unwrap(), "12,5");
        assert_eq!(format_percent(&german, "0.125").unwrap(), "12,5%");
    }

    #[test]
    fn icu4x_formats_date_duration_and_list() {
        let french = LocaleTag::parse("fr-FR").unwrap();
        assert!(
            format_date(
                &french,
                DateValue {
                    year: 2026,
                    month: 7,
                    day: 20
                }
            )
            .unwrap()
            .contains("2026")
        );
        assert_eq!(
            format_duration(&french, Duration::from_secs(65)).unwrap(),
            "1 m 5 s"
        );
        assert_eq!(
            format_list(&french, &["A".to_owned(), "B".to_owned()]).unwrap(),
            "A et B"
        );
    }
}
