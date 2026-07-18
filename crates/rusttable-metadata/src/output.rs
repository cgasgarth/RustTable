use rusttable_core::{ImageMetadata, MetadataEntry, MetadataField, MetadataText, PositiveRational};

use crate::error::MetadataOutputError;
use crate::limits::MetadataOutputLimits;

const TAG_MAKE: u16 = 0x010f;
const TAG_MODEL: u16 = 0x0110;
const TAG_ORIENTATION: u16 = 0x0112;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_EXPOSURE_TIME: u16 = 0x829a;
const TAG_F_NUMBER: u16 = 0x829d;
const TAG_ISO_SPEED: u16 = 0x8833;
const TAG_DATE_TIME_ORIGINAL: u16 = 0x9003;
const TAG_FOCAL_LENGTH: u16 = 0x920a;
const TAG_LENS_MODEL: u16 = 0xa434;

pub trait MetadataOutput: Send + Sync {
    fn encode_exif(
        &self,
        metadata: &ImageMetadata,
    ) -> Result<Option<EncodedExif>, MetadataOutputError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedExif {
    bytes: Vec<u8>,
}

impl EncodedExif {
    fn new(bytes: Vec<u8>) -> Result<Self, MetadataOutputError> {
        if bytes.is_empty() {
            return Err(MetadataOutputError::InternalInvariant {
                context: "encoded EXIF payload must be nonempty",
            });
        }
        Ok(Self { bytes })
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalExifOutput {
    limits: MetadataOutputLimits,
}

impl CanonicalExifOutput {
    #[must_use]
    pub const fn new(limits: MetadataOutputLimits) -> Self {
        Self { limits }
    }

    /// Creates an encoder from explicit checked limits.
    ///
    /// # Errors
    ///
    /// Returns [`MetadataOutputError::InvalidLimits`] when any limit is zero,
    /// inconsistent, or cannot be represented by the encoder.
    pub const fn with_limits(
        max_payload_bytes: u64,
        max_ifd_entries: u32,
        max_value_bytes: u64,
        max_allocation_bytes: u64,
    ) -> Result<Self, MetadataOutputError> {
        match MetadataOutputLimits::new(
            max_payload_bytes,
            max_ifd_entries,
            max_value_bytes,
            max_allocation_bytes,
        ) {
            Ok(limits) => Ok(Self::new(limits)),
            Err(reason) => Err(MetadataOutputError::InvalidLimits { reason }),
        }
    }
}

impl MetadataOutput for CanonicalExifOutput {
    fn encode_exif(
        &self,
        metadata: &ImageMetadata,
    ) -> Result<Option<EncodedExif>, MetadataOutputError> {
        if metadata.is_empty() {
            return Ok(None);
        }

        let mut primary = [None; 4];
        primary[0] = text_entry(metadata, MetadataField::CameraMake, TAG_MAKE)?;
        primary[1] = text_entry(metadata, MetadataField::CameraModel, TAG_MODEL)?;
        primary[2] = orientation_entry(metadata)?;

        let mut exif = [None; 6];
        exif[0] = rational_entry(metadata, MetadataField::ExposureTime, TAG_EXPOSURE_TIME)?;
        exif[1] = rational_entry(metadata, MetadataField::FNumber, TAG_F_NUMBER)?;
        exif[2] = iso_entry(metadata)?;
        exif[3] = text_entry(
            metadata,
            MetadataField::CaptureDateTimeOriginal,
            TAG_DATE_TIME_ORIGINAL,
        )?;
        exif[4] = rational_entry(metadata, MetadataField::FocalLength, TAG_FOCAL_LENGTH)?;
        exif[5] = text_entry(metadata, MetadataField::LensModel, TAG_LENS_MODEL)?;

        let exif_count = count_entries(&exif)?;
        if exif_count != 0 {
            primary[3] = Some(Entry {
                tag: TAG_EXIF_IFD,
                kind: 4,
                value: Value::Long(0),
                offset: None,
            });
        }
        let primary_count = count_entries(&primary)?;
        let total_count = primary_count.checked_add(exif_count).ok_or(
            MetadataOutputError::ArithmeticOverflow {
                context: "IFD entry count",
            },
        )?;
        if total_count > self.limits.max_ifd_entries {
            return Err(MetadataOutputError::IfdEntryLimit {
                limit: self.limits.max_ifd_entries,
                actual: total_count,
            });
        }

        let mut cursor = 8u64;
        cursor = add(cursor, 2, "primary IFD count")?;
        cursor = add(
            cursor,
            u64::from(primary_count).checked_mul(12).ok_or(
                MetadataOutputError::ArithmeticOverflow {
                    context: "primary IFD entries",
                },
            )?,
            "primary IFD entries",
        )?;
        cursor = add(cursor, 4, "primary next IFD offset")?;
        assign_offsets(&mut primary, &mut cursor, self.limits)?;

        if exif_count != 0 {
            cursor = align_word(cursor)?;
            let exif_offset = tiff_offset(cursor)?;
            for entry in primary.iter_mut().flatten() {
                if entry.tag == TAG_EXIF_IFD {
                    entry.value = Value::Long(exif_offset);
                }
            }
            cursor = add(cursor, 2, "EXIF IFD count")?;
            cursor = add(
                cursor,
                u64::from(exif_count).checked_mul(12).ok_or(
                    MetadataOutputError::ArithmeticOverflow {
                        context: "EXIF IFD entries",
                    },
                )?,
                "EXIF IFD entries",
            )?;
            cursor = add(cursor, 4, "EXIF next IFD offset")?;
            assign_offsets(&mut exif, &mut cursor, self.limits)?;
        }

        let payload_len =
            usize::try_from(cursor).map_err(|_| MetadataOutputError::ArithmeticOverflow {
                context: "EXIF payload length",
            })?;
        let actual =
            u64::try_from(payload_len).map_err(|_| MetadataOutputError::ArithmeticOverflow {
                context: "EXIF payload length",
            })?;
        if actual > self.limits.max_payload_bytes {
            return Err(MetadataOutputError::PayloadLimit {
                limit: self.limits.max_payload_bytes,
                actual,
            });
        }
        if actual > self.limits.max_allocation_bytes {
            return Err(MetadataOutputError::AllocationLimit {
                limit: self.limits.max_allocation_bytes,
                actual,
            });
        }

        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(payload_len)
            .map_err(|_| MetadataOutputError::AllocationFailure { requested: actual })?;
        bytes.extend_from_slice(b"II*\0");
        put_u32(&mut bytes, 8);
        write_ifd(&mut bytes, &primary, 0)?;
        write_external_values(&mut bytes, &primary)?;
        if exif_count != 0 {
            let exif_offset = primary
                .iter()
                .flatten()
                .find(|entry| entry.tag == TAG_EXIF_IFD)
                .and_then(|entry| match entry.value {
                    Value::Long(offset) => Some(offset),
                    _ => None,
                })
                .ok_or(MetadataOutputError::InternalInvariant {
                    context: "EXIF pointer entry",
                })?;
            pad_to(&mut bytes, exif_offset)?;
            write_ifd(&mut bytes, &exif, 0)?;
            write_external_values(&mut bytes, &exif)?;
        }
        if bytes.len() != payload_len {
            return Err(MetadataOutputError::InternalInvariant {
                context: "EXIF payload layout",
            });
        }
        EncodedExif::new(bytes).map(Some)
    }
}

#[derive(Debug, Clone, Copy)]
struct Entry<'a> {
    tag: u16,
    kind: u16,
    value: Value<'a>,
    offset: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
enum Value<'a> {
    Ascii { text: &'a MetadataText },
    Short(u16),
    Long(u32),
    Rational { value: PositiveRational },
}

fn text_entry<'a>(
    metadata: &'a ImageMetadata,
    field: MetadataField,
    tag: u16,
) -> Result<Option<Entry<'a>>, MetadataOutputError> {
    let Some(entry) = metadata.get(field) else {
        return Ok(None);
    };
    let (MetadataEntry::CameraMake(text)
    | MetadataEntry::CameraModel(text)
    | MetadataEntry::LensModel(text)
    | MetadataEntry::CaptureDateTimeOriginal(text)) = entry
    else {
        return Err(MetadataOutputError::InternalInvariant {
            context: "text metadata field variant",
        });
    };
    if !text.as_str().is_ascii() {
        return Err(MetadataOutputError::UnrepresentableText { field });
    }
    Ok(Some(Entry {
        tag,
        kind: 2,
        value: Value::Ascii { text },
        offset: None,
    }))
}

fn orientation_entry<'a>(
    metadata: &'a ImageMetadata,
) -> Result<Option<Entry<'a>>, MetadataOutputError> {
    let Some(entry) = metadata.get(MetadataField::Orientation) else {
        return Ok(None);
    };
    let MetadataEntry::Orientation(value) = entry else {
        return Err(MetadataOutputError::InternalInvariant {
            context: "orientation metadata field variant",
        });
    };
    Ok(Some(Entry {
        tag: TAG_ORIENTATION,
        kind: 3,
        value: Value::Short(u16::from(value.code())),
        offset: None,
    }))
}

fn rational_entry<'a>(
    metadata: &'a ImageMetadata,
    field: MetadataField,
    tag: u16,
) -> Result<Option<Entry<'a>>, MetadataOutputError> {
    let Some(entry) = metadata.get(field) else {
        return Ok(None);
    };
    let value = match entry {
        MetadataEntry::ExposureTime(value)
        | MetadataEntry::FNumber(value)
        | MetadataEntry::FocalLength(value) => *value,
        _ => {
            return Err(MetadataOutputError::InternalInvariant {
                context: "rational metadata field variant",
            });
        }
    };
    if value.numerator() > u64::from(u32::MAX) || value.denominator() > u64::from(u32::MAX) {
        return Err(MetadataOutputError::UnrepresentableRational {
            field,
            numerator: value.numerator(),
            denominator: value.denominator(),
        });
    }
    Ok(Some(Entry {
        tag,
        kind: 5,
        value: Value::Rational { value },
        offset: None,
    }))
}

fn iso_entry<'a>(metadata: &'a ImageMetadata) -> Result<Option<Entry<'a>>, MetadataOutputError> {
    let Some(entry) = metadata.get(MetadataField::IsoSpeed) else {
        return Ok(None);
    };
    let MetadataEntry::IsoSpeed(value) = entry else {
        return Err(MetadataOutputError::InternalInvariant {
            context: "ISO metadata field variant",
        });
    };
    Ok(Some(Entry {
        tag: TAG_ISO_SPEED,
        kind: 4,
        value: Value::Long(value.get()),
        offset: None,
    }))
}

fn count_entries<T>(entries: &[Option<T>]) -> Result<u32, MetadataOutputError> {
    u32::try_from(entries.iter().flatten().count()).map_err(|_| {
        MetadataOutputError::ArithmeticOverflow {
            context: "IFD entry count",
        }
    })
}

fn assign_offsets<'a>(
    entries: &mut [Option<Entry<'a>>],
    cursor: &mut u64,
    limits: MetadataOutputLimits,
) -> Result<(), MetadataOutputError> {
    for entry in entries.iter_mut().flatten() {
        check_value_limit(entry, limits)?;
        if entry.value.encoded_len()? > 4 {
            *cursor = align_word(*cursor)?;
            entry.offset = Some(tiff_offset(*cursor)?);
            *cursor = add(*cursor, entry.value.encoded_len()?, "EXIF value")?;
        }
    }
    Ok(())
}

fn check_value_limit(
    entry: &Entry<'_>,
    limits: MetadataOutputLimits,
) -> Result<(), MetadataOutputError> {
    let actual = entry.value.encoded_len()?;
    if actual > limits.max_value_bytes {
        let Some(field) = entry.field() else {
            return Err(MetadataOutputError::InternalInvariant {
                context: "unattributed EXIF value",
            });
        };
        return Err(MetadataOutputError::ValueLimit {
            field,
            limit: limits.max_value_bytes,
            actual,
        });
    }
    Ok(())
}

impl Value<'_> {
    fn encoded_len(self) -> Result<u64, MetadataOutputError> {
        match self {
            Self::Ascii { text, .. } => u64::try_from(text.byte_len())
                .map_err(|_| MetadataOutputError::ArithmeticOverflow {
                    context: "ASCII value length",
                })?
                .checked_add(1)
                .ok_or(MetadataOutputError::ArithmeticOverflow {
                    context: "ASCII NUL terminator",
                }),
            Self::Short(_) => Ok(2),
            Self::Long(_) => Ok(4),
            Self::Rational { .. } => Ok(8),
        }
    }
}

fn write_ifd(
    bytes: &mut Vec<u8>,
    entries: &[Option<Entry<'_>>],
    next_offset: u32,
) -> Result<(), MetadataOutputError> {
    let count = count_entries(entries)?;
    let count = u16::try_from(count).map_err(|_| MetadataOutputError::ArithmeticOverflow {
        context: "IFD entry count",
    })?;
    put_u16(bytes, count);
    for entry in entries.iter().flatten() {
        put_u16(bytes, entry.tag);
        put_u16(bytes, entry.kind);
        put_u32(bytes, entry.count()?);
        write_entry_value(bytes, entry)?;
    }
    put_u32(bytes, next_offset);
    Ok(())
}

fn write_entry_value(bytes: &mut Vec<u8>, entry: &Entry<'_>) -> Result<(), MetadataOutputError> {
    if entry.value.encoded_len()? <= 4 {
        match entry.value {
            Value::Ascii { text, .. } => {
                bytes.extend_from_slice(text.as_str().as_bytes());
                bytes.push(0);
                for _ in (text.byte_len() + 1)..4 {
                    bytes.push(0);
                }
            }
            Value::Short(value) => {
                put_u16(bytes, value);
                put_u16(bytes, 0);
            }
            Value::Long(value) => put_u32(bytes, value),
            Value::Rational { .. } => {
                return Err(MetadataOutputError::InternalInvariant {
                    context: "inline rational value",
                });
            }
        }
    } else {
        let offset = entry.offset.ok_or(MetadataOutputError::InternalInvariant {
            context: "missing EXIF value offset",
        })?;
        put_u32(bytes, offset);
    }
    Ok(())
}

fn write_external_values(
    bytes: &mut Vec<u8>,
    entries: &[Option<Entry<'_>>],
) -> Result<(), MetadataOutputError> {
    for entry in entries.iter().flatten() {
        if entry.value.encoded_len()? <= 4 {
            continue;
        }
        let offset = entry.offset.ok_or(MetadataOutputError::InternalInvariant {
            context: "missing EXIF value offset",
        })?;
        pad_to(bytes, offset)?;
        match entry.value {
            Value::Ascii { text, .. } => {
                bytes.extend_from_slice(text.as_str().as_bytes());
                bytes.push(0);
            }
            Value::Rational { value, .. } => {
                let field = entry
                    .field()
                    .ok_or(MetadataOutputError::InternalInvariant {
                        context: "unattributed rational value",
                    })?;
                let numerator = u32::try_from(value.numerator()).map_err(|_| {
                    MetadataOutputError::UnrepresentableRational {
                        field,
                        numerator: value.numerator(),
                        denominator: value.denominator(),
                    }
                })?;
                let denominator = u32::try_from(value.denominator()).map_err(|_| {
                    MetadataOutputError::UnrepresentableRational {
                        field,
                        numerator: value.numerator(),
                        denominator: value.denominator(),
                    }
                })?;
                put_u32(bytes, numerator);
                put_u32(bytes, denominator);
            }
            Value::Short(_) | Value::Long(_) => {
                return Err(MetadataOutputError::InternalInvariant {
                    context: "inline EXIF value marked external",
                });
            }
        }
    }
    Ok(())
}

fn pad_to(bytes: &mut Vec<u8>, target: u32) -> Result<(), MetadataOutputError> {
    let target = usize::try_from(target).map_err(|_| MetadataOutputError::ArithmeticOverflow {
        context: "EXIF value offset",
    })?;
    if bytes.len() > target {
        return Err(MetadataOutputError::InternalInvariant {
            context: "EXIF value offset order",
        });
    }
    bytes.resize(target, 0);
    Ok(())
}

fn put_u16(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn add(left: u64, right: u64, context: &'static str) -> Result<u64, MetadataOutputError> {
    left.checked_add(right)
        .ok_or(MetadataOutputError::ArithmeticOverflow { context })
}

fn align_word(value: u64) -> Result<u64, MetadataOutputError> {
    value
        .checked_add(value & 1)
        .ok_or(MetadataOutputError::ArithmeticOverflow {
            context: "EXIF word alignment",
        })
}

fn tiff_offset(value: u64) -> Result<u32, MetadataOutputError> {
    u32::try_from(value).map_err(|_| MetadataOutputError::ArithmeticOverflow {
        context: "classic TIFF offset",
    })
}

impl Entry<'_> {
    fn count(self) -> Result<u32, MetadataOutputError> {
        match self.value {
            Value::Ascii { text, .. } => u32::try_from(text.byte_len())
                .map_err(|_| MetadataOutputError::ArithmeticOverflow {
                    context: "ASCII element count",
                })?
                .checked_add(1)
                .ok_or(MetadataOutputError::ArithmeticOverflow {
                    context: "ASCII element count",
                }),
            Value::Short(_) | Value::Long(_) | Value::Rational { .. } => Ok(1),
        }
    }

    fn field(self) -> Option<MetadataField> {
        match self.tag {
            TAG_MAKE => Some(MetadataField::CameraMake),
            TAG_MODEL => Some(MetadataField::CameraModel),
            TAG_ORIENTATION => Some(MetadataField::Orientation),
            TAG_EXPOSURE_TIME => Some(MetadataField::ExposureTime),
            TAG_F_NUMBER => Some(MetadataField::FNumber),
            TAG_ISO_SPEED => Some(MetadataField::IsoSpeed),
            TAG_DATE_TIME_ORIGINAL => Some(MetadataField::CaptureDateTimeOriginal),
            TAG_FOCAL_LENGTH => Some(MetadataField::FocalLength),
            TAG_LENS_MODEL => Some(MetadataField::LensModel),
            TAG_EXIF_IFD => None,
            _ => None,
        }
    }
}
