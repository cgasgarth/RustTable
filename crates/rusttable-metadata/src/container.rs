use rusttable_image::InputFormat;

use crate::error::MetadataInputError;
use crate::limits::MetadataLimits;

pub(crate) fn exif_payload(
    format: InputFormat,
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    match format {
        InputFormat::Jpeg => jpeg_payload(source, limits),
        InputFormat::Png => png_payload(source, limits),
        InputFormat::Tiff => tiff_payload(source, limits),
    }
}

fn jpeg_payload(
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    if !source.starts_with(&[0xff, 0xd8]) {
        return Err(MetadataInputError::FormatMismatch {
            format: InputFormat::Jpeg,
        });
    }
    let mut cursor = 2usize;
    let mut segments = 0u32;
    let mut found = None;
    while cursor < source.len() {
        if source[cursor] != 0xff {
            return Err(MetadataInputError::MalformedContainer {
                format: InputFormat::Jpeg,
                reason: "marker prefix missing",
            });
        }
        while cursor < source.len() && source[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *source
            .get(cursor)
            .ok_or(MetadataInputError::MalformedContainer {
                format: InputFormat::Jpeg,
                reason: "truncated marker",
            })?;
        cursor += 1;
        if marker == 0xd9 || marker == 0xda {
            break;
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        segments = segments
            .checked_add(1)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if segments > limits.jpeg_segments {
            return Err(MetadataInputError::JpegSegmentLimit {
                limit: limits.jpeg_segments,
            });
        }
        let length = usize::from(read_u16(source, cursor, InputFormat::Jpeg)?);
        if length < 2 {
            return Err(MetadataInputError::MalformedContainer {
                format: InputFormat::Jpeg,
                reason: "invalid segment length",
            });
        }
        let end = cursor
            .checked_add(length)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if end > source.len() {
            return Err(MetadataInputError::MalformedContainer {
                format: InputFormat::Jpeg,
                reason: "segment exceeds source",
            });
        }
        let payload = &source[cursor + 2..end];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            if found.is_some() {
                return Err(MetadataInputError::DuplicateExifPayload {
                    format: InputFormat::Jpeg,
                });
            }
            let exif = &payload[6..];
            check_payload(exif, limits)?;
            found = Some(exif.to_vec());
        }
        cursor = end;
    }
    Ok(found)
}

fn png_payload(
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !source.starts_with(SIGNATURE) {
        return Err(MetadataInputError::FormatMismatch {
            format: InputFormat::Png,
        });
    }
    let mut cursor = 8usize;
    let mut chunks = 0u32;
    let mut found = None;
    loop {
        let length = usize::try_from(read_u32(source, cursor, InputFormat::Png)?)
            .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
        let kind_start = cursor
            .checked_add(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let data_start = kind_start
            .checked_add(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let data_end = data_start
            .checked_add(length)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let end = data_end
            .checked_add(4)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if end > source.len() {
            return Err(MetadataInputError::MalformedContainer {
                format: InputFormat::Png,
                reason: "chunk exceeds source",
            });
        }
        chunks = chunks
            .checked_add(1)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if chunks > limits.png_chunks {
            return Err(MetadataInputError::PngChunkLimit {
                limit: limits.png_chunks,
            });
        }
        let kind = &source[kind_start..data_start];
        if kind == b"eXIf" {
            if found.is_some() {
                return Err(MetadataInputError::DuplicateExifPayload {
                    format: InputFormat::Png,
                });
            }
            let exif = &source[data_start..data_end];
            check_payload(exif, limits)?;
            found = Some(exif.to_vec());
        }
        cursor = end;
        if kind == b"IEND" {
            return Ok(found);
        }
    }
}

fn tiff_payload(
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    if !is_tiff(source) {
        return Err(MetadataInputError::FormatMismatch {
            format: InputFormat::Tiff,
        });
    }
    check_payload(source, limits)?;
    Ok(Some(source.to_vec()))
}

fn is_tiff(source: &[u8]) -> bool {
    source.len() >= 4 && (source.starts_with(b"II*\0") || source.starts_with(b"MM\0*"))
}

fn check_payload(payload: &[u8], limits: MetadataLimits) -> Result<(), MetadataInputError> {
    let actual =
        u64::try_from(payload.len()).map_err(|_| MetadataInputError::ArithmeticOverflow)?;
    if actual > limits.exif_bytes {
        return Err(MetadataInputError::ExifPayloadTooLarge {
            limit: limits.exif_bytes,
            actual,
        });
    }
    Ok(())
}

fn read_u16(source: &[u8], offset: usize, format: InputFormat) -> Result<u16, MetadataInputError> {
    let bytes = source
        .get(offset..offset + 2)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(source: &[u8], offset: usize, format: InputFormat) -> Result<u32, MetadataInputError> {
    let bytes = source
        .get(offset..offset + 4)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}
