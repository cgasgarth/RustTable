use std::num::NonZeroU32;

use exif::{Reader, Tag, Value};
use rusttable_core::{ImageMetadata, MetadataEntry, MetadataText, Orientation, PositiveRational};
use rusttable_image::InputFormat;

use crate::container::exif_payload;
use crate::error::MetadataInputError;
use crate::limits::MetadataLimits;

pub trait MetadataInput: Send + Sync {
    /// Reads bounded metadata from an image container.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataInputError`] when the input, container, or EXIF payload
    /// is malformed, exceeds a configured limit, or cannot become canonical metadata.
    fn read_bytes(
        &self,
        format: InputFormat,
        source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError>;

    /// Reads metadata without making ancillary metadata a prerequisite for image use.
    fn read_bytes_tolerant(&self, format: InputFormat, source: &[u8]) -> MetadataReadResult {
        match self.read_bytes(format, source) {
            Ok(metadata) => MetadataReadResult::available(metadata),
            Err(error) => MetadataReadResult::unavailable(ImageMetadata::empty(), error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetadataReadResult {
    metadata: ImageMetadata,
    status: MetadataReadStatus,
}

impl MetadataReadResult {
    #[must_use]
    pub fn available(metadata: ImageMetadata) -> Self {
        Self {
            metadata,
            status: MetadataReadStatus::Available,
        }
    }

    #[must_use]
    pub fn unavailable(metadata: ImageMetadata, error: MetadataInputError) -> Self {
        Self {
            metadata,
            status: MetadataReadStatus::Unavailable(error),
        }
    }

    #[must_use]
    pub const fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn status(&self) -> &MetadataReadStatus {
        &self.status
    }

    #[must_use]
    pub fn into_parts(self) -> (ImageMetadata, MetadataReadStatus) {
        (self.metadata, self.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataReadStatus {
    Available,
    Unavailable(MetadataInputError),
}

#[derive(Debug, Clone, Copy)]
pub struct ExifMetadataInput {
    limits: MetadataLimits,
}

impl ExifMetadataInput {
    #[must_use]
    pub const fn new(limits: MetadataLimits) -> Self {
        Self { limits }
    }
}

impl MetadataInput for ExifMetadataInput {
    fn read_bytes(
        &self,
        format: InputFormat,
        source: &[u8],
    ) -> Result<ImageMetadata, MetadataInputError> {
        let actual =
            u64::try_from(source.len()).map_err(|_| MetadataInputError::ArithmeticOverflow)?;
        if actual > self.limits.source_bytes {
            return Err(MetadataInputError::SourceTooLarge {
                limit: self.limits.source_bytes,
                actual,
            });
        }
        let Some(payload) = exif_payload(format, source, self.limits)? else {
            return Ok(ImageMetadata::empty());
        };
        parse_payload(&payload, self.limits)
    }

    fn read_bytes_tolerant(&self, format: InputFormat, source: &[u8]) -> MetadataReadResult {
        let Ok(actual) = u64::try_from(source.len()) else {
            return MetadataReadResult::unavailable(
                ImageMetadata::empty(),
                MetadataInputError::ArithmeticOverflow,
            );
        };
        if actual > self.limits.source_bytes {
            return MetadataReadResult::unavailable(
                ImageMetadata::empty(),
                MetadataInputError::SourceTooLarge {
                    limit: self.limits.source_bytes,
                    actual,
                },
            );
        }
        let payload = match exif_payload(format, source, self.limits) {
            Ok(Some(payload)) => payload,
            Ok(None) => return MetadataReadResult::available(ImageMetadata::empty()),
            Err(error) => return MetadataReadResult::unavailable(ImageMetadata::empty(), error),
        };
        parse_payload_tolerant(&payload, self.limits)
    }
}

fn parse_payload(
    payload: &[u8],
    limits: MetadataLimits,
) -> Result<ImageMetadata, MetadataInputError> {
    preflight_tiff(payload, limits)?;
    let exif = Reader::new()
        .read_raw(payload.to_vec())
        .map_err(|_| MetadataInputError::MalformedExif)?;
    validate_exif(&exif, limits)?;
    canonical_entries(&exif)
}

fn parse_payload_tolerant(payload: &[u8], limits: MetadataLimits) -> MetadataReadResult {
    if let Err(error) = preflight_tiff(payload, limits) {
        return MetadataReadResult::unavailable(ImageMetadata::empty(), error);
    }
    let Ok(exif) = Reader::new().read_raw(payload.to_vec()) else {
        return MetadataReadResult::unavailable(
            ImageMetadata::empty(),
            MetadataInputError::MalformedExif,
        );
    };
    if let Err(error) = validate_exif(&exif, limits) {
        return MetadataReadResult::unavailable(ImageMetadata::empty(), error);
    }
    let (entries, error) = canonical_entries_tolerant(&exif);
    let metadata = ImageMetadata::from_entries(entries).unwrap_or_else(|_| ImageMetadata::empty());
    match error {
        Some(error) => MetadataReadResult::unavailable(metadata, error),
        None => MetadataReadResult::available(metadata),
    }
}

fn validate_exif(exif: &exif::Exif, limits: MetadataLimits) -> Result<(), MetadataInputError> {
    let field_count =
        u32::try_from(exif.fields().len()).map_err(|_| MetadataInputError::ArithmeticOverflow)?;
    if field_count > limits.ifd_entries {
        return Err(MetadataInputError::IfdEntryLimit {
            limit: limits.ifd_entries,
        });
    }
    for field in exif.fields() {
        let value_bytes = value_bytes(&field.value)?;
        if value_bytes > limits.value_bytes {
            return Err(MetadataInputError::ValueTooLarge {
                limit: limits.value_bytes,
                actual: value_bytes,
            });
        }
        if u32::from(field.ifd_num.index()) >= limits.ifd_nesting {
            return Err(MetadataInputError::IfdNestingLimit {
                limit: limits.ifd_nesting,
            });
        }
    }
    Ok(())
}

fn canonical_entries(exif: &exif::Exif) -> Result<ImageMetadata, MetadataInputError> {
    let mut entries = Vec::new();
    text_entry(
        exif,
        Tag::Make,
        "camera make",
        MetadataEntry::CameraMake,
        &mut entries,
    )?;
    text_entry(
        exif,
        Tag::Model,
        "camera model",
        MetadataEntry::CameraModel,
        &mut entries,
    )?;
    text_entry(
        exif,
        Tag::LensModel,
        "lens model",
        MetadataEntry::LensModel,
        &mut entries,
    )?;
    text_entry(
        exif,
        Tag::DateTimeOriginal,
        "capture date time",
        MetadataEntry::CaptureDateTimeOriginal,
        &mut entries,
    )?;
    if let Some(field) = unique_field(exif, Tag::Orientation, "orientation")? {
        let code = field
            .value
            .get_uint(0)
            .and_then(|value| u8::try_from(value).ok())
            .ok_or(MetadataInputError::InvalidField {
                field: "orientation",
            })?;
        let orientation =
            Orientation::from_u8(code).map_err(|_| MetadataInputError::InvalidField {
                field: "orientation",
            })?;
        entries.push(MetadataEntry::Orientation(orientation));
    }
    rational_entry(
        exif,
        Tag::ExposureTime,
        "exposure time",
        MetadataEntry::ExposureTime,
        &mut entries,
    )?;
    rational_entry(
        exif,
        Tag::FNumber,
        "f-number",
        MetadataEntry::FNumber,
        &mut entries,
    )?;
    let iso = unique_field(exif, Tag::ISOSpeed, "ISO speed")?.or(unique_field(
        exif,
        Tag::PhotographicSensitivity,
        "ISO speed",
    )?);
    if let Some(field) = iso {
        let value = field
            .value
            .get_uint(0)
            .and_then(NonZeroU32::new)
            .ok_or(MetadataInputError::InvalidField { field: "ISO speed" })?;
        entries.push(MetadataEntry::IsoSpeed(value));
    }
    rational_entry(
        exif,
        Tag::FocalLength,
        "focal length",
        MetadataEntry::FocalLength,
        &mut entries,
    )?;
    ImageMetadata::from_entries(entries).map_err(|_| MetadataInputError::DuplicateField {
        field: "canonical metadata",
    })
}

#[allow(clippy::too_many_lines)]
fn canonical_entries_tolerant(
    exif: &exif::Exif,
) -> (Vec<MetadataEntry>, Option<MetadataInputError>) {
    let mut entries = Vec::new();
    let mut first_error = None;
    remember_error(
        &mut first_error,
        text_entry(
            exif,
            Tag::Make,
            "camera make",
            MetadataEntry::CameraMake,
            &mut entries,
        ),
    );
    remember_error(
        &mut first_error,
        text_entry(
            exif,
            Tag::Model,
            "camera model",
            MetadataEntry::CameraModel,
            &mut entries,
        ),
    );
    remember_error(
        &mut first_error,
        text_entry(
            exif,
            Tag::LensModel,
            "lens model",
            MetadataEntry::LensModel,
            &mut entries,
        ),
    );
    remember_error(
        &mut first_error,
        text_entry(
            exif,
            Tag::DateTimeOriginal,
            "capture date time",
            MetadataEntry::CaptureDateTimeOriginal,
            &mut entries,
        ),
    );
    match unique_field(exif, Tag::Orientation, "orientation") {
        Ok(Some(field)) => {
            let orientation = field
                .value
                .get_uint(0)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(MetadataInputError::InvalidField {
                    field: "orientation",
                })
                .and_then(|code| {
                    Orientation::from_u8(code).map_err(|_| MetadataInputError::InvalidField {
                        field: "orientation",
                    })
                });
            match orientation {
                Ok(orientation) => entries.push(MetadataEntry::Orientation(orientation)),
                Err(error) => remember_error(&mut first_error, Err(error)),
            }
        }
        Ok(None) => {}
        Err(error) => remember_error(&mut first_error, Err(error)),
    }
    remember_error(
        &mut first_error,
        rational_entry(
            exif,
            Tag::ExposureTime,
            "exposure time",
            MetadataEntry::ExposureTime,
            &mut entries,
        ),
    );
    remember_error(
        &mut first_error,
        rational_entry(
            exif,
            Tag::FNumber,
            "f-number",
            MetadataEntry::FNumber,
            &mut entries,
        ),
    );
    let iso = match unique_field(exif, Tag::ISOSpeed, "ISO speed") {
        Ok(Some(field)) => Some(field),
        Ok(None) => match unique_field(exif, Tag::PhotographicSensitivity, "ISO speed") {
            Ok(field) => field,
            Err(error) => {
                remember_error(&mut first_error, Err(error));
                None
            }
        },
        Err(error) => {
            remember_error(&mut first_error, Err(error));
            None
        }
    };
    if let Some(field) = iso {
        match field
            .value
            .get_uint(0)
            .and_then(NonZeroU32::new)
            .ok_or(MetadataInputError::InvalidField { field: "ISO speed" })
        {
            Ok(value) => entries.push(MetadataEntry::IsoSpeed(value)),
            Err(error) => remember_error(&mut first_error, Err(error)),
        }
    }
    remember_error(
        &mut first_error,
        rational_entry(
            exif,
            Tag::FocalLength,
            "focal length",
            MetadataEntry::FocalLength,
            &mut entries,
        ),
    );
    (entries, first_error)
}

fn remember_error(
    first_error: &mut Option<MetadataInputError>,
    result: Result<(), MetadataInputError>,
) {
    if let Err(error) = result
        && first_error.is_none()
    {
        *first_error = Some(error);
    }
}

fn preflight_tiff(payload: &[u8], limits: MetadataLimits) -> Result<(), MetadataInputError> {
    let little = match payload.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return Err(MetadataInputError::MalformedExif),
    };
    if read_u16(payload, 2, little)? != 42 {
        return Err(MetadataInputError::MalformedExif);
    }
    let first_ifd = usize::try_from(read_u32(payload, 4, little)?)
        .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
    let mut pending = vec![(first_ifd, 0u32)];
    let mut entries_seen = 0u32;
    while let Some((offset, depth)) = pending.pop() {
        if offset == 0 {
            continue;
        }
        if depth >= limits.ifd_nesting {
            return Err(MetadataInputError::IfdNestingLimit {
                limit: limits.ifd_nesting,
            });
        }
        let count = u32::from(read_u16(payload, offset, little)?);
        entries_seen = entries_seen
            .checked_add(count)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if entries_seen > limits.ifd_entries {
            return Err(MetadataInputError::IfdEntryLimit {
                limit: limits.ifd_entries,
            });
        }
        let entries_start = offset
            .checked_add(2)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let entries_bytes = usize::try_from(count)
            .map_err(|_| MetadataInputError::ArithmeticOverflow)?
            .checked_mul(12)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let next_offset = entries_start
            .checked_add(entries_bytes)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        read_u32(payload, next_offset, little)?;
        for index in 0..count {
            let entry_offset = entries_start
                .checked_add(
                    usize::try_from(index)
                        .map_err(|_| MetadataInputError::ArithmeticOverflow)?
                        .checked_mul(12)
                        .ok_or(MetadataInputError::ArithmeticOverflow)?,
                )
                .ok_or(MetadataInputError::ArithmeticOverflow)?;
            let tag = read_u16(payload, entry_offset, little)?;
            let kind = read_u16(payload, entry_offset + 2, little)?;
            let count = read_u32(payload, entry_offset + 4, little)?;
            let unit = type_size(kind).ok_or(MetadataInputError::MalformedExif)?;
            let value_bytes = u64::from(count)
                .checked_mul(u64::from(unit))
                .ok_or(MetadataInputError::ArithmeticOverflow)?;
            if value_bytes > limits.value_bytes {
                return Err(MetadataInputError::ValueTooLarge {
                    limit: limits.value_bytes,
                    actual: value_bytes,
                });
            }
            let value_offset = if value_bytes <= 4 {
                entry_offset + 8
            } else {
                usize::try_from(read_u32(payload, entry_offset + 8, little)?)
                    .map_err(|_| MetadataInputError::ArithmeticOverflow)?
            };
            let value_end = value_offset
                .checked_add(
                    usize::try_from(value_bytes)
                        .map_err(|_| MetadataInputError::ArithmeticOverflow)?,
                )
                .ok_or(MetadataInputError::ArithmeticOverflow)?;
            if value_end > payload.len() {
                return Err(MetadataInputError::MalformedExif);
            }
            if matches!(tag, 0x8769 | 0x8825 | 0xa005) && kind == 4 && count == 1 {
                let child = usize::try_from(read_u32(payload, value_offset, little)?)
                    .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
                pending.push((
                    child,
                    depth
                        .checked_add(1)
                        .ok_or(MetadataInputError::ArithmeticOverflow)?,
                ));
            }
        }
        let next = usize::try_from(read_u32(payload, next_offset, little)?)
            .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
        pending.push((next, depth));
    }
    Ok(())
}

fn type_size(kind: u16) -> Option<u32> {
    match kind {
        1 | 2 | 7 | 8 => Some(1),
        3 | 9 => Some(2),
        4 | 11 => Some(4),
        5 | 10 | 12 => Some(8),
        _ => None,
    }
}

fn read_u16(payload: &[u8], offset: usize, little: bool) -> Result<u16, MetadataInputError> {
    let bytes = payload
        .get(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or(MetadataInputError::ArithmeticOverflow)?,
        )
        .ok_or(MetadataInputError::MalformedExif)?;
    Ok(if little {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    })
}

fn read_u32(payload: &[u8], offset: usize, little: bool) -> Result<u32, MetadataInputError> {
    let bytes = payload
        .get(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or(MetadataInputError::ArithmeticOverflow)?,
        )
        .ok_or(MetadataInputError::MalformedExif)?;
    Ok(if little {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

fn text_entry<F>(
    exif: &exif::Exif,
    tag: Tag,
    name: &'static str,
    build: F,
    entries: &mut Vec<MetadataEntry>,
) -> Result<(), MetadataInputError>
where
    F: FnOnce(MetadataText) -> MetadataEntry,
{
    if let Some(field) = unique_field(exif, tag, name)? {
        let bytes = match &field.value {
            Value::Ascii(values) => values.first().cloned(),
            _ => None,
        };
        let bytes = bytes.ok_or(MetadataInputError::InvalidField { field: name })?;
        let value = MetadataText::from_bytes(bytes)
            .map_err(|_| MetadataInputError::InvalidField { field: name })?;
        entries.push(build(value));
    }
    Ok(())
}

fn rational_entry<F>(
    exif: &exif::Exif,
    tag: Tag,
    name: &'static str,
    build: F,
    entries: &mut Vec<MetadataEntry>,
) -> Result<(), MetadataInputError>
where
    F: FnOnce(PositiveRational) -> MetadataEntry,
{
    if let Some(field) = unique_field(exif, tag, name)? {
        let rational = match &field.value {
            Value::Rational(values) => values.first(),
            _ => None,
        };
        let rational = rational.ok_or(MetadataInputError::InvalidField { field: name })?;
        let value = PositiveRational::new(u64::from(rational.num), u64::from(rational.denom))
            .map_err(|_| MetadataInputError::InvalidField { field: name })?;
        entries.push(build(value));
    }
    Ok(())
}

fn unique_field<'a>(
    exif: &'a exif::Exif,
    tag: Tag,
    name: &'static str,
) -> Result<Option<&'a exif::Field>, MetadataInputError> {
    let mut matches = exif.fields().filter(|field| field.tag == tag);
    let first = matches.next();
    if matches.next().is_some() {
        return Err(MetadataInputError::DuplicateField { field: name });
    }
    Ok(first)
}

fn value_bytes(value: &Value) -> Result<u64, MetadataInputError> {
    let bytes = match value {
        Value::Byte(values) | Value::Undefined(values, _) => values.len(),
        Value::Ascii(values) => values
            .iter()
            .try_fold(0usize, |total, value| total.checked_add(value.len()))
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Short(values) => values
            .len()
            .checked_mul(2)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Long(values) => values
            .len()
            .checked_mul(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::SLong(values) => values
            .len()
            .checked_mul(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Rational(values) => values
            .len()
            .checked_mul(8)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::SRational(values) => values
            .len()
            .checked_mul(8)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::SByte(values) => values.len(),
        Value::SShort(values) => values
            .len()
            .checked_mul(2)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Float(values) => values
            .len()
            .checked_mul(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Double(values) => values
            .len()
            .checked_mul(8)
            .ok_or(MetadataInputError::ArithmeticOverflow)?,
        Value::Unknown(_, count, _) => {
            usize::try_from(*count).map_err(|_| MetadataInputError::ArithmeticOverflow)?
        }
    };
    u64::try_from(bytes).map_err(|_| MetadataInputError::ArithmeticOverflow)
}
