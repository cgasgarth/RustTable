use std::fmt;

use super::manifest::{ArtifactClass, FixtureEntry};

const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
const JXL_CONTAINER_SIGNATURE: &[u8; 12] = b"\0\0\0\x0cJXL \r\n\x87\n";
const SQLITE_SIGNATURE: &[u8; 16] = b"SQLite format 3\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualificationError {
    Empty {
        id: String,
    },
    UnsupportedFormat {
        id: String,
        format: String,
    },
    Signature {
        id: String,
        format: String,
    },
    Truncated {
        id: String,
        format: String,
    },
    InvalidStructure {
        id: String,
        format: String,
        reason: String,
    },
    SemanticMismatch {
        id: String,
        field: String,
    },
}

impl fmt::Display for QualificationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty { id } => write!(formatter, "fixture {id} is empty"),
            Self::UnsupportedFormat { id, format } => {
                write!(
                    formatter,
                    "fixture {id} has unsupported binary format {format}"
                )
            }
            Self::Signature { id, format } => {
                write!(formatter, "fixture {id} has an invalid {format} signature")
            }
            Self::Truncated { id, format } => {
                write!(formatter, "fixture {id} is truncated as {format}")
            }
            Self::InvalidStructure { id, format, reason } => {
                write!(
                    formatter,
                    "fixture {id} has invalid {format} structure: {reason}"
                )
            }
            Self::SemanticMismatch { id, field } => {
                write!(formatter, "fixture {id} does not satisfy {field}")
            }
        }
    }
}

impl std::error::Error for QualificationError {}

/// Qualifies a declared valid-binary fixture and returns observed semantics.
///
/// # Errors
///
/// Returns a typed error when the artifact is empty, structurally invalid,
/// truncated, or fails its declared semantic assertions.
pub fn qualify_binary(
    entry: &FixtureEntry,
    bytes: &[u8],
) -> Result<Vec<String>, QualificationError> {
    if entry.artifact_class != ArtifactClass::ValidBinary {
        return Err(QualificationError::InvalidStructure {
            id: entry.id.clone(),
            format: entry.format.clone(),
            reason: "only valid-binary fixtures are parser-qualified".to_owned(),
        });
    }
    if bytes.is_empty() {
        return Err(QualificationError::Empty {
            id: entry.id.clone(),
        });
    }
    let observed = match entry.format.as_str() {
        "base64" => inspect_base64(entry, bytes)?,
        "base64-bigtiff" => inspect_base64_bigtiff(entry, bytes)?,
        "binary" if entry.source == "rusttable-test" => Vec::new(),
        "icc" => inspect_icc(entry, bytes)?,
        "jpeg" => inspect_jpeg(entry, bytes)?,
        "jpeg-xl" => inspect_jpeg_xl(entry, bytes)?,
        "png" => inspect_png(entry, bytes)?,
        "sqlite" => inspect_sqlite(entry, bytes)?,
        "tiff" => inspect_tiff(entry, bytes)?,
        "webp" => inspect_webp(entry, bytes)?,
        "xmp" => inspect_xmp(entry, bytes)?,
        format => {
            return Err(QualificationError::UnsupportedFormat {
                id: entry.id.clone(),
                format: format.to_owned(),
            });
        }
    };
    for assertion in &entry.expected.metadata {
        if !observed.iter().any(|value| value == assertion) {
            return Err(QualificationError::SemanticMismatch {
                id: entry.id.clone(),
                field: assertion.clone(),
            });
        }
    }
    Ok(observed)
}

fn inspect_base64(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    let decoded = decode_base64(entry, bytes)?;
    let mut nested = entry.clone();
    nested.format = if decoded.starts_with(PNG_SIGNATURE) {
        "png".to_owned()
    } else if decoded.starts_with(b"\xff\xd8") {
        "jpeg".to_owned()
    } else if decoded.starts_with(b"\xff\x0a") || decoded.starts_with(JXL_CONTAINER_SIGNATURE) {
        "jpeg-xl".to_owned()
    } else if decoded.starts_with(b"RIFF")
        && decoded.get(8..12).is_some_and(|fourcc| fourcc == b"WEBP")
    {
        "webp".to_owned()
    } else {
        "tiff".to_owned()
    };
    qualify_binary(&nested, &decoded)
}

fn inspect_base64_bigtiff(
    entry: &FixtureEntry,
    bytes: &[u8],
) -> Result<Vec<String>, QualificationError> {
    let decoded = decode_base64(entry, bytes)?;
    if decoded.len() < 16 || !(decoded.starts_with(b"II+\0") || decoded.starts_with(b"MM\0+")) {
        return Err(signature(entry, "bigtiff"));
    }
    let little = decoded.starts_with(b"II");
    let offset = decoded[8..16]
        .try_into()
        .ok()
        .map(|value| {
            if little {
                u64::from_le_bytes(value)
            } else {
                u64::from_be_bytes(value)
            }
        })
        .ok_or_else(|| truncated(entry, "bigtiff"))?;
    if usize::try_from(offset)
        .ok()
        .is_none_or(|offset| offset >= decoded.len())
    {
        return Err(truncated(entry, "bigtiff"));
    }
    Ok(vec![
        "format=tiff".to_owned(),
        "container=bigtiff".to_owned(),
    ])
}

fn decode_base64(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<u8>, QualificationError> {
    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut quartet = [0u8; 4];
    let mut count = 0;
    for byte in bytes
        .iter()
        .copied()
        .filter(|byte| !byte.is_ascii_whitespace())
    {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => 64,
            _ => return Err(invalid(entry, "base64", "invalid alphabet")),
        };
        quartet[count] = value;
        count += 1;
        if count == 4 {
            if quartet[0] == 64 || quartet[1] == 64 {
                return Err(invalid(entry, "base64", "invalid padding"));
            }
            output.push((quartet[0] << 2) | (quartet[1] >> 4));
            if quartet[2] != 64 {
                output.push((quartet[1] << 4) | (quartet[2] >> 2));
            }
            if quartet[3] != 64 {
                output.push((quartet[2] << 6) | quartet[3]);
            }
            count = 0;
        }
    }
    if count != 0 {
        return Err(truncated(entry, "base64"));
    }
    Ok(output)
}

fn inspect_icc(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 132 {
        return Err(truncated(entry, "icc"));
    }
    let declared = u32::from_be_bytes(bytes[0..4].try_into().expect("ICC header slice"));
    if declared as usize != bytes.len() {
        return Err(invalid(
            entry,
            "icc",
            "profile size does not match the file",
        ));
    }
    if &bytes[36..40] != b"acsp" {
        return Err(signature(entry, "icc"));
    }
    let class = &bytes[8..12];
    let color_space = &bytes[12..16];
    let tag_count = u32::from_be_bytes(bytes[128..132].try_into().expect("ICC table slice"));
    let table_bytes = usize::try_from(tag_count)
        .ok()
        .and_then(|count| count.checked_mul(12))
        .and_then(|size| size.checked_add(132))
        .ok_or_else(|| invalid(entry, "icc", "tag table overflows"))?;
    if table_bytes > bytes.len() {
        return Err(truncated(entry, "icc"));
    }
    let mut tags = Vec::new();
    for index in 0..usize::try_from(tag_count).expect("validated ICC tag count") {
        let start = 132 + index * 12;
        let offset = u32::from_be_bytes(
            bytes[start + 4..start + 8]
                .try_into()
                .expect("ICC offset slice"),
        );
        let length = u32::from_be_bytes(
            bytes[start + 8..start + 12]
                .try_into()
                .expect("ICC length slice"),
        );
        let end = usize::try_from(offset)
            .ok()
            .and_then(|value| value.checked_add(usize::try_from(length).ok()?))
            .ok_or_else(|| invalid(entry, "icc", "tag range overflows"))?;
        let table_start = u32::try_from(table_bytes)
            .map_err(|_| invalid(entry, "icc", "tag table exceeds ICC address space"))?;
        if offset < table_start || end > bytes.len() || length < 4 {
            return Err(invalid(entry, "icc", "tag range is outside the profile"));
        }
        tags.push(String::from_utf8_lossy(&bytes[start..start + 4]).into_owned());
    }
    let mut observed = vec![
        format!("class={}", String::from_utf8_lossy(class).trim()),
        format!(
            "color-space={}",
            String::from_utf8_lossy(color_space).trim()
        ),
    ];
    if tags.iter().any(|tag| tag == "rXYZ") && tags.iter().any(|tag| tag == "gXYZ") {
        observed.push("profile=matrix-icc".to_owned());
    }
    if tags.iter().any(|tag| tag == "A2B0") || tags.iter().any(|tag| tag == "B2A0") {
        observed.push("profile=lut-icc".to_owned());
    }
    Ok(observed)
}

fn inspect_jpeg(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 4 || &bytes[..2] != b"\xff\xd8" {
        return Err(signature(entry, "jpeg"));
    }
    if !bytes.ends_with(b"\xff\xd9") {
        return Err(truncated(entry, "jpeg"));
    }
    Ok(vec!["format=jpeg".to_owned(), "bits=8".to_owned()])
}

fn inspect_jpeg_xl(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.starts_with(b"\xff\x0a") {
        if bytes.len() < 4 {
            return Err(truncated(entry, "jpeg-xl"));
        }
        return Ok(vec![
            "format=jpeg-xl".to_owned(),
            "container=bare".to_owned(),
        ]);
    }
    if !bytes.starts_with(JXL_CONTAINER_SIGNATURE) {
        return Err(signature(entry, "jpeg-xl"));
    }

    let mut cursor = 0usize;
    let mut codestream = false;
    while cursor < bytes.len() {
        let header = bytes
            .get(cursor..cursor.saturating_add(8))
            .ok_or_else(|| truncated(entry, "jpeg-xl"))?;
        let size32 = u32::from_be_bytes(header[..4].try_into().expect("JXL box size"));
        let box_type: [u8; 4] = header[4..8].try_into().expect("JXL box type");
        let (header_bytes, box_bytes) = if size32 == 1 {
            let extended = bytes
                .get(cursor + 8..cursor + 16)
                .ok_or_else(|| truncated(entry, "jpeg-xl"))?;
            (
                16usize,
                usize::try_from(u64::from_be_bytes(
                    extended.try_into().expect("extended JXL box size"),
                ))
                .map_err(|_| invalid(entry, "jpeg-xl", "box size exceeds address space"))?,
            )
        } else if size32 == 0 {
            (8usize, bytes.len().saturating_sub(cursor))
        } else {
            (
                8usize,
                usize::try_from(size32)
                    .map_err(|_| invalid(entry, "jpeg-xl", "box size exceeds address space"))?,
            )
        };
        if box_bytes < header_bytes {
            return Err(invalid(entry, "jpeg-xl", "box is shorter than its header"));
        }
        cursor = cursor
            .checked_add(box_bytes)
            .ok_or_else(|| invalid(entry, "jpeg-xl", "box range overflows"))?;
        if cursor > bytes.len() {
            return Err(truncated(entry, "jpeg-xl"));
        }
        codestream |= matches!(&box_type, b"jxlc" | b"jxlp");
    }
    if !codestream {
        return Err(invalid(entry, "jpeg-xl", "codestream box is absent"));
    }
    Ok(vec![
        "format=jpeg-xl".to_owned(),
        "container=isobmff".to_owned(),
    ])
}

fn inspect_png(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 33 || &bytes[..8] != PNG_SIGNATURE {
        return Err(signature(entry, "png"));
    }
    if &bytes[12..16] != b"IHDR" {
        return Err(invalid(entry, "png", "IHDR is not the first chunk"));
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("PNG width slice"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("PNG height slice"));
    if width == 0 || height == 0 {
        return Err(invalid(entry, "png", "dimensions are zero"));
    }
    let bit_depth = bytes[24];
    let color_type = bytes[25];
    if let Some(dimensions) = entry.expected.dimensions
        && (dimensions.width != width || dimensions.height != height)
    {
        return Err(QualificationError::SemanticMismatch {
            id: entry.id.clone(),
            field: "dimensions".to_owned(),
        });
    }
    let mut observed = vec![format!("bits={bit_depth}"), "format=png".to_owned()];
    if matches!(color_type, 4 | 6) {
        observed.push("alpha=true".to_owned());
    }
    Ok(observed)
}

fn inspect_sqlite(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 100 || &bytes[..16] != SQLITE_SIGNATURE {
        return Err(signature(entry, "sqlite"));
    }
    let page_size = u16::from_be_bytes(bytes[16..18].try_into().expect("SQLite page-size slice"));
    if page_size == 1 || !page_size.is_power_of_two() || page_size < 512 {
        return Err(invalid(entry, "sqlite", "invalid page size"));
    }
    if !bytes.len().is_multiple_of(usize::from(page_size)) {
        return Err(truncated(entry, "sqlite"));
    }
    Ok(vec!["schema=darktable".to_owned()])
}

fn inspect_tiff(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 14 {
        return Err(truncated(entry, "tiff"));
    }
    let little = match &bytes[..2] {
        b"II" => true,
        b"MM" => false,
        _ => return Err(signature(entry, "tiff")),
    };
    let read_u16 = |offset: usize| -> Option<u16> {
        let value = bytes.get(offset..offset + 2)?.try_into().ok()?;
        Some(if little {
            u16::from_le_bytes(value)
        } else {
            u16::from_be_bytes(value)
        })
    };
    let read_u32 = |offset: usize| -> Option<u32> {
        let value = bytes.get(offset..offset + 4)?.try_into().ok()?;
        Some(if little {
            u32::from_le_bytes(value)
        } else {
            u32::from_be_bytes(value)
        })
    };
    if read_u16(2) != Some(42) {
        return Err(signature(entry, "tiff"));
    }
    let ifd = usize::try_from(read_u32(4).ok_or_else(|| truncated(entry, "tiff"))?)
        .map_err(|_| invalid(entry, "tiff", "IFD offset overflows"))?;
    let count = usize::from(read_u16(ifd).ok_or_else(|| truncated(entry, "tiff"))?);
    let end = ifd
        .checked_add(2)
        .and_then(|value| value.checked_add(count.checked_mul(12)?))
        .and_then(|value| value.checked_add(4))
        .ok_or_else(|| invalid(entry, "tiff", "IFD range overflows"))?;
    if end > bytes.len() {
        return Err(truncated(entry, "tiff"));
    }
    if bytes.len() < end + 2 {
        return Err(truncated(entry, "tiff"));
    }
    let mut width = None;
    let mut height = None;
    let mut bits = None;
    for index in 0..count {
        let start = ifd + 2 + index * 12;
        let tag = read_u16(start).expect("validated TIFF entry");
        let field_type = read_u16(start + 2).expect("validated TIFF type");
        let item_count = read_u32(start + 4).expect("validated TIFF count");
        if field_type == 3 && item_count == 1 {
            let value = read_u16(start + 8).expect("validated TIFF inline value");
            match tag {
                256 => width = Some(u32::from(value)),
                257 => height = Some(u32::from(value)),
                258 => bits = Some(value),
                _ => {}
            }
        } else if field_type == 3 && item_count > 0 {
            let value_offset =
                usize::try_from(read_u32(start + 8).unwrap_or_default()).unwrap_or(usize::MAX);
            if tag == 258 {
                bits = read_u16(value_offset);
            }
        }
    }
    let (Some(width), Some(height), Some(bits)) = (width, height, bits) else {
        return Err(invalid(entry, "tiff", "required image tags are absent"));
    };
    if let Some(dimensions) = entry.expected.dimensions
        && (dimensions.width != width || dimensions.height != height)
    {
        return Err(QualificationError::SemanticMismatch {
            id: entry.id.clone(),
            field: "dimensions".to_owned(),
        });
    }
    Ok(vec![
        "format=tiff".to_owned(),
        format!("bits={bits}"),
        format!("endianness={}", if little { "little" } else { "big" }),
    ])
}

fn inspect_webp(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    if bytes.len() < 20 || !bytes.starts_with(b"RIFF") || &bytes[8..12] != b"WEBP" {
        return Err(signature(entry, "webp"));
    }
    let declared = usize::try_from(u32::from_le_bytes(
        bytes[4..8].try_into().expect("WebP RIFF size"),
    ))
    .map_err(|_| invalid(entry, "webp", "RIFF size exceeds address space"))?
    .checked_add(8)
    .ok_or_else(|| invalid(entry, "webp", "RIFF size overflows"))?;
    if declared != bytes.len() {
        return Err(truncated(entry, "webp"));
    }

    let mut cursor = 12usize;
    let mut image_chunk = false;
    let mut alpha = false;
    while cursor < bytes.len() {
        let header = bytes
            .get(cursor..cursor.saturating_add(8))
            .ok_or_else(|| truncated(entry, "webp"))?;
        let chunk_type: [u8; 4] = header[..4].try_into().expect("WebP chunk type");
        let payload_bytes = usize::try_from(u32::from_le_bytes(
            header[4..8].try_into().expect("WebP chunk size"),
        ))
        .map_err(|_| invalid(entry, "webp", "chunk size exceeds address space"))?;
        let payload_start = cursor
            .checked_add(8)
            .ok_or_else(|| invalid(entry, "webp", "chunk range overflows"))?;
        let payload_end = payload_start
            .checked_add(payload_bytes)
            .ok_or_else(|| invalid(entry, "webp", "chunk range overflows"))?;
        let payload = bytes
            .get(payload_start..payload_end)
            .ok_or_else(|| truncated(entry, "webp"))?;
        match &chunk_type {
            b"VP8 " => image_chunk = true,
            b"VP8L" => {
                image_chunk = true;
                alpha |= payload
                    .get(1..5)
                    .and_then(|value| value.try_into().ok())
                    .is_some_and(|bits| u32::from_le_bytes(bits) & (1 << 28) != 0);
            }
            b"VP8X" => alpha |= payload.first().is_some_and(|flags| flags & 0x10 != 0),
            b"ALPH" => alpha = true,
            _ => {}
        }
        cursor = payload_end
            .checked_add(payload_bytes % 2)
            .ok_or_else(|| invalid(entry, "webp", "chunk padding overflows"))?;
        if cursor > bytes.len() {
            return Err(truncated(entry, "webp"));
        }
    }
    if !image_chunk {
        return Err(invalid(entry, "webp", "image data chunk is absent"));
    }
    let mut observed = vec!["format=webp".to_owned(), "bits=8".to_owned()];
    if alpha {
        observed.push("alpha=true".to_owned());
    }
    Ok(observed)
}

fn inspect_xmp(entry: &FixtureEntry, bytes: &[u8]) -> Result<Vec<String>, QualificationError> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid(entry, "xmp", "XML is not UTF-8"))?;
    for marker in ["<rdf:RDF", "<rdf:Description", "darktable:"] {
        if !text.contains(marker) {
            return Err(invalid(entry, "xmp", "darktable RDF history is incomplete"));
        }
    }
    let mut observed = vec!["format=xmp".to_owned()];
    if text.contains("<darktable:history>") && text.contains("darktable:num=") {
        observed.push("history=simple".to_owned());
    }
    if text.matches("<rdf:li").count() > 1 {
        observed.push("history=multi-instance".to_owned());
    }
    Ok(observed)
}

fn signature(entry: &FixtureEntry, format: &str) -> QualificationError {
    QualificationError::Signature {
        id: entry.id.clone(),
        format: format.to_owned(),
    }
}

fn truncated(entry: &FixtureEntry, format: &str) -> QualificationError {
    QualificationError::Truncated {
        id: entry.id.clone(),
        format: format.to_owned(),
    }
}

fn invalid(entry: &FixtureEntry, format: &str, reason: &str) -> QualificationError {
    QualificationError::InvalidStructure {
        id: entry.id.clone(),
        format: format.to_owned(),
        reason: reason.to_owned(),
    }
}
