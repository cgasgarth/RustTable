use crate::domain::{
    Confidence, DatePrecision, DomainValue, GpsCoordinate, HierarchicalKeywords,
    LanguageAlternative, LanguageTag, MAX_METADATA_CODEC_BYTES, MAX_METADATA_KEY_BYTES,
    MAX_METADATA_LIST_ITEMS, MAX_METADATA_RAW_BYTES, MAX_METADATA_RECORDS, MetadataDateTime,
    MetadataDocument, MetadataDomainError, MetadataKey, MetadataNamespace, MetadataProvenance,
    MetadataRecord, NormalizationWarning, PrivacyClass, Rational, RawRepresentation,
    StructuredValue,
};
use crate::policy::MetadataSource;
use rusttable_core::Orientation;

const MAGIC: &[u8] = b"RustTableMetadata\0";
const VERSION: u16 = 1;

/// Deterministic, length-delimited binary representation of the canonical domain.
pub struct CanonicalCodec;

impl CanonicalCodec {
    /// Encodes a document into stable, bounded canonical bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when the document cannot fit the codec bounds.
    pub fn encode(document: &MetadataDocument) -> Result<Vec<u8>, MetadataDomainError> {
        let mut writer = Writer::new();
        writer.bytes(MAGIC);
        writer.u16(VERSION);
        writer.u32(u32::try_from(document.len()).map_err(|_| MetadataDomainError::TooManyRecords)?);
        for record in document.records() {
            encode_namespace(&mut writer, record.key().namespace())?;
            writer.string(record.key().name())?;
            encode_value(&mut writer, record.value())?;
            encode_provenance(&mut writer, record.provenance())?;
        }
        if writer.bytes.len() > MAX_METADATA_CODEC_BYTES {
            return Err(MetadataDomainError::DocumentTooLarge);
        }
        Ok(writer.bytes)
    }

    /// Decodes and validates canonical bytes without exposing parser types.
    ///
    /// # Errors
    ///
    /// Returns an error for truncated, malformed, out-of-version, or out-of-bounds input.
    pub fn decode(bytes: &[u8]) -> Result<MetadataDocument, MetadataDomainError> {
        if bytes.len() > MAX_METADATA_CODEC_BYTES {
            return Err(MetadataDomainError::DocumentTooLarge);
        }
        let mut reader = Reader { bytes, position: 0 };
        if reader.take(MAGIC.len())? != MAGIC {
            return Err(MetadataDomainError::InvalidCodec("magic"));
        }
        if reader.u16()? != VERSION {
            return Err(MetadataDomainError::InvalidCodec("version"));
        }
        let count = reader.count(MAX_METADATA_RECORDS)?;
        let mut records = Vec::with_capacity(count);
        for _ in 0..count {
            let namespace = decode_namespace(&mut reader)?;
            let key = MetadataKey::new(namespace, reader.string(MAX_METADATA_KEY_BYTES)?)?;
            let value = decode_value(&mut reader, 0)?;
            let provenance = decode_provenance(&mut reader)?;
            records.push(MetadataRecord::new(key, value, provenance));
        }
        if reader.position != bytes.len() {
            return Err(MetadataDomainError::InvalidCodec("trailing bytes"));
        }
        MetadataDocument::from_records(records)
    }
}

fn encode_namespace(
    writer: &mut Writer,
    namespace: &MetadataNamespace,
) -> Result<(), MetadataDomainError> {
    match namespace {
        MetadataNamespace::Exif => writer.u8(0),
        MetadataNamespace::Iptc => writer.u8(1),
        MetadataNamespace::Xmp => writer.u8(2),
        MetadataNamespace::DublinCore => writer.u8(3),
        MetadataNamespace::Photoshop => writer.u8(4),
        MetadataNamespace::Unknown(value) => {
            writer.u8(255);
            writer.string(value)?;
        }
    }
    Ok(())
}

fn decode_namespace(reader: &mut Reader<'_>) -> Result<MetadataNamespace, MetadataDomainError> {
    match reader.u8()? {
        0 => Ok(MetadataNamespace::Exif),
        1 => Ok(MetadataNamespace::Iptc),
        2 => Ok(MetadataNamespace::Xmp),
        3 => Ok(MetadataNamespace::DublinCore),
        4 => Ok(MetadataNamespace::Photoshop),
        255 => MetadataNamespace::unknown(reader.string(256)?),
        _ => Err(MetadataDomainError::InvalidCodec("namespace")),
    }
}

fn encode_value(writer: &mut Writer, value: &DomainValue) -> Result<(), MetadataDomainError> {
    match value {
        DomainValue::Boolean(value) => {
            writer.u8(0);
            writer.u8(u8::from(*value));
        }
        DomainValue::Integer(value) => {
            writer.u8(1);
            writer.i64(*value);
        }
        DomainValue::Unsigned(value) => {
            writer.u8(2);
            writer.u64(*value);
        }
        DomainValue::Rational(value) => {
            writer.u8(3);
            encode_rational(writer, *value);
        }
        DomainValue::Text(value) => {
            writer.u8(4);
            writer.string(value)?;
        }
        DomainValue::Binary(value) => {
            writer.u8(5);
            writer.blob(value)?;
        }
        DomainValue::List(values) => {
            writer.u8(6);
            writer.count(values.len())?;
            for value in values {
                encode_value(writer, value)?;
            }
        }
        DomainValue::LanguageAlternative(value) => {
            writer.u8(7);
            writer.count(value.values.len())?;
            for (tag, text) in &value.values {
                writer.string(tag.as_str())?;
                writer.string(text)?;
            }
        }
        DomainValue::Structure(value) => {
            writer.u8(8);
            writer.count(value.fields.len())?;
            for (key, value) in &value.fields {
                writer.string(key)?;
                encode_value(writer, value)?;
            }
        }
        DomainValue::DateTime(value) => {
            writer.u8(9);
            encode_datetime(writer, value);
        }
        DomainValue::Gps(value) => {
            writer.u8(10);
            encode_rational(writer, value.latitude);
            encode_rational(writer, value.longitude);
            match value.altitude {
                Some(value) => {
                    writer.u8(1);
                    encode_rational(writer, value);
                }
                None => writer.u8(0),
            }
        }
        DomainValue::Orientation(value) => {
            writer.u8(11);
            writer.u8(value.code());
        }
        DomainValue::Keywords(value) => {
            writer.u8(12);
            writer.count(value.paths.len())?;
            for path in &value.paths {
                writer.count(path.len())?;
                for item in path {
                    writer.string(item)?;
                }
            }
        }
        DomainValue::Opaque(value) => {
            writer.u8(13);
            writer.string(&value.media_type)?;
            writer.blob(&value.bytes)?;
        }
    }
    Ok(())
}

fn decode_value(reader: &mut Reader<'_>, depth: usize) -> Result<DomainValue, MetadataDomainError> {
    if depth > 32 {
        return Err(MetadataDomainError::InvalidCodec("nesting"));
    }
    match reader.u8()? {
        0 => match reader.u8()? {
            0 => Ok(DomainValue::Boolean(false)),
            1 => Ok(DomainValue::Boolean(true)),
            _ => Err(MetadataDomainError::InvalidCodec("boolean")),
        },
        1 => Ok(DomainValue::Integer(reader.i64()?)),
        2 => Ok(DomainValue::Unsigned(reader.u64()?)),
        3 => Ok(DomainValue::Rational(decode_rational(reader)?)),
        4 => Ok(DomainValue::Text(
            reader.string(crate::domain::MAX_METADATA_TEXT_BYTES)?,
        )),
        5 => Ok(DomainValue::Binary(reader.blob(MAX_METADATA_RAW_BYTES)?)),
        6 => {
            let count = reader.count(MAX_METADATA_LIST_ITEMS)?;
            Ok(DomainValue::List(
                (0..count)
                    .map(|_| decode_value(reader, depth + 1))
                    .collect::<Result<_, _>>()?,
            ))
        }
        7 => {
            let count = reader.count(MAX_METADATA_LIST_ITEMS)?;
            let mut values = std::collections::BTreeMap::new();
            for _ in 0..count {
                values.insert(
                    LanguageTag::new(reader.string(64)?)?,
                    reader.string(crate::domain::MAX_METADATA_TEXT_BYTES)?,
                );
            }
            Ok(DomainValue::LanguageAlternative(LanguageAlternative::new(
                values,
            )?))
        }
        8 => {
            let count = reader.count(crate::domain::MAX_METADATA_STRUCTURED_FIELDS)?;
            let mut fields = std::collections::BTreeMap::new();
            for _ in 0..count {
                fields.insert(
                    reader.string(crate::domain::MAX_METADATA_KEY_BYTES)?,
                    decode_value(reader, depth + 1)?,
                );
            }
            Ok(DomainValue::Structure(StructuredValue::new(fields)?))
        }
        9 => Ok(DomainValue::DateTime(decode_datetime(reader)?)),
        10 => {
            let latitude = decode_rational(reader)?;
            let longitude = decode_rational(reader)?;
            let altitude = match reader.u8()? {
                0 => None,
                1 => Some(decode_rational(reader)?),
                _ => return Err(MetadataDomainError::InvalidCodec("gps altitude")),
            };
            Ok(DomainValue::Gps(GpsCoordinate::new(
                latitude, longitude, altitude,
            )?))
        }
        11 => Ok(DomainValue::Orientation(
            Orientation::from_u8(reader.u8()?)
                .map_err(|_| MetadataDomainError::InvalidCodec("orientation"))?,
        )),
        12 => {
            let count = reader.count(MAX_METADATA_LIST_ITEMS)?;
            let mut paths = Vec::with_capacity(count);
            for _ in 0..count {
                let length = reader.count(MAX_METADATA_LIST_ITEMS)?;
                paths.push(
                    (0..length)
                        .map(|_| reader.string(crate::domain::MAX_METADATA_TEXT_BYTES))
                        .collect::<Result<_, _>>()?,
                );
            }
            Ok(DomainValue::Keywords(HierarchicalKeywords::new(paths)?))
        }
        13 => Ok(DomainValue::Opaque(RawRepresentation::try_new(
            reader.string(256)?,
            reader.blob(MAX_METADATA_RAW_BYTES)?,
        )?)),
        _ => Err(MetadataDomainError::InvalidCodec("value")),
    }
}

fn encode_provenance(
    writer: &mut Writer,
    provenance: &MetadataProvenance,
) -> Result<(), MetadataDomainError> {
    writer.u8(source_code(provenance.source));
    writer.u8(provenance.confidence.get());
    writer.u8(privacy_code(provenance.privacy));
    match &provenance.raw {
        Some(raw) => {
            writer.u8(1);
            writer.string(&raw.media_type)?;
            writer.blob(&raw.bytes)?;
        }
        None => writer.u8(0),
    }
    writer.count(provenance.warnings.len())?;
    for warning in &provenance.warnings {
        writer.u8(warning_code(*warning));
    }
    Ok(())
}

fn decode_provenance(reader: &mut Reader<'_>) -> Result<MetadataProvenance, MetadataDomainError> {
    let source = decode_source(reader.u8()?)?;
    let confidence = Confidence::new(reader.u8()?)?;
    let privacy = decode_privacy(reader.u8()?)?;
    let raw = match reader.u8()? {
        0 => None,
        1 => Some(RawRepresentation::try_new(
            reader.string(256)?,
            reader.blob(MAX_METADATA_RAW_BYTES)?,
        )?),
        _ => return Err(MetadataDomainError::InvalidCodec("raw marker")),
    };
    let count = reader.count(32)?;
    let mut provenance = MetadataProvenance::new(source, confidence, privacy);
    if let Some(raw) = raw {
        provenance = provenance.with_raw(raw);
    }
    for _ in 0..count {
        provenance = provenance.with_warning(decode_warning(reader.u8()?)?);
    }
    Ok(provenance)
}

fn encode_rational(writer: &mut Writer, value: Rational) {
    writer.i64(value.numerator);
    writer.u64(value.denominator);
}
fn decode_rational(reader: &mut Reader<'_>) -> Result<Rational, MetadataDomainError> {
    Rational::new(reader.i64()?, reader.u64()?)
}

fn encode_datetime(writer: &mut Writer, value: &MetadataDateTime) {
    writer.i32(value.year);
    writer.u8(value.month);
    writer.u8(value.day);
    writer.u8(value.hour);
    writer.u8(value.minute);
    writer.u8(value.second);
    writer.u32(value.nanosecond);
    match value.timezone_offset_minutes {
        Some(offset) => {
            writer.u8(1);
            writer.i32(offset);
        }
        None => writer.u8(0),
    }
    writer.u8(precision_code(value.precision));
}

fn decode_datetime(reader: &mut Reader<'_>) -> Result<MetadataDateTime, MetadataDomainError> {
    let year = reader.i32()?;
    let month = reader.u8()?;
    let day = reader.u8()?;
    let hour = reader.u8()?;
    let minute = reader.u8()?;
    let second = reader.u8()?;
    let nanosecond = reader.u32()?;
    let timezone = match reader.u8()? {
        0 => None,
        1 => Some(reader.i32()?),
        _ => return Err(MetadataDomainError::InvalidCodec("timezone marker")),
    };
    MetadataDateTime::new(
        year,
        month,
        day,
        hour,
        minute,
        second,
        nanosecond,
        timezone,
        decode_precision(reader.u8()?)?,
    )
}

fn source_code(source: MetadataSource) -> u8 {
    match source {
        MetadataSource::Exif => 0,
        MetadataSource::Iptc => 1,
        MetadataSource::Xmp => 2,
        MetadataSource::Imported => 3,
        MetadataSource::CatalogEdit => 4,
        MetadataSource::RecipeOverride => 5,
        MetadataSource::GeneratedTechnical => 6,
        MetadataSource::Container => 7,
        MetadataSource::MakerNote => 8,
        MetadataSource::EmbeddedXmp => 9,
        MetadataSource::SidecarXmp => 10,
        MetadataSource::ImportDefault => 11,
        MetadataSource::CatalogValue => 12,
        MetadataSource::UserOverride => 13,
        MetadataSource::ExportOverride => 14,
    }
}
fn decode_source(code: u8) -> Result<MetadataSource, MetadataDomainError> {
    match code {
        0 => Ok(MetadataSource::Exif),
        1 => Ok(MetadataSource::Iptc),
        2 => Ok(MetadataSource::Xmp),
        3 => Ok(MetadataSource::Imported),
        4 => Ok(MetadataSource::CatalogEdit),
        5 => Ok(MetadataSource::RecipeOverride),
        6 => Ok(MetadataSource::GeneratedTechnical),
        7 => Ok(MetadataSource::Container),
        8 => Ok(MetadataSource::MakerNote),
        9 => Ok(MetadataSource::EmbeddedXmp),
        10 => Ok(MetadataSource::SidecarXmp),
        11 => Ok(MetadataSource::ImportDefault),
        12 => Ok(MetadataSource::CatalogValue),
        13 => Ok(MetadataSource::UserOverride),
        14 => Ok(MetadataSource::ExportOverride),
        _ => Err(MetadataDomainError::InvalidCodec("source")),
    }
}
fn privacy_code(value: PrivacyClass) -> u8 {
    match value {
        PrivacyClass::Public => 0,
        PrivacyClass::Personal => 1,
        PrivacyClass::Sensitive => 2,
        PrivacyClass::Location => 3,
    }
}
fn decode_privacy(code: u8) -> Result<PrivacyClass, MetadataDomainError> {
    match code {
        0 => Ok(PrivacyClass::Public),
        1 => Ok(PrivacyClass::Personal),
        2 => Ok(PrivacyClass::Sensitive),
        3 => Ok(PrivacyClass::Location),
        _ => Err(MetadataDomainError::InvalidCodec("privacy")),
    }
}
fn warning_code(value: NormalizationWarning) -> u8 {
    match value {
        NormalizationWarning::UnicodeNfc => 0,
        NormalizationWarning::Trimmed => 1,
        NormalizationWarning::RepairedWhitespace => 2,
        NormalizationWarning::ParserSpecificRepresentation => 3,
        NormalizationWarning::LossyConversion => 4,
    }
}
fn decode_warning(code: u8) -> Result<NormalizationWarning, MetadataDomainError> {
    match code {
        0 => Ok(NormalizationWarning::UnicodeNfc),
        1 => Ok(NormalizationWarning::Trimmed),
        2 => Ok(NormalizationWarning::RepairedWhitespace),
        3 => Ok(NormalizationWarning::ParserSpecificRepresentation),
        4 => Ok(NormalizationWarning::LossyConversion),
        _ => Err(MetadataDomainError::InvalidCodec("warning")),
    }
}
fn precision_code(value: DatePrecision) -> u8 {
    match value {
        DatePrecision::Year => 0,
        DatePrecision::Month => 1,
        DatePrecision::Day => 2,
        DatePrecision::Hour => 3,
        DatePrecision::Minute => 4,
        DatePrecision::Second => 5,
        DatePrecision::Millisecond => 6,
        DatePrecision::Microsecond => 7,
        DatePrecision::Nanosecond => 8,
    }
}
fn decode_precision(code: u8) -> Result<DatePrecision, MetadataDomainError> {
    match code {
        0 => Ok(DatePrecision::Year),
        1 => Ok(DatePrecision::Month),
        2 => Ok(DatePrecision::Day),
        3 => Ok(DatePrecision::Hour),
        4 => Ok(DatePrecision::Minute),
        5 => Ok(DatePrecision::Second),
        6 => Ok(DatePrecision::Millisecond),
        7 => Ok(DatePrecision::Microsecond),
        8 => Ok(DatePrecision::Nanosecond),
        _ => Err(MetadataDomainError::InvalidCodec("date precision")),
    }
}

struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }
    fn bytes(&mut self, value: &[u8]) {
        self.bytes.extend_from_slice(value);
    }
    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn string(&mut self, value: &str) -> Result<(), MetadataDomainError> {
        self.blob(value.as_bytes())
    }
    fn blob(&mut self, value: &[u8]) -> Result<(), MetadataDomainError> {
        self.u32(u32::try_from(value.len()).map_err(|_| MetadataDomainError::DocumentTooLarge)?);
        self.bytes(value);
        Ok(())
    }
    fn count(&mut self, value: usize) -> Result<(), MetadataDomainError> {
        self.u32(u32::try_from(value).map_err(|_| MetadataDomainError::TooManyItems)?);
        Ok(())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], MetadataDomainError> {
        let end = self
            .position
            .checked_add(length)
            .ok_or(MetadataDomainError::InvalidCodec("length"))?;
        let value = self
            .bytes
            .get(self.position..end)
            .ok_or(MetadataDomainError::InvalidCodec("truncated"))?;
        self.position = end;
        Ok(value)
    }
    fn u8(&mut self) -> Result<u8, MetadataDomainError> {
        Ok(*self
            .take(1)?
            .first()
            .ok_or(MetadataDomainError::InvalidCodec("truncated"))?)
    }
    fn u16(&mut self) -> Result<u16, MetadataDomainError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| MetadataDomainError::InvalidCodec("u16"))?,
        ))
    }
    fn u32(&mut self) -> Result<u32, MetadataDomainError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| MetadataDomainError::InvalidCodec("u32"))?,
        ))
    }
    fn i32(&mut self) -> Result<i32, MetadataDomainError> {
        Ok(i32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| MetadataDomainError::InvalidCodec("i32"))?,
        ))
    }
    fn u64(&mut self) -> Result<u64, MetadataDomainError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| MetadataDomainError::InvalidCodec("u64"))?,
        ))
    }
    fn i64(&mut self) -> Result<i64, MetadataDomainError> {
        Ok(i64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| MetadataDomainError::InvalidCodec("i64"))?,
        ))
    }
    fn count(&mut self, limit: usize) -> Result<usize, MetadataDomainError> {
        let value =
            usize::try_from(self.u32()?).map_err(|_| MetadataDomainError::InvalidCodec("count"))?;
        if value > limit {
            return Err(MetadataDomainError::InvalidCodec("count bound"));
        }
        Ok(value)
    }
    fn string(&mut self, limit: usize) -> Result<String, MetadataDomainError> {
        let value = self.blob(limit)?;
        String::from_utf8(value).map_err(|_| MetadataDomainError::InvalidCodec("utf8"))
    }
    fn blob(&mut self, limit: usize) -> Result<Vec<u8>, MetadataDomainError> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| MetadataDomainError::InvalidCodec("length"))?;
        if length > limit {
            return Err(MetadataDomainError::InvalidCodec("length bound"));
        }
        Ok(self.take(length)?.to_vec())
    }
}
