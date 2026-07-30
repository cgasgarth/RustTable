use std::collections::BTreeSet;

use rusttable_image::ImageDimensions;
use sha2::{Digest, Sha256};

use super::types::{
    WebPChunk, WebPChunkInventory, WebPCodingMode, WebPContainer, WebPDataLocation,
    WebPDecodeError, WebPDecodeLimits, WebPFeatures, WebPHeader, WebPMetadataChunk,
    WebPMetadataInventory,
};

const RIFF_HEADER_BYTES: usize = 12;
const CHUNK_HEADER_BYTES: usize = 8;
const VP8X_BYTES: usize = 10;
const VP8_MINIMUM_BYTES: usize = 10;
const VP8L_MINIMUM_BYTES: usize = 5;
const VP8X_VALID_FLAGS: u8 = 0b0011_1110;

#[derive(Debug, Clone)]
pub(super) struct ParsedWebP {
    pub header: WebPHeader,
}

#[derive(Debug, Clone, Copy)]
struct ImageInfo {
    coding: WebPCodingMode,
    dimensions: ImageDimensions,
    alpha: bool,
    location: WebPDataLocation,
}

#[must_use]
pub(crate) fn is_webp_signature(bytes: &[u8]) -> bool {
    bytes.len() >= RIFF_HEADER_BYTES
        && bytes.get(..4) == Some(b"RIFF")
        && bytes.get(8..12) == Some(b"WEBP")
}

/// Validates a complete RIFF/WebP source without decoding pixels.
///
/// # Errors
///
/// Returns a typed malformed, unsupported-animation, or resource-limit failure.
pub fn probe(bytes: &[u8], limits: WebPDecodeLimits) -> Result<WebPHeader, WebPDecodeError> {
    parse(bytes, limits).map(|parsed| parsed.header)
}

/// Fully inventories and validates a complete RIFF/WebP source.
///
/// # Errors
///
/// Returns a typed malformed, unsupported-animation, or resource-limit failure.
pub fn inspect(bytes: &[u8], limits: WebPDecodeLimits) -> Result<WebPHeader, WebPDecodeError> {
    probe(bytes, limits)
}

pub(super) fn parse(bytes: &[u8], limits: WebPDecodeLimits) -> Result<ParsedWebP, WebPDecodeError> {
    enforce_source(bytes, limits)?;
    if bytes.len() < RIFF_HEADER_BYTES {
        return Err(malformed("truncated RIFF/WEBP header"));
    }
    if bytes.get(..4) != Some(b"RIFF") {
        return Err(malformed("missing RIFF signature"));
    }
    if bytes.get(8..12) != Some(b"WEBP") {
        return Err(malformed("missing WEBP form type"));
    }
    let riff_size = u64::from(le_u32(&bytes[4..8]));
    let declared_bytes = riff_size
        .checked_add(8)
        .ok_or_else(|| malformed("RIFF size overflows"))?;
    if riff_size < 4 {
        return Err(malformed("RIFF size does not include the WEBP form type"));
    }
    let actual_bytes =
        u64::try_from(bytes.len()).map_err(|_| malformed("source length is not representable"))?;
    if declared_bytes != actual_bytes {
        return Err(malformed(if declared_bytes < actual_bytes {
            "trailing bytes follow the declared RIFF payload"
        } else {
            "RIFF payload is truncated"
        }));
    }
    if declared_bytes > limits.max_source_bytes {
        return Err(limit(
            "declared source bytes",
            declared_bytes,
            limits.max_source_bytes,
        ));
    }

    let chunks = read_chunks(bytes, limits)?;
    let first = chunks
        .first()
        .ok_or_else(|| malformed("WEBP contains no image chunk"))?;
    let (container, flags, dimensions) = match first.kind {
        kind if kind == *b"VP8 " || kind == *b"VP8L" => (WebPContainer::Simple, None, None),
        kind if kind == *b"VP8X" => {
            let (flags, dimensions) = parse_vp8x(bytes, first, limits)?;
            (WebPContainer::Extended, Some(flags), Some(dimensions))
        }
        _ => {
            return Err(malformed("first WebP chunk must be VP8, VP8L, or VP8X"));
        }
    };

    let (image, features, metadata, alpha_data) = match container {
        WebPContainer::Simple => parse_simple(bytes, &chunks, limits)?,
        WebPContainer::Extended => parse_extended(
            bytes,
            &chunks,
            flags.expect("extended flags"),
            dimensions.expect("extended dimensions"),
            limits,
        )?,
    };
    let compressed_bytes = image
        .location
        .length
        .checked_add(alpha_data.map_or(0, |location| location.length))
        .ok_or_else(|| malformed("compressed byte count overflows"))?;
    let header = WebPHeader {
        dimensions: image.dimensions,
        container,
        coding: image.coding,
        features,
        vp8x_flags: flags,
        riff_declared_bytes: declared_bytes,
        chunks: WebPChunkInventory {
            chunks,
            image_data: image.location,
            alpha_data,
            compressed_bytes,
        },
        metadata,
    };
    Ok(ParsedWebP { header })
}

fn read_chunks(bytes: &[u8], limits: WebPDecodeLimits) -> Result<Vec<WebPChunk>, WebPDecodeError> {
    let mut chunks = Vec::new();
    let mut cursor = RIFF_HEADER_BYTES;
    while cursor < bytes.len() {
        if chunks.len()
            >= usize::try_from(limits.max_chunks)
                .map_err(|_| malformed("chunk count limit is not representable"))?
        {
            return Err(limit(
                "chunk count",
                u64::try_from(chunks.len() + 1).unwrap_or(u64::MAX),
                u64::from(limits.max_chunks),
            ));
        }
        let header_end = cursor
            .checked_add(CHUNK_HEADER_BYTES)
            .ok_or_else(|| malformed("chunk header offset overflows"))?;
        let header = bytes
            .get(cursor..header_end)
            .ok_or_else(|| malformed("truncated WebP chunk header"))?;
        let kind = header[..4].try_into().expect("four-byte chunk kind");
        let length = u64::from(le_u32(&header[4..8]));
        if length > limits.max_chunk_bytes {
            return Err(limit("chunk bytes", length, limits.max_chunk_bytes));
        }
        let payload_start = header_end;
        let payload_start_u64 =
            u64::try_from(payload_start).map_err(|_| malformed("chunk offset is too large"))?;
        let payload_end_u64 = payload_start_u64
            .checked_add(length)
            .ok_or_else(|| malformed("chunk extent overflows"))?;
        let payload_end = usize::try_from(payload_end_u64)
            .map_err(|_| malformed("chunk extent is not representable"))?;
        bytes
            .get(payload_start..payload_end)
            .ok_or_else(|| malformed("truncated WebP chunk payload"))?;
        let padded_length = length
            .checked_add(length & 1)
            .ok_or_else(|| malformed("padded chunk length overflows"))?;
        let next_u64 = payload_start_u64
            .checked_add(padded_length)
            .ok_or_else(|| malformed("padded chunk extent overflows"))?;
        let next = usize::try_from(next_u64)
            .map_err(|_| malformed("padded chunk extent is not representable"))?;
        if length & 1 != 0 {
            let pad = bytes
                .get(payload_end)
                .ok_or_else(|| malformed("missing odd-length WebP padding byte"))?;
            if *pad != 0 {
                return Err(malformed("WebP padding byte must be zero"));
            }
        }
        if next > bytes.len() {
            return Err(malformed("WebP chunk padding exceeds RIFF payload"));
        }
        chunks
            .try_reserve(1)
            .map_err(|_| WebPDecodeError::AllocationFailure)?;
        chunks.push(WebPChunk {
            kind,
            location: WebPDataLocation {
                offset: payload_start_u64,
                length,
            },
            padded_length,
        });
        cursor = next;
    }
    Ok(chunks)
}

fn parse_simple(
    bytes: &[u8],
    chunks: &[WebPChunk],
    limits: WebPDecodeLimits,
) -> Result<
    (
        ImageInfo,
        WebPFeatures,
        WebPMetadataInventory,
        Option<WebPDataLocation>,
    ),
    WebPDecodeError,
> {
    if chunks.len() != 1 {
        return Err(malformed(
            "simple WebP must contain exactly one VP8 or VP8L chunk",
        ));
    }
    let image = parse_image_chunk(bytes, &chunks[0], limits)?;
    Ok((
        image,
        WebPFeatures {
            alpha: image.alpha,
            ..WebPFeatures::default()
        },
        WebPMetadataInventory::default(),
        None,
    ))
}

fn parse_extended(
    bytes: &[u8],
    chunks: &[WebPChunk],
    flags: u8,
    canvas: ImageDimensions,
    limits: WebPDecodeLimits,
) -> Result<
    (
        ImageInfo,
        WebPFeatures,
        WebPMetadataInventory,
        Option<WebPDataLocation>,
    ),
    WebPDecodeError,
> {
    let declared = features_from_flags(flags);
    let mut seen = BTreeSet::new();
    let mut metadata = WebPMetadataInventory::default();
    let mut image = None;
    let mut alpha = None;
    let mut rank = 0_u8;
    let mut animation_chunk = false;
    let mut metadata_bytes = 0_u64;

    for (index, chunk) in chunks.iter().enumerate().skip(1) {
        if is_singleton(chunk.kind) && !seen.insert(chunk.kind) {
            return Err(malformed(&format!(
                "duplicate {} chunk",
                fourcc(chunk.kind)
            )));
        }
        match chunk.kind {
            kind if kind == *b"VP8X" => return Err(malformed("duplicate VP8X chunk")),
            kind if kind == *b"ANIM" || kind == *b"ANMF" => animation_chunk = true,
            kind if kind == *b"ICCP" => {
                advance_rank(&mut rank, 1, "ICCP")?;
                metadata_bytes = add_metadata(metadata_bytes, chunk, limits)?;
                metadata.icc_profile = Some(metadata_chunk(bytes, chunk)?);
            }
            kind if kind == *b"ALPH" => {
                advance_rank(&mut rank, 2, "ALPH")?;
                alpha = Some(chunk.location);
                validate_alpha_header(bytes, chunk, canvas)?;
            }
            kind if kind == *b"VP8 " || kind == *b"VP8L" => {
                advance_rank(&mut rank, 3, "image data")?;
                if image.is_some() {
                    return Err(malformed("duplicate or conflicting image data chunk"));
                }
                let parsed = parse_image_chunk(bytes, chunk, limits)?;
                if parsed.dimensions != canvas {
                    return Err(malformed("VP8/VP8L dimensions differ from the VP8X canvas"));
                }
                if kind == *b"VP8 " && declared.alpha {
                    let previous = index.checked_sub(1).and_then(|value| chunks.get(value));
                    if previous.is_none_or(|previous| previous.kind != *b"ALPH") {
                        return Err(malformed("ALPH must immediately precede lossy VP8 data"));
                    }
                }
                image = Some(parsed);
            }
            kind if kind == *b"EXIF" => {
                // WebP metadata MAY appear out of order. It therefore does not
                // participate in the reconstruction-chunk ordering state.
                metadata_bytes = add_metadata(metadata_bytes, chunk, limits)?;
                metadata.exif = Some(metadata_chunk(bytes, chunk)?);
            }
            kind if kind == *b"XMP " => {
                // XMP has the same free placement rule as EXIF.
                metadata_bytes = add_metadata(metadata_bytes, chunk, limits)?;
                metadata.xmp = Some(metadata_chunk(bytes, chunk)?);
            }
            _ => {}
        }
    }
    if declared.animation || animation_chunk {
        return Err(WebPDecodeError::UnsupportedAnimation);
    }
    let image = image.ok_or_else(|| malformed("extended WebP is missing image data"))?;
    if image.coding == WebPCodingMode::LosslessVp8l && alpha.is_some() {
        return Err(malformed("ALPH is forbidden with VP8L image data"));
    }
    let actual_alpha = match image.coding {
        WebPCodingMode::LossyVp8 => alpha.is_some(),
        WebPCodingMode::LosslessVp8l => image.alpha,
    };
    let actual = WebPFeatures {
        icc_profile: metadata.icc_profile.is_some(),
        alpha: actual_alpha,
        exif: metadata.exif.is_some(),
        xmp: metadata.xmp.is_some(),
        animation: false,
    };
    if declared != actual {
        return Err(malformed(
            "VP8X feature flags do not match the present WebP chunks",
        ));
    }
    enforce_output(canvas, if actual_alpha { 4 } else { 3 }, limits)?;
    Ok((image, actual, metadata, alpha))
}

fn parse_vp8x(
    bytes: &[u8],
    chunk: &WebPChunk,
    limits: WebPDecodeLimits,
) -> Result<(u8, ImageDimensions), WebPDecodeError> {
    if chunk.location.length != VP8X_BYTES as u64 {
        return Err(malformed("VP8X payload must be exactly 10 bytes"));
    }
    let data = payload(bytes, chunk)?;
    let flags = data[0];
    if flags & !VP8X_VALID_FLAGS != 0 || data[1..4] != [0, 0, 0] {
        return Err(malformed("VP8X reserved bits or bytes are nonzero"));
    }
    let width = le_u24(&data[4..7])
        .checked_add(1)
        .ok_or_else(|| malformed("VP8X width overflows"))?;
    let height = le_u24(&data[7..10])
        .checked_add(1)
        .ok_or_else(|| malformed("VP8X height overflows"))?;
    let dimensions = dimensions(width, height)?;
    enforce_dimensions(dimensions, limits)?;
    Ok((flags, dimensions))
}

fn parse_image_chunk(
    bytes: &[u8],
    chunk: &WebPChunk,
    limits: WebPDecodeLimits,
) -> Result<ImageInfo, WebPDecodeError> {
    let data = payload(bytes, chunk)?;
    let (coding, dimensions, alpha) = match chunk.kind {
        kind if kind == *b"VP8 " => {
            if data.len() < VP8_MINIMUM_BYTES {
                return Err(malformed("VP8 keyframe header is truncated"));
            }
            let frame_tag = le_u24(&data[..3]);
            if frame_tag & 1 != 0 {
                return Err(malformed("VP8 WebP image is not a keyframe"));
            }
            if data[3..6] != [0x9d, 0x01, 0x2a] {
                return Err(malformed("VP8 keyframe signature is invalid"));
            }
            let width = u32::from(u16::from_le_bytes([data[6], data[7]]) & 0x3fff);
            let height = u32::from(u16::from_le_bytes([data[8], data[9]]) & 0x3fff);
            (WebPCodingMode::LossyVp8, dimensions(width, height)?, false)
        }
        kind if kind == *b"VP8L" => {
            if data.len() < VP8L_MINIMUM_BYTES {
                return Err(malformed("VP8L header is truncated"));
            }
            if data[0] != 0x2f {
                return Err(malformed("VP8L signature is invalid"));
            }
            let bits = le_u32(&data[1..5]);
            if bits >> 29 != 0 {
                return Err(malformed("VP8L version bits are nonzero"));
            }
            let width = (bits & 0x3fff) + 1;
            let height = ((bits >> 14) & 0x3fff) + 1;
            (
                WebPCodingMode::LosslessVp8l,
                dimensions(width, height)?,
                bits & (1 << 28) != 0,
            )
        }
        _ => return Err(malformed("chunk is not VP8 or VP8L image data")),
    };
    enforce_dimensions(dimensions, limits)?;
    let channels = if alpha { 4 } else { 3 };
    enforce_output(dimensions, channels, limits)?;
    Ok(ImageInfo {
        coding,
        dimensions,
        alpha,
        location: chunk.location,
    })
}

fn validate_alpha_header(
    bytes: &[u8],
    chunk: &WebPChunk,
    canvas: ImageDimensions,
) -> Result<(), WebPDecodeError> {
    let data = payload(bytes, chunk)?;
    let info = *data
        .first()
        .ok_or_else(|| malformed("ALPH payload is empty"))?;
    if info & 0b1100_0000 != 0 {
        return Err(malformed("ALPH reserved bits are nonzero"));
    }
    let preprocessing = (info >> 4) & 0b11;
    let compression = info & 0b11;
    if preprocessing > 1 {
        return Err(malformed("ALPH preprocessing mode is invalid"));
    }
    if compression > 1 {
        return Err(malformed("ALPH compression mode is invalid"));
    }
    if compression == 0 {
        let expected = canvas
            .pixel_count()
            .map_err(|_| malformed("ALPH dimensions overflow"))?
            .checked_add(1)
            .ok_or_else(|| malformed("ALPH byte count overflows"))?;
        if chunk.location.length != expected {
            return Err(malformed(
                "uncompressed ALPH length does not match canvas dimensions",
            ));
        }
    }
    Ok(())
}

fn metadata_chunk(bytes: &[u8], chunk: &WebPChunk) -> Result<WebPMetadataChunk, WebPDecodeError> {
    let data = payload(bytes, chunk)?;
    if data.is_empty() {
        return Err(malformed("metadata chunks must not be empty"));
    }
    Ok(WebPMetadataChunk {
        location: chunk.location,
        sha256: Sha256::digest(data).into(),
    })
}

fn payload<'a>(bytes: &'a [u8], chunk: &WebPChunk) -> Result<&'a [u8], WebPDecodeError> {
    let start = usize::try_from(chunk.location.offset)
        .map_err(|_| malformed("chunk offset is not representable"))?;
    let length = usize::try_from(chunk.location.length)
        .map_err(|_| malformed("chunk length is not representable"))?;
    let end = start
        .checked_add(length)
        .ok_or_else(|| malformed("chunk payload range overflows"))?;
    bytes
        .get(start..end)
        .ok_or_else(|| malformed("chunk payload is outside the source"))
}

fn add_metadata(
    current: u64,
    chunk: &WebPChunk,
    limits: WebPDecodeLimits,
) -> Result<u64, WebPDecodeError> {
    let total = current
        .checked_add(chunk.location.length)
        .ok_or_else(|| malformed("metadata byte count overflows"))?;
    if total > limits.max_metadata_bytes {
        return Err(limit("metadata bytes", total, limits.max_metadata_bytes));
    }
    Ok(total)
}

fn advance_rank(rank: &mut u8, next: u8, name: &str) -> Result<(), WebPDecodeError> {
    if next < *rank {
        return Err(malformed(&format!("{name} chunk is out of order")));
    }
    *rank = next;
    Ok(())
}

fn is_singleton(kind: [u8; 4]) -> bool {
    matches!(
        &kind,
        b"VP8X" | b"VP8 " | b"VP8L" | b"ALPH" | b"ICCP" | b"EXIF" | b"XMP " | b"ANIM"
    )
}

fn features_from_flags(flags: u8) -> WebPFeatures {
    WebPFeatures {
        icc_profile: flags & 0b0010_0000 != 0,
        alpha: flags & 0b0001_0000 != 0,
        exif: flags & 0b0000_1000 != 0,
        xmp: flags & 0b0000_0100 != 0,
        animation: flags & 0b0000_0010 != 0,
    }
}

fn enforce_source(bytes: &[u8], limits: WebPDecodeLimits) -> Result<(), WebPDecodeError> {
    let actual =
        u64::try_from(bytes.len()).map_err(|_| malformed("source length is not representable"))?;
    if actual > limits.max_source_bytes {
        return Err(limit("source bytes", actual, limits.max_source_bytes));
    }
    Ok(())
}

fn enforce_dimensions(
    dimensions: ImageDimensions,
    limits: WebPDecodeLimits,
) -> Result<(), WebPDecodeError> {
    if dimensions.width() > limits.max_width {
        return Err(limit(
            "width",
            u64::from(dimensions.width()),
            u64::from(limits.max_width),
        ));
    }
    if dimensions.height() > limits.max_height {
        return Err(limit(
            "height",
            u64::from(dimensions.height()),
            u64::from(limits.max_height),
        ));
    }
    let pixels = dimensions
        .pixel_count()
        .map_err(|_| malformed("pixel count overflows"))?;
    if pixels > limits.max_pixels {
        return Err(limit("pixels", pixels, limits.max_pixels));
    }
    Ok(())
}

fn enforce_output(
    dimensions: ImageDimensions,
    channels: u64,
    limits: WebPDecodeLimits,
) -> Result<u64, WebPDecodeError> {
    let output = dimensions
        .pixel_count()
        .map_err(|_| malformed("pixel count overflows"))?
        .checked_mul(channels)
        .ok_or_else(|| malformed("decoded byte count overflows"))?;
    if output > limits.max_decoded_bytes {
        return Err(limit("decoded bytes", output, limits.max_decoded_bytes));
    }
    let temporary = output
        .checked_add(
            dimensions
                .pixel_count()
                .map_err(|_| malformed("pixel count overflows"))?
                .saturating_mul(5),
        )
        .ok_or_else(|| malformed("temporary byte count overflows"))?;
    if temporary > limits.max_temporary_bytes {
        return Err(limit(
            "temporary bytes",
            temporary,
            limits.max_temporary_bytes,
        ));
    }
    Ok(output)
}

fn dimensions(width: u32, height: u32) -> Result<ImageDimensions, WebPDecodeError> {
    ImageDimensions::new(width, height)
        .map_err(|_| malformed("WebP dimensions must both be nonzero"))
}

fn le_u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn le_u32(bytes: &[u8]) -> u32 {
    u32::from_le_bytes(bytes[..4].try_into().expect("four-byte integer"))
}

fn fourcc(kind: [u8; 4]) -> String {
    String::from_utf8_lossy(&kind).into_owned()
}

fn malformed(message: &str) -> WebPDecodeError {
    WebPDecodeError::Malformed(message.to_owned())
}

fn limit(kind: &'static str, actual: u64, limit: u64) -> WebPDecodeError {
    WebPDecodeError::Limit {
        kind,
        actual,
        limit,
    }
}
