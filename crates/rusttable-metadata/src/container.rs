use rusttable_image::InputFormat;

use crate::error::MetadataInputError;
use crate::limits::MetadataLimits;

const WEBP_HEADER_BYTES: usize = 12;
const JPEG_XL_BARE_SIGNATURE: [u8; 2] = [0xff, 0x0a];
const JPEG_XL_CONTAINER_SIGNATURE: [u8; 12] =
    [0, 0, 0, 12, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];

pub(crate) fn exif_payload(
    format: InputFormat,
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    match format {
        InputFormat::Jpeg => jpeg_payload(source, limits),
        InputFormat::Png => png_payload(source, limits),
        InputFormat::Tiff => tiff_payload(source, limits),
        InputFormat::JpegXl => jpeg_xl_payload(source, limits),
        InputFormat::Webp => webp_payload(source, limits),
        InputFormat::OpenExr => Ok(None),
        // Camera-RAW containers do not share the classic TIFF byte signature at offset zero;
        // metadata is optional when the decoder has already accepted the RAW container.
        InputFormat::Raw => {
            if is_tiff(source) {
                tiff_payload(source, limits)
            } else {
                Ok(None)
            }
        }
    }
}

fn webp_payload(
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    if source.get(..4) != Some(b"RIFF") || source.get(8..WEBP_HEADER_BYTES) != Some(b"WEBP") {
        return Err(MetadataInputError::FormatMismatch {
            format: InputFormat::Webp,
        });
    }
    let riff_size = u64::from(read_u32_le(source, 4, InputFormat::Webp)?);
    if riff_size < 4 {
        return Err(malformed(
            InputFormat::Webp,
            "RIFF size does not include the WEBP form type",
        ));
    }
    let declared_bytes = riff_size
        .checked_add(8)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let actual_bytes =
        u64::try_from(source.len()).map_err(|_| MetadataInputError::ArithmeticOverflow)?;
    if declared_bytes != actual_bytes {
        return Err(malformed(
            InputFormat::Webp,
            if declared_bytes < actual_bytes {
                "trailing bytes follow the declared RIFF payload"
            } else {
                "RIFF payload is truncated"
            },
        ));
    }

    let mut cursor = WEBP_HEADER_BYTES;
    let mut found = None;
    while cursor < source.len() {
        let header_end = cursor
            .checked_add(8)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        let header = source
            .get(cursor..header_end)
            .ok_or_else(|| malformed(InputFormat::Webp, "truncated chunk header"))?;
        let length = usize::try_from(u32::from_le_bytes([
            header[4], header[5], header[6], header[7],
        ]))
        .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
        let payload_end = header_end
            .checked_add(length)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if payload_end > source.len() {
            return Err(malformed(InputFormat::Webp, "chunk exceeds source"));
        }
        let padded_end = payload_end
            .checked_add(length & 1)
            .ok_or(MetadataInputError::ArithmeticOverflow)?;
        if padded_end > source.len() {
            return Err(malformed(InputFormat::Webp, "chunk padding exceeds source"));
        }
        if length & 1 != 0 && source[payload_end] != 0 {
            return Err(malformed(
                InputFormat::Webp,
                "chunk padding byte is not zero",
            ));
        }

        if header.get(..4) == Some(b"EXIF") {
            if found.is_some() {
                return Err(MetadataInputError::DuplicateExifPayload {
                    format: InputFormat::Webp,
                });
            }
            let framed = &source[header_end..payload_end];
            let exif = framed.strip_prefix(b"Exif\0\0").unwrap_or(framed);
            check_payload(exif, limits)?;
            found = Some(exif.to_vec());
        }
        cursor = padded_end;
    }
    Ok(found)
}

fn jpeg_xl_payload(
    source: &[u8],
    limits: MetadataLimits,
) -> Result<Option<Vec<u8>>, MetadataInputError> {
    if source.starts_with(&JPEG_XL_BARE_SIGNATURE) {
        return Ok(None);
    }
    if !source.starts_with(&JPEG_XL_CONTAINER_SIGNATURE) {
        return Err(MetadataInputError::FormatMismatch {
            format: InputFormat::JpegXl,
        });
    }

    let mut cursor = JPEG_XL_CONTAINER_SIGNATURE.len();
    let mut found = None;
    while cursor < source.len() {
        let header = isobmff_box_header(source, cursor)?;
        if header.box_type == *b"Exif" {
            if found.is_some() {
                return Err(MetadataInputError::DuplicateExifPayload {
                    format: InputFormat::JpegXl,
                });
            }
            let payload = &source[header.payload_start..header.end];
            let tiff_header_offset = usize::try_from(read_u32(payload, 0, InputFormat::JpegXl)?)
                .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
            let exif_start = header
                .payload_start
                .checked_add(4)
                .and_then(|start| start.checked_add(tiff_header_offset))
                .ok_or(MetadataInputError::ArithmeticOverflow)?;
            if exif_start > header.end {
                return Err(malformed(
                    InputFormat::JpegXl,
                    "TIFF header offset exceeds Exif box",
                ));
            }
            let exif = &source[exif_start..header.end];
            check_payload(exif, limits)?;
            found = Some(exif.to_vec());
        }
        cursor = header.end;
    }
    Ok(found)
}

#[derive(Debug, Clone, Copy)]
struct IsobmffBoxHeader {
    box_type: [u8; 4],
    payload_start: usize,
    end: usize,
}

fn isobmff_box_header(
    source: &[u8],
    offset: usize,
) -> Result<IsobmffBoxHeader, MetadataInputError> {
    let prefix_end = offset
        .checked_add(8)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let prefix = source
        .get(offset..prefix_end)
        .ok_or_else(|| malformed(InputFormat::JpegXl, "truncated box header"))?;
    let size32 = u32::from_be_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);
    let box_type = [prefix[4], prefix[5], prefix[6], prefix[7]];
    let (header_bytes, total_bytes) = match size32 {
        0 => (
            8,
            source
                .len()
                .checked_sub(offset)
                .ok_or(MetadataInputError::ArithmeticOverflow)?,
        ),
        1 => {
            let size = usize::try_from(read_u64(source, prefix_end, InputFormat::JpegXl)?)
                .map_err(|_| MetadataInputError::ArithmeticOverflow)?;
            (16, size)
        }
        size => (
            8,
            usize::try_from(size).map_err(|_| MetadataInputError::ArithmeticOverflow)?,
        ),
    };
    if total_bytes < header_bytes {
        return Err(malformed(
            InputFormat::JpegXl,
            "box size is smaller than its header",
        ));
    }
    let payload_start = offset
        .checked_add(header_bytes)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let end = offset
        .checked_add(total_bytes)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    if end > source.len() {
        return Err(malformed(InputFormat::JpegXl, "box exceeds source"));
    }
    Ok(IsobmffBoxHeader {
        box_type,
        payload_start,
        end,
    })
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

fn malformed(format: InputFormat, reason: &'static str) -> MetadataInputError {
    MetadataInputError::MalformedContainer { format, reason }
}

fn read_u16(source: &[u8], offset: usize, format: InputFormat) -> Result<u16, MetadataInputError> {
    let end = offset
        .checked_add(2)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let bytes = source
        .get(offset..end)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
}

fn read_u32(source: &[u8], offset: usize, format: InputFormat) -> Result<u32, MetadataInputError> {
    let end = offset
        .checked_add(4)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let bytes = source
        .get(offset..end)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u32_le(
    source: &[u8],
    offset: usize,
    format: InputFormat,
) -> Result<u32, MetadataInputError> {
    let end = offset
        .checked_add(4)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let bytes = source
        .get(offset..end)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn read_u64(source: &[u8], offset: usize, format: InputFormat) -> Result<u64, MetadataInputError> {
    let end = offset
        .checked_add(8)
        .ok_or(MetadataInputError::ArithmeticOverflow)?;
    let bytes = source
        .get(offset..end)
        .ok_or(MetadataInputError::MalformedContainer {
            format,
            reason: "truncated integer",
        })?;
    Ok(u64::from_be_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ]))
}
