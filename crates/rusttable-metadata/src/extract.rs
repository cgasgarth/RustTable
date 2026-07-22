use std::{num::NonZeroU32, ops::Range};

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
            let value_bytes = checked_tiff_value_bytes(kind, u64::from(count))?;
            if value_bytes > limits.value_bytes {
                return Err(MetadataInputError::ValueTooLarge {
                    limit: limits.value_bytes,
                    actual: value_bytes,
                });
            }
            let _value = tiff_value_range(payload, entry_offset, kind, count, little)?;
            if matches!(tag, 0x8769 | 0x8825 | 0xa005) && kind == 4 && count == 1 {
                let child_offset = read_tiff_u32(payload, entry_offset, kind, count, little)?;
                let child = usize::try_from(child_offset)
                    .map_err(|_| tiff_offset_overflow(u64::from(child_offset), 1))?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TiffFieldType {
    code: u16,
    element_size: u8,
}

// TIFF 6.0 field widths. Keep this as the sole width authority for preflight
// and the typed Value accounting below. BigTIFF's 64-bit types are not part of
// the classic TIFF/EXIF contract implemented by RustTable.
const TIFF_FIELD_TYPES: [TiffFieldType; 13] = [
    TiffFieldType {
        code: 1,
        element_size: 1,
    }, // BYTE
    TiffFieldType {
        code: 2,
        element_size: 1,
    }, // ASCII
    TiffFieldType {
        code: 3,
        element_size: 2,
    }, // SHORT
    TiffFieldType {
        code: 4,
        element_size: 4,
    }, // LONG
    TiffFieldType {
        code: 5,
        element_size: 8,
    }, // RATIONAL
    TiffFieldType {
        code: 6,
        element_size: 1,
    }, // SBYTE
    TiffFieldType {
        code: 7,
        element_size: 1,
    }, // UNDEFINED
    TiffFieldType {
        code: 8,
        element_size: 2,
    }, // SSHORT
    TiffFieldType {
        code: 9,
        element_size: 4,
    }, // SLONG
    TiffFieldType {
        code: 10,
        element_size: 8,
    }, // SRATIONAL
    TiffFieldType {
        code: 11,
        element_size: 4,
    }, // FLOAT
    TiffFieldType {
        code: 12,
        element_size: 8,
    }, // DOUBLE
    TiffFieldType {
        code: 13,
        element_size: 4,
    }, // IFD
];

fn tiff_field_type(kind: u16) -> Option<TiffFieldType> {
    TIFF_FIELD_TYPES
        .iter()
        .copied()
        .find(|field_type| field_type.code == kind)
}

fn checked_tiff_value_bytes(kind: u16, count: u64) -> Result<u64, MetadataInputError> {
    let field_type = tiff_field_type(kind).ok_or(MetadataInputError::MalformedExif)?;
    count.checked_mul(u64::from(field_type.element_size)).ok_or(
        MetadataInputError::TiffValueSizeOverflow {
            kind,
            count,
            element_size: field_type.element_size,
        },
    )
}

fn tiff_value_range(
    payload: &[u8],
    entry_offset: usize,
    kind: u16,
    count: u32,
    little: bool,
) -> Result<Range<usize>, MetadataInputError> {
    let value_bytes = checked_tiff_value_bytes(kind, u64::from(count))?;
    let inline_slot = entry_offset
        .checked_add(8)
        .ok_or_else(|| tiff_offset_overflow(u64::try_from(entry_offset).unwrap_or(u64::MAX), 8))?;
    if value_bytes <= 4 {
        let _slot = read_fixed(payload, inline_slot, 4)?;
        return bounded_value_range(payload, inline_slot, value_bytes, kind, u64::from(count));
    }

    let value_offset = read_u32(payload, inline_slot, little)?;
    let value_offset = usize::try_from(value_offset)
        .map_err(|_| tiff_offset_overflow(u64::from(value_offset), value_bytes))?;
    bounded_value_range(payload, value_offset, value_bytes, kind, u64::from(count))
}

fn bounded_value_range(
    payload: &[u8],
    value_offset: usize,
    value_bytes: u64,
    kind: u16,
    count: u64,
) -> Result<Range<usize>, MetadataInputError> {
    let value_length = usize::try_from(value_bytes).map_err(|_| {
        tiff_offset_overflow(u64::try_from(value_offset).unwrap_or(u64::MAX), value_bytes)
    })?;
    let value_end = value_offset.checked_add(value_length).ok_or_else(|| {
        tiff_offset_overflow(u64::try_from(value_offset).unwrap_or(u64::MAX), value_bytes)
    })?;
    if value_end > payload.len() {
        return Err(MetadataInputError::TiffValueTruncated {
            kind,
            count,
            offset: u64::try_from(value_offset).unwrap_or(u64::MAX),
            required: value_bytes,
            available: u64::try_from(payload.len().saturating_sub(value_offset))
                .unwrap_or(u64::MAX),
        });
    }
    Ok(value_offset..value_end)
}

fn tiff_offset_overflow(offset: u64, length: u64) -> MetadataInputError {
    MetadataInputError::TiffOffsetOverflow { offset, length }
}

fn read_tiff_u32(
    payload: &[u8],
    entry_offset: usize,
    kind: u16,
    count: u32,
    little: bool,
) -> Result<u32, MetadataInputError> {
    let range = tiff_value_range(payload, entry_offset, kind, count, little)?;
    let bytes = payload
        .get(range)
        .ok_or(MetadataInputError::TiffValueTruncated {
            kind,
            count: u64::from(count),
            offset: u64::try_from(entry_offset).unwrap_or(u64::MAX),
            required: 4,
            available: 0,
        })?;
    Ok(if little {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

fn read_u16(payload: &[u8], offset: usize, little: bool) -> Result<u16, MetadataInputError> {
    let bytes = read_fixed(payload, offset, 2)?;
    Ok(if little {
        u16::from_le_bytes([bytes[0], bytes[1]])
    } else {
        u16::from_be_bytes([bytes[0], bytes[1]])
    })
}

fn read_u32(payload: &[u8], offset: usize, little: bool) -> Result<u32, MetadataInputError> {
    let bytes = read_fixed(payload, offset, 4)?;
    Ok(if little {
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    })
}

fn read_fixed(payload: &[u8], offset: usize, length: usize) -> Result<&[u8], MetadataInputError> {
    let end = offset.checked_add(length).ok_or_else(|| {
        tiff_offset_overflow(
            u64::try_from(offset).unwrap_or(u64::MAX),
            u64::try_from(length).unwrap_or(u64::MAX),
        )
    })?;
    payload
        .get(offset..end)
        .ok_or_else(|| MetadataInputError::TiffStructureTruncated {
            offset: u64::try_from(offset).unwrap_or(u64::MAX),
            required: u64::try_from(length).unwrap_or(u64::MAX),
            available: u64::try_from(payload.len().saturating_sub(offset)).unwrap_or(u64::MAX),
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
        Value::Byte(values) => typed_value_bytes(1, values.len())?,
        Value::Ascii(values) => {
            let count = values
                .iter()
                .try_fold(0usize, |total, value| total.checked_add(value.len()))
                .ok_or(MetadataInputError::ArithmeticOverflow)?;
            typed_value_bytes(2, count)?
        }
        Value::Undefined(values, _) => typed_value_bytes(7, values.len())?,
        Value::Short(values) => typed_value_bytes(3, values.len())?,
        Value::Long(values) => typed_value_bytes(4, values.len())?,
        Value::SLong(values) => typed_value_bytes(9, values.len())?,
        Value::Rational(values) => typed_value_bytes(5, values.len())?,
        Value::SRational(values) => typed_value_bytes(10, values.len())?,
        Value::SByte(values) => typed_value_bytes(6, values.len())?,
        Value::SShort(values) => typed_value_bytes(8, values.len())?,
        Value::Float(values) => typed_value_bytes(11, values.len())?,
        Value::Double(values) => typed_value_bytes(12, values.len())?,
        Value::Unknown(_, count, _) => u64::from(*count),
    };
    Ok(bytes)
}

fn typed_value_bytes(kind: u16, count: usize) -> Result<u64, MetadataInputError> {
    let count = u64::try_from(count).map_err(|_| MetadataInputError::ArithmeticOverflow)?;
    checked_tiff_value_bytes(kind, count)
}

#[cfg(test)]
mod tests {
    use super::{TIFF_FIELD_TYPES, checked_tiff_value_bytes, tiff_field_type};
    use crate::error::MetadataInputError;

    #[test]
    fn authoritative_table_matches_classic_tiff_widths() {
        let expected = [
            (1, 1),
            (2, 1),
            (3, 2),
            (4, 4),
            (5, 8),
            (6, 1),
            (7, 1),
            (8, 2),
            (9, 4),
            (10, 8),
            (11, 4),
            (12, 8),
            (13, 4),
        ];
        assert_eq!(TIFF_FIELD_TYPES.len(), expected.len());
        for (kind, width) in expected {
            assert_eq!(
                tiff_field_type(kind).map(|field| field.element_size),
                Some(width)
            );
        }
    }

    #[test]
    fn checked_width_arithmetic_preserves_count_for_representable_values() {
        for field_type in TIFF_FIELD_TYPES {
            for count in [0, 1, 2, 3, 7, 31, 255, u64::from(u32::MAX)] {
                let bytes = checked_tiff_value_bytes(field_type.code, count).unwrap();
                assert_eq!(bytes / u64::from(field_type.element_size), count);
                assert_eq!(bytes % u64::from(field_type.element_size), 0);
            }
        }
    }

    #[test]
    fn checked_width_arithmetic_rejects_u64_multiplication_overflow() {
        assert!(matches!(
            checked_tiff_value_bytes(12, u64::MAX),
            Err(MetadataInputError::TiffValueSizeOverflow {
                kind: 12,
                count: u64::MAX,
                element_size: 8,
            })
        ));
    }
}
