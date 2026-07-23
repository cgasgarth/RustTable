use std::collections::BTreeMap;
use std::fmt;

use unicode_normalization::UnicodeNormalization;

use crate::policy::MetadataSource;
use rusttable_core::Orientation;

pub const MAX_METADATA_RECORDS: usize = 4_096;
pub const MAX_METADATA_TEXT_BYTES: usize = 4_096;
pub const MAX_METADATA_KEY_BYTES: usize = 256;
pub const MAX_METADATA_NAMESPACE_BYTES: usize = 256;
pub const MAX_METADATA_LIST_ITEMS: usize = 4_096;
pub const MAX_METADATA_STRUCTURED_FIELDS: usize = 256;
pub const MAX_METADATA_RAW_BYTES: usize = 1 << 20;
pub const MAX_METADATA_CODEC_BYTES: usize = 16 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalField {
    CameraMake,
    CameraModel,
    LensModel,
    CaptureDateTimeOriginal,
    Orientation,
    ExposureTime,
    FNumber,
    IsoSpeed,
    FocalLength,
    GpsLatitude,
    GpsLongitude,
    GpsAltitude,
    Rights,
    Caption,
    People,
    Keywords,
    HierarchicalKeywords,
    Rating,
    Creator,
    Copyright,
    Software,
    Description,
}

impl CanonicalField {
    #[must_use]
    pub const fn as_key(self) -> &'static str {
        match self {
            Self::CameraMake => "camera.make",
            Self::CameraModel => "camera.model",
            Self::LensModel => "lens.model",
            Self::CaptureDateTimeOriginal => "capture.datetime",
            Self::Orientation => "orientation",
            Self::ExposureTime => "camera.exposure-time",
            Self::FNumber => "camera.f-number",
            Self::IsoSpeed => "camera.iso-speed",
            Self::FocalLength => "lens.focal-length",
            Self::GpsLatitude => "gps.latitude",
            Self::GpsLongitude => "gps.longitude",
            Self::GpsAltitude => "gps.altitude",
            Self::Rights => "rights",
            Self::Caption => "caption",
            Self::People => "people",
            Self::Keywords => "keywords",
            Self::HierarchicalKeywords => "keywords.hierarchical",
            Self::Rating => "rating",
            Self::Creator => "creator",
            Self::Copyright => "copyright",
            Self::Software => "software",
            Self::Description => "description",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MetadataNamespace {
    Exif,
    Iptc,
    Xmp,
    DublinCore,
    Photoshop,
    Unknown(String),
}

impl MetadataNamespace {
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Exif => "exif",
            Self::Iptc => "iptc",
            Self::Xmp => "xmp",
            Self::DublinCore => "dc",
            Self::Photoshop => "photoshop",
            Self::Unknown(value) => value.as_str(),
        }
    }

    /// Creates a bounded namespace for a vendor or future standard.
    ///
    /// # Errors
    ///
    /// Returns an error when the namespace is empty, malformed, oversized, or reserved.
    pub fn unknown(value: impl Into<String>) -> Result<Self, MetadataDomainError> {
        let value = value.into();
        let value = normalize_identifier(&value, MAX_METADATA_NAMESPACE_BYTES, "namespace")?;
        if matches!(value.as_str(), "exif" | "iptc" | "xmp" | "dc" | "photoshop") {
            return Err(MetadataDomainError::ReservedNamespace);
        }
        Ok(Self::Unknown(value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataKey {
    pub(crate) namespace: MetadataNamespace,
    pub(crate) name: String,
}

impl MetadataKey {
    /// Creates a normalized, bounded key independent of parser-library tags.
    ///
    /// # Errors
    ///
    /// Returns an error when the key name is empty, malformed, or oversized.
    pub fn new(
        namespace: MetadataNamespace,
        name: impl Into<String>,
    ) -> Result<Self, MetadataDomainError> {
        let name = name.into();
        let name = normalize_identifier(&name, MAX_METADATA_KEY_BYTES, "key")?;
        Ok(Self { namespace, name })
    }

    #[must_use]
    pub const fn namespace(&self) -> &MetadataNamespace {
        &self.namespace
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn as_str(&self) -> String {
        format!("{}:{}", self.namespace.as_str(), self.name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Confidence(u8);

impl Confidence {
    /// # Errors
    ///
    /// Returns an error for values greater than 100.
    pub fn new(value: u8) -> Result<Self, MetadataDomainError> {
        if value > 100 {
            return Err(MetadataDomainError::ConfidenceOutOfRange(value));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PrivacyClass {
    Public,
    Personal,
    Sensitive,
    Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NormalizationWarning {
    UnicodeNfc,
    Trimmed,
    RepairedWhitespace,
    ParserSpecificRepresentation,
    LossyConversion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRepresentation {
    pub(crate) media_type: String,
    pub(crate) bytes: Vec<u8>,
}

impl RawRepresentation {
    #[must_use]
    pub fn new(media_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            media_type: media_type.into(),
            bytes,
        }
    }

    /// # Errors
    ///
    /// Returns an error when the media type is malformed or bytes exceed the raw-value bound.
    pub fn try_new(
        media_type: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<Self, MetadataDomainError> {
        let media_type = media_type.into();
        let media_type = normalize_identifier(&media_type, 256, "media type")?;
        if bytes.len() > MAX_METADATA_RAW_BYTES {
            return Err(MetadataDomainError::RawTooLarge(bytes.len()));
        }
        Ok(Self { media_type, bytes })
    }

    #[must_use]
    pub fn expect(self, _message: &str) -> Self {
        self
    }

    #[must_use]
    pub fn media_type(&self) -> &str {
        &self.media_type
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataProvenance {
    pub(crate) source: MetadataSource,
    pub(crate) confidence: Confidence,
    pub(crate) privacy: PrivacyClass,
    pub(crate) raw: Option<RawRepresentation>,
    pub(crate) warnings: Vec<NormalizationWarning>,
}

impl MetadataProvenance {
    #[must_use]
    pub fn new(source: MetadataSource, confidence: Confidence, privacy: PrivacyClass) -> Self {
        Self {
            source,
            confidence,
            privacy,
            raw: None,
            warnings: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_raw(mut self, raw: RawRepresentation) -> Self {
        self.raw = Some(raw);
        self
    }

    #[must_use]
    pub fn with_warning(mut self, warning: NormalizationWarning) -> Self {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
            self.warnings.sort_unstable();
        }
        self
    }

    #[must_use]
    pub const fn source(&self) -> MetadataSource {
        self.source
    }

    #[must_use]
    pub const fn confidence(&self) -> Confidence {
        self.confidence
    }

    #[must_use]
    pub const fn privacy(&self) -> PrivacyClass {
        self.privacy
    }

    #[must_use]
    pub fn raw(&self) -> Option<&RawRepresentation> {
        self.raw.as_ref()
    }

    #[must_use]
    pub fn warnings(&self) -> &[NormalizationWarning] {
        &self.warnings
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rational {
    pub(crate) numerator: i64,
    pub(crate) denominator: u64,
}

impl Rational {
    /// Creates an exact reduced rational. The denominator is always positive.
    ///
    /// # Errors
    ///
    /// Returns an error for a zero denominator or an unrepresentable reduced value.
    pub fn new(numerator: i64, denominator: u64) -> Result<Self, MetadataDomainError> {
        if denominator == 0 {
            return Err(MetadataDomainError::ZeroDenominator);
        }
        let divisor = gcd_u128(abs_i64(numerator), u128::from(denominator));
        let reduced_numerator = i128::from(numerator) / i128::try_from(divisor).unwrap_or(1);
        let reduced_denominator = u128::from(denominator) / divisor;
        Ok(Self {
            numerator: i64::try_from(reduced_numerator)
                .map_err(|_| MetadataDomainError::RationalOutOfRange)?,
            denominator: u64::try_from(reduced_denominator)
                .map_err(|_| MetadataDomainError::RationalOutOfRange)?,
        })
    }

    #[must_use]
    pub const fn numerator(self) -> i64 {
        self.numerator
    }

    #[must_use]
    pub const fn denominator(self) -> u64 {
        self.denominator
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DatePrecision {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Millisecond,
    Microsecond,
    Nanosecond,
}

impl DatePrecision {
    const fn fractional_digits(self) -> u8 {
        match self {
            Self::Year | Self::Month | Self::Day | Self::Hour | Self::Minute | Self::Second => 0,
            Self::Millisecond => 3,
            Self::Microsecond => 6,
            Self::Nanosecond => 9,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MetadataDateTime {
    pub(crate) year: i32,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) second: u8,
    pub(crate) nanosecond: u32,
    pub(crate) timezone_offset_minutes: Option<i32>,
    pub(crate) precision: DatePrecision,
}

impl MetadataDateTime {
    /// # Errors
    ///
    /// Returns an error when calendar, clock, timezone, or precision bounds are invalid.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
        nanosecond: u32,
        timezone_offset_minutes: Option<i32>,
        precision: DatePrecision,
    ) -> Result<Self, MetadataDomainError> {
        if !(1..=12).contains(&month)
            || !(1..=days_in_month(year, month)).contains(&day)
            || hour > 23
            || minute > 59
            || second > 60
            || nanosecond >= 1_000_000_000
            || timezone_offset_minutes.is_some_and(|offset| !(-1_440..=1_440).contains(&offset))
            || !nanosecond.is_multiple_of(10u32.pow(9 - u32::from(precision.fractional_digits())))
        {
            return Err(MetadataDomainError::InvalidDateTime);
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
            nanosecond,
            timezone_offset_minutes,
            precision,
        })
    }

    #[must_use]
    pub const fn year(&self) -> i32 {
        self.year
    }
    #[must_use]
    pub const fn month(&self) -> u8 {
        self.month
    }
    #[must_use]
    pub const fn day(&self) -> u8 {
        self.day
    }
    #[must_use]
    pub const fn hour(&self) -> u8 {
        self.hour
    }
    #[must_use]
    pub const fn minute(&self) -> u8 {
        self.minute
    }
    #[must_use]
    pub const fn second(&self) -> u8 {
        self.second
    }
    #[must_use]
    pub const fn nanosecond(&self) -> u32 {
        self.nanosecond
    }
    #[must_use]
    pub const fn timezone(&self) -> Option<i32> {
        self.timezone_offset_minutes
    }
    #[must_use]
    pub const fn precision(&self) -> DatePrecision {
        self.precision
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LanguageTag(String);

impl LanguageTag {
    /// # Errors
    ///
    /// Returns an error when the tag is empty or contains invalid subtags.
    pub fn new(value: impl Into<String>) -> Result<Self, MetadataDomainError> {
        let value = value.into().to_ascii_lowercase();
        if value.is_empty()
            || value.len() > 64
            || value.starts_with('-')
            || value.ends_with('-')
            || value.split('-').any(|part| {
                part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
        {
            return Err(MetadataDomainError::InvalidLanguageTag);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageAlternative {
    pub(crate) values: BTreeMap<LanguageTag, String>,
}

impl LanguageAlternative {
    /// # Errors
    ///
    /// Returns an error when a caption is invalid or the alternative count is too large.
    pub fn new(values: BTreeMap<LanguageTag, String>) -> Result<Self, MetadataDomainError> {
        if values.len() > MAX_METADATA_LIST_ITEMS {
            return Err(MetadataDomainError::TooManyItems);
        }
        let values = values
            .into_iter()
            .map(|(tag, value)| Ok((tag, normalize_text(&value, "language alternative")?)))
            .collect::<Result<BTreeMap<_, _>, MetadataDomainError>>()?;
        Ok(Self { values })
    }

    #[must_use]
    pub fn get(&self, tag: &LanguageTag) -> Option<&str> {
        self.values.get(tag).map(String::as_str)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&LanguageTag, &str)> {
        self.values.iter().map(|(tag, value)| (tag, value.as_str()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredValue {
    pub fields: BTreeMap<String, DomainValue>,
}

impl StructuredValue {
    /// # Errors
    ///
    /// Returns an error when a field name/value is invalid or the field count is too large.
    pub fn new(fields: BTreeMap<String, DomainValue>) -> Result<Self, MetadataDomainError> {
        if fields.len() > MAX_METADATA_STRUCTURED_FIELDS {
            return Err(MetadataDomainError::TooManyFields);
        }
        let mut normalized = BTreeMap::new();
        for (key, value) in fields {
            let key = normalize_identifier(&key, MAX_METADATA_KEY_BYTES, "structured key")?;
            normalized.insert(key, normalize_value(value)?);
        }
        Ok(Self { fields: normalized })
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&DomainValue> {
        self.fields.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &DomainValue)> {
        self.fields.iter().map(|(key, value)| (key.as_str(), value))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpsCoordinate {
    pub(crate) latitude: Rational,
    pub(crate) longitude: Rational,
    pub(crate) altitude: Option<Rational>,
}

impl GpsCoordinate {
    /// # Errors
    ///
    /// Returns an error when latitude or longitude lies outside its exact coordinate range.
    pub fn new(
        latitude: Rational,
        longitude: Rational,
        altitude: Option<Rational>,
    ) -> Result<Self, MetadataDomainError> {
        if !within_rational(latitude, -90, 90) || !within_rational(longitude, -180, 180) {
            return Err(MetadataDomainError::GpsOutOfRange);
        }
        Ok(Self {
            latitude,
            longitude,
            altitude,
        })
    }

    #[must_use]
    pub const fn latitude(&self) -> Rational {
        self.latitude
    }
    #[must_use]
    pub const fn longitude(&self) -> Rational {
        self.longitude
    }
    #[must_use]
    pub const fn altitude(&self) -> Option<Rational> {
        self.altitude
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HierarchicalKeywords {
    pub(crate) paths: Vec<Vec<String>>,
}

impl HierarchicalKeywords {
    /// # Errors
    ///
    /// Returns an error when a path is empty, a keyword is invalid, or a bound is exceeded.
    pub fn new(paths: Vec<Vec<String>>) -> Result<Self, MetadataDomainError> {
        if paths.len() > MAX_METADATA_LIST_ITEMS {
            return Err(MetadataDomainError::TooManyItems);
        }
        let mut normalized = Vec::with_capacity(paths.len());
        for path in paths {
            if path.is_empty() || path.len() > MAX_METADATA_LIST_ITEMS {
                return Err(MetadataDomainError::InvalidKeywordPath);
            }
            normalized.push(
                path.into_iter()
                    .map(|value| normalize_text(&value, "keyword"))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        normalized.sort_unstable();
        normalized.dedup();
        Ok(Self { paths: normalized })
    }

    #[must_use]
    pub fn paths(&self) -> &[Vec<String>] {
        &self.paths
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainValue {
    Boolean(bool),
    Integer(i64),
    Unsigned(u64),
    Rational(Rational),
    Text(String),
    Binary(Vec<u8>),
    List(Vec<DomainValue>),
    LanguageAlternative(LanguageAlternative),
    Structure(StructuredValue),
    DateTime(MetadataDateTime),
    Gps(GpsCoordinate),
    Orientation(Orientation),
    Keywords(HierarchicalKeywords),
    Opaque(RawRepresentation),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataRecord {
    key: MetadataKey,
    value: DomainValue,
    provenance: MetadataProvenance,
}

impl MetadataRecord {
    #[must_use]
    pub fn new(key: MetadataKey, value: DomainValue, provenance: MetadataProvenance) -> Self {
        Self {
            key,
            value,
            provenance,
        }
    }

    #[must_use]
    pub fn key(&self) -> &MetadataKey {
        &self.key
    }
    #[must_use]
    pub fn value(&self) -> &DomainValue {
        &self.value
    }
    #[must_use]
    pub fn provenance(&self) -> &MetadataProvenance {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MetadataDocument {
    records: BTreeMap<MetadataKey, MetadataRecord>,
}

impl MetadataDocument {
    /// # Errors
    ///
    /// Returns an error for duplicate keys, invalid values/provenance, or exceeded bounds.
    pub fn from_records<I>(records: I) -> Result<Self, MetadataDomainError>
    where
        I: IntoIterator<Item = MetadataRecord>,
    {
        let mut normalized = BTreeMap::new();
        for mut record in records {
            if normalized.len() >= MAX_METADATA_RECORDS {
                return Err(MetadataDomainError::TooManyRecords);
            }
            record.value = normalize_value(record.value)?;
            validate_provenance(&record.provenance)?;
            if normalized.insert(record.key.clone(), record).is_some() {
                return Err(MetadataDomainError::DuplicateKey);
            }
        }
        let document = Self {
            records: normalized,
        };
        if document.encoded_size_hint() > MAX_METADATA_CODEC_BYTES {
            return Err(MetadataDomainError::DocumentTooLarge);
        }
        Ok(document)
    }

    #[must_use]
    pub fn get(&self, key: &MetadataKey) -> Option<&MetadataRecord> {
        self.records.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&MetadataKey, &MetadataRecord)> {
        self.records.iter()
    }
    pub fn records(&self) -> impl Iterator<Item = &MetadataRecord> {
        self.records.values()
    }
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    fn encoded_size_hint(&self) -> usize {
        self.records
            .values()
            .map(record_size_hint)
            .sum::<usize>()
            .saturating_add(32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataDomainError {
    EmptyIdentifier(&'static str),
    ControlCharacter(&'static str),
    ContainsNul(&'static str),
    TooLong {
        field: &'static str,
        bytes: usize,
        limit: usize,
    },
    ReservedNamespace,
    ConfidenceOutOfRange(u8),
    ZeroDenominator,
    RationalOutOfRange,
    InvalidDateTime,
    InvalidLanguageTag,
    TooManyItems,
    TooManyFields,
    InvalidKeywordPath,
    RawTooLarge(usize),
    GpsOutOfRange,
    DuplicateKey,
    TooManyRecords,
    DocumentTooLarge,
    InvalidCodec(&'static str),
}

impl fmt::Display for MetadataDomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "metadata domain error: {self:?}")
    }
}

impl std::error::Error for MetadataDomainError {}

fn normalize_identifier(
    value: &str,
    limit: usize,
    field: &'static str,
) -> Result<String, MetadataDomainError> {
    let value: String = value.nfc().map(|(character, _)| character).collect();
    if value.is_empty() {
        return Err(MetadataDomainError::EmptyIdentifier(field));
    }
    if value.contains('\0') {
        return Err(MetadataDomainError::ContainsNul(field));
    }
    if value.chars().any(char::is_control) {
        return Err(MetadataDomainError::ControlCharacter(field));
    }
    if value.len() > limit {
        return Err(MetadataDomainError::TooLong {
            field,
            bytes: value.len(),
            limit,
        });
    }
    Ok(value)
}

fn normalize_text(value: &str, field: &'static str) -> Result<String, MetadataDomainError> {
    normalize_identifier(value, MAX_METADATA_TEXT_BYTES, field)
}

fn normalize_value(value: DomainValue) -> Result<DomainValue, MetadataDomainError> {
    match value {
        DomainValue::Text(value) => Ok(DomainValue::Text(normalize_text(&value, "text")?)),
        DomainValue::Binary(value) if value.len() > MAX_METADATA_RAW_BYTES => {
            Err(MetadataDomainError::RawTooLarge(value.len()))
        }
        DomainValue::List(values) => {
            if values.len() > MAX_METADATA_LIST_ITEMS {
                return Err(MetadataDomainError::TooManyItems);
            }
            Ok(DomainValue::List(
                values
                    .into_iter()
                    .map(normalize_value)
                    .collect::<Result<_, _>>()?,
            ))
        }
        DomainValue::Structure(value) => {
            Ok(DomainValue::Structure(StructuredValue::new(value.fields)?))
        }
        DomainValue::LanguageAlternative(value) => Ok(DomainValue::LanguageAlternative(value)),
        DomainValue::Keywords(value) => Ok(DomainValue::Keywords(value)),
        DomainValue::Opaque(value) => Ok(DomainValue::Opaque(value)),
        value => Ok(value),
    }
}

fn validate_provenance(provenance: &MetadataProvenance) -> Result<(), MetadataDomainError> {
    if let Some(raw) = &provenance.raw
        && raw.bytes.len() > MAX_METADATA_RAW_BYTES
    {
        return Err(MetadataDomainError::RawTooLarge(raw.bytes.len()));
    }
    Ok(())
}

fn record_size_hint(record: &MetadataRecord) -> usize {
    record.key.name.len()
        + record.key.namespace.as_str().len()
        + value_size_hint(&record.value)
        + 128
}

fn value_size_hint(value: &DomainValue) -> usize {
    match value {
        DomainValue::Text(value) => value.len(),
        DomainValue::Binary(value) => value.len(),
        DomainValue::List(values) => values.iter().map(value_size_hint).sum(),
        DomainValue::LanguageAlternative(value) => value
            .values
            .iter()
            .map(|(key, value)| key.0.len() + value.len())
            .sum(),
        DomainValue::Structure(value) => value
            .fields
            .iter()
            .map(|(key, value)| key.len() + value_size_hint(value))
            .sum(),
        DomainValue::Opaque(value) => value.media_type.len() + value.bytes.len(),
        _ => 64,
    }
}

fn abs_i64(value: i64) -> u128 {
    u128::from(value.unsigned_abs())
}

fn gcd_u128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left.max(1)
}

fn within_rational(value: Rational, minimum: i64, maximum: i64) -> bool {
    let numerator = i128::from(value.numerator());
    let denominator = i128::from(value.denominator());
    numerator >= i128::from(minimum) * denominator && numerator <= i128::from(maximum) * denominator
}

const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}
