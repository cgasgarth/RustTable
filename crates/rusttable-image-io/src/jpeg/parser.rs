use rusttable_image::{
    DecodeLimits, ImageDimensions, ImageInputError, InputFormat, Orientation,
    UnsupportedImageFeature,
};

use super::types::{
    JpegCodingProcess, JpegComponentModel, JpegHeader, JpegMetadataSegment, JpegSampling, JpegSof,
};

const MAX_METADATA_ITEMS: usize = 64;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCANS: u16 = 1024;

/// Parses only the deterministic header prefix needed by a registry probe.
///
/// # Errors
///
/// Returns a typed malformed-input, unsupported-feature, or limit error.
pub fn probe(bytes: &[u8], limits: DecodeLimits) -> Result<JpegHeader, ImageInputError> {
    probe_bounded(bytes, limits, false)
}

pub(crate) fn probe_bounded(
    bytes: &[u8],
    limits: DecodeLimits,
    budgeted: bool,
) -> Result<JpegHeader, ImageInputError> {
    let header = parse(bytes, true, false, budgeted)?;
    enforce_limits(header.dimensions, limits)?;
    Ok(header)
}

/// Parses the complete marker and entropy structure before a full decode.
///
/// # Errors
///
/// Returns a typed malformed-input, unsupported-feature, or limit error.
pub fn inspect(bytes: &[u8], limits: DecodeLimits) -> Result<JpegHeader, ImageInputError> {
    let header = parse(bytes, false, true, false)?;
    enforce_limits(header.dimensions, limits)?;
    Ok(header)
}

#[allow(clippy::too_many_lines)]
fn parse(
    bytes: &[u8],
    stop_at_scan: bool,
    require_eoi: bool,
    budgeted: bool,
) -> Result<JpegHeader, ImageInputError> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return Err(malformed("missing JPEG SOI"));
    }

    let mut cursor = 2_usize;
    let mut frame: Option<FrameHeader> = None;
    let mut metadata = Vec::new();
    let mut metadata_bytes = 0_usize;
    let mut scans = 0_u16;
    let mut restart_interval = 0_u16;
    let mut adobe_color_transform = None;
    let mut orientation = Orientation::Normal;
    let mut in_entropy = false;
    let mut pending_marker = None;
    let mut saw_eoi = false;

    while cursor < bytes.len() {
        if in_entropy {
            let marker = next_entropy_marker(bytes, &mut cursor)?;
            match marker {
                0xd0..=0xd7 => {
                    if restart_interval == 0 {
                        return Err(unsupported(UnsupportedImageFeature::RestartInterval));
                    }
                }
                0xd9 => {
                    saw_eoi = true;
                    break;
                }
                _ => {
                    in_entropy = false;
                    pending_marker = Some(marker);
                }
            }
            if in_entropy {
                continue;
            }
        }

        let marker = match pending_marker.take() {
            Some(marker) => marker,
            None => read_marker(bytes, &mut cursor)?,
        };
        match marker {
            0xd8 => return Err(malformed("duplicate JPEG SOI")),
            0xd9 => {
                saw_eoi = true;
                break;
            }
            0x01 => return Err(malformed("TEM marker is not valid in a JPEG header")),
            0xd0..=0xd7 => {
                return Err(malformed("restart marker is outside JPEG entropy data"));
            }
            0xda => {
                let (segment, end) = read_segment(bytes, &mut cursor, budgeted)?;
                parse_scan(segment, frame.as_ref(), scans)?;
                scans = scans
                    .checked_add(1)
                    .ok_or(ImageInputError::ArithmeticOverflow)?;
                if stop_at_scan {
                    return finish(
                        frame,
                        metadata,
                        scans,
                        restart_interval,
                        orientation,
                        adobe_color_transform,
                        budgeted,
                    );
                }
                cursor = end;
                in_entropy = true;
            }
            0xc0..=0xcf if marker != 0xc4 && marker != 0xc8 && marker != 0xcc => {
                let (segment, end) = read_segment(bytes, &mut cursor, budgeted)?;
                if frame.is_some() {
                    return Err(malformed("duplicate or conflicting JPEG frame headers"));
                }
                frame = Some(parse_frame(marker, segment)?);
                cursor = end;
            }
            0xdd => {
                let (segment, end) = read_segment(bytes, &mut cursor, budgeted)?;
                if segment.len() != 2 {
                    return Err(malformed("JPEG restart interval has an invalid length"));
                }
                let interval = u16::from_be_bytes([segment[0], segment[1]]);
                if restart_interval != 0 && restart_interval != interval {
                    return Err(malformed("conflicting JPEG restart intervals"));
                }
                restart_interval = interval;
                cursor = end;
            }
            0xe0..=0xef | 0xfe => {
                let marker_offset = cursor.saturating_sub(2);
                let (segment, end) = read_segment(bytes, &mut cursor, budgeted)?;
                if marker == 0xee && segment.len() >= 12 && &segment[..5] == b"Adobe" {
                    let transform = segment[11];
                    if transform > 2 {
                        return Err(malformed("Adobe APP14 has an invalid color transform"));
                    }
                    adobe_color_transform = Some(transform);
                }
                if marker == 0xe1
                    && let Some(value) = exif_orientation(segment)
                {
                    orientation = value;
                }
                if is_metadata_marker(marker) {
                    metadata_bytes = metadata_bytes
                        .checked_add(segment.len())
                        .ok_or(ImageInputError::ArithmeticOverflow)?;
                    if metadata.len() >= MAX_METADATA_ITEMS || metadata_bytes > MAX_METADATA_BYTES {
                        return Err(malformed("JPEG metadata inventory exceeds its bound"));
                    }
                    metadata.push(JpegMetadataSegment {
                        marker,
                        offset: u64::try_from(marker_offset)
                            .map_err(|_| ImageInputError::ArithmeticOverflow)?,
                        byte_length: u32::try_from(segment.len())
                            .map_err(|_| ImageInputError::ArithmeticOverflow)?,
                        data: segment.to_vec(),
                    });
                }
                cursor = end;
            }
            _ => {
                let (_, end) = read_segment(bytes, &mut cursor, budgeted)?;
                cursor = end;
            }
        }
        if !in_entropy && pending_marker.is_none() {
            skip_marker_padding(bytes, &mut cursor);
        }
    }

    if in_entropy && !saw_eoi && require_eoi {
        return Err(malformed("truncated JPEG entropy data"));
    }
    if require_eoi && !saw_eoi {
        return Err(malformed("JPEG is missing the EOI marker"));
    }
    finish(
        frame,
        metadata,
        scans,
        restart_interval,
        orientation,
        adobe_color_transform,
        budgeted,
    )
}

fn finish(
    frame: Option<FrameHeader>,
    metadata: Vec<JpegMetadataSegment>,
    scans: u16,
    restart_interval: u16,
    orientation: Orientation,
    adobe_color_transform: Option<u8>,
    budgeted: bool,
) -> Result<JpegHeader, ImageInputError> {
    let frame = frame.ok_or_else(|| {
        if budgeted {
            ImageInputError::ProbeBudgetExceeded {
                limit: super::types::JPEG_PROBE_BUDGET_BYTES as u64,
            }
        } else {
            malformed("JPEG frame header is missing")
        }
    })?;
    let dimensions = ImageDimensions::new(frame.width, frame.height)
        .map_err(|_| malformed("JPEG dimensions must be nonzero"))?;
    let components = component_model(&frame, adobe_color_transform);
    let coding_process = match frame.sof {
        JpegSof::BaselineDct => JpegCodingProcess::Baseline,
        JpegSof::ExtendedSequentialDct => JpegCodingProcess::ExtendedSequential,
        JpegSof::ProgressiveDct => JpegCodingProcess::Progressive,
        JpegSof::Lossless => JpegCodingProcess::Lossless,
        JpegSof::Arithmetic => JpegCodingProcess::Arithmetic,
        sof => JpegCodingProcess::Unsupported(sof),
    };
    Ok(JpegHeader {
        dimensions,
        precision: frame.precision,
        components,
        coding_process,
        sof: frame.sof,
        sampling: frame.sampling,
        scans,
        restart_interval,
        orientation,
        adobe_color_transform,
        metadata,
    })
}

#[derive(Debug, Clone)]
struct FrameHeader {
    width: u32,
    height: u32,
    precision: u8,
    sof: JpegSof,
    components: Vec<u8>,
    sampling: Vec<JpegSampling>,
}

fn parse_frame(marker: u8, segment: &[u8]) -> Result<FrameHeader, ImageInputError> {
    if segment.len() < 6 {
        return Err(malformed("truncated JPEG frame header"));
    }
    let precision = segment[0];
    let height = u32::from(u16::from_be_bytes([segment[1], segment[2]]));
    let width = u32::from(u16::from_be_bytes([segment[3], segment[4]]));
    let component_count = usize::from(segment[5]);
    if component_count == 0 || component_count > 4 {
        return Err(unsupported(UnsupportedImageFeature::ColorModel));
    }
    let expected = 6_usize
        .checked_add(
            component_count
                .checked_mul(3)
                .ok_or(ImageInputError::ArithmeticOverflow)?,
        )
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    if segment.len() != expected {
        return Err(malformed(
            "JPEG frame header length does not match its components",
        ));
    }
    let sof = sof_kind(marker);
    let mut components = Vec::with_capacity(component_count);
    let mut sampling = Vec::with_capacity(component_count);
    for chunk in segment[6..].as_chunks::<3>().0 {
        let id = chunk[0];
        if components.contains(&id) {
            return Err(malformed(
                "JPEG frame contains duplicate component identifiers",
            ));
        }
        let horizontal = chunk[1] >> 4;
        let vertical = chunk[1] & 0x0f;
        if !(1..=4).contains(&horizontal) || !(1..=4).contains(&vertical) {
            return Err(unsupported(UnsupportedImageFeature::Sampling));
        }
        if chunk[2] > 3 {
            return Err(malformed(
                "JPEG component references an invalid quantization table",
            ));
        }
        components.push(id);
        sampling.push(JpegSampling {
            component_id: id,
            horizontal,
            vertical,
            quantization_table: chunk[2],
        });
    }
    if width == 0 || height == 0 {
        return Err(malformed("JPEG dimensions must be nonzero"));
    }
    if precision != 8 && precision != 12 {
        return Err(unsupported(UnsupportedImageFeature::BitDepth));
    }
    Ok(FrameHeader {
        width,
        height,
        precision,
        sof,
        components,
        sampling,
    })
}

fn parse_scan(
    segment: &[u8],
    frame: Option<&FrameHeader>,
    scans: u16,
) -> Result<(), ImageInputError> {
    let frame = frame.ok_or_else(|| malformed("JPEG scan precedes its frame header"))?;
    if scans >= MAX_SCANS || segment.len() < 4 {
        return Err(malformed("JPEG has too many or too-short scans"));
    }
    let count = usize::from(segment[0]);
    let expected = 1_usize
        .checked_add(
            count
                .checked_mul(2)
                .ok_or(ImageInputError::ArithmeticOverflow)?,
        )
        .and_then(|value| value.checked_add(3))
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    if count == 0 || count > frame.components.len() || segment.len() != expected {
        return Err(malformed("JPEG scan component list is invalid"));
    }
    let mut seen = Vec::with_capacity(count);
    let component_bytes_end = 1 + count * 2;
    for chunk in segment[1..component_bytes_end].as_chunks::<2>().0 {
        if !frame.components.contains(&chunk[0]) || seen.contains(&chunk[0]) {
            return Err(malformed("JPEG scan references an invalid component"));
        }
        if chunk[1] >> 4 > 3 || (chunk[1] & 0x0f) > 3 {
            return Err(malformed("JPEG scan references an invalid Huffman table"));
        }
        seen.push(chunk[0]);
    }
    let spectral_start = segment[1 + count * 2];
    let spectral_end = segment[2 + count * 2];
    let approximation = segment[3 + count * 2];
    if spectral_start > spectral_end
        || spectral_end > 63
        || approximation >> 4 > 13
        || approximation & 0x0f > 13
    {
        return Err(malformed("JPEG scan spectral selection is invalid"));
    }
    if matches!(
        frame.sof,
        JpegSof::BaselineDct | JpegSof::ExtendedSequentialDct
    ) && (spectral_start != 0 || spectral_end != 63 || approximation != 0)
    {
        return Err(malformed("sequential JPEG has progressive scan parameters"));
    }
    Ok(())
}

fn next_entropy_marker(bytes: &[u8], cursor: &mut usize) -> Result<u8, ImageInputError> {
    while *cursor < bytes.len() {
        if bytes[*cursor] != 0xff {
            *cursor += 1;
            continue;
        }
        while bytes.get(*cursor) == Some(&0xff) {
            *cursor += 1;
        }
        let marker = *bytes
            .get(*cursor)
            .ok_or_else(|| malformed("truncated JPEG entropy marker"))?;
        *cursor += 1;
        if marker == 0 {
            continue;
        }
        return Ok(marker);
    }
    Err(malformed("truncated JPEG entropy data"))
}

fn read_marker(bytes: &[u8], cursor: &mut usize) -> Result<u8, ImageInputError> {
    if bytes.get(*cursor) != Some(&0xff) {
        return Err(malformed("JPEG marker prefix is missing"));
    }
    while bytes.get(*cursor) == Some(&0xff) {
        *cursor += 1;
    }
    let marker = *bytes
        .get(*cursor)
        .ok_or_else(|| malformed("truncated JPEG marker"))?;
    *cursor += 1;
    if marker == 0 {
        return Err(malformed("JPEG stuffed byte appears outside entropy data"));
    }
    Ok(marker)
}

fn skip_marker_padding(bytes: &[u8], cursor: &mut usize) {
    let start = *cursor;
    while (*cursor).saturating_sub(start) < 8 && bytes.get(*cursor) != Some(&0xff) {
        *cursor += 1;
    }
}

fn read_segment<'a>(
    bytes: &'a [u8],
    cursor: &mut usize,
    budgeted: bool,
) -> Result<(&'a [u8], usize), ImageInputError> {
    let length_bytes = bytes
        .get(*cursor..(*cursor).saturating_add(2))
        .ok_or_else(|| segment_limit_error(budgeted, "truncated JPEG segment length"))?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    if length < 2 {
        return Err(malformed("JPEG segment length is less than two"));
    }
    let start = (*cursor)
        .checked_add(2)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let end = start
        .checked_add(length - 2)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let segment = bytes
        .get(start..end)
        .ok_or_else(|| segment_limit_error(budgeted, "JPEG segment exceeds the source"))?;
    *cursor = end;
    Ok((segment, end))
}

fn segment_limit_error(budgeted: bool, message: &str) -> ImageInputError {
    if budgeted {
        ImageInputError::ProbeBudgetExceeded {
            limit: super::types::JPEG_PROBE_BUDGET_BYTES as u64,
        }
    } else {
        malformed(message)
    }
}

fn sof_kind(marker: u8) -> JpegSof {
    match marker {
        0xc0 => JpegSof::BaselineDct,
        0xc1 => JpegSof::ExtendedSequentialDct,
        0xc2 => JpegSof::ProgressiveDct,
        0xc3 => JpegSof::Lossless,
        0xc5..=0xc7 => JpegSof::Differential,
        0xc9..=0xcb | 0xcd..=0xcf => JpegSof::Arithmetic,
        _ => JpegSof::Unknown(marker),
    }
}

fn component_model(frame: &FrameHeader, adobe: Option<u8>) -> JpegComponentModel {
    match frame.components.as_slice() {
        [_] => JpegComponentModel::Gray,
        [82, 71, 66] => JpegComponentModel::Rgb,
        [_, _, _] if adobe == Some(0) => JpegComponentModel::Rgb,
        [_, _, _] => JpegComponentModel::Ycbcr,
        [_, _, _, _] if adobe == Some(2) => JpegComponentModel::Ycck,
        [_, _, _, _] => JpegComponentModel::Cmyk,
        _ => JpegComponentModel::Unknown(
            u8::try_from(frame.components.len()).expect("JPEG component count is bounded"),
        ),
    }
}

fn exif_orientation(segment: &[u8]) -> Option<Orientation> {
    let tiff = segment.strip_prefix(b"Exif\0\0")?;
    let little = match tiff.get(0..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if read_u16(tiff, 2, little)? != 42 {
        return None;
    }
    let ifd = usize::try_from(read_u32(tiff, 4, little)?).ok()?;
    let count = usize::from(read_u16(tiff, ifd, little)?);
    if count > 256 {
        return None;
    }
    for index in 0..count {
        let entry = ifd.checked_add(2)?.checked_add(index.checked_mul(12)?)?;
        if read_u16(tiff, entry, little)? != 0x0112 {
            continue;
        }
        if read_u16(tiff, entry + 2, little)? != 3 || read_u32(tiff, entry + 4, little)? != 1 {
            return None;
        }
        return match read_u16(tiff, entry + 8, little)? {
            1 => Some(Orientation::Normal),
            2 => Some(Orientation::FlipHorizontal),
            3 => Some(Orientation::Rotate180),
            4 => Some(Orientation::FlipVertical),
            5 => Some(Orientation::Transpose),
            6 => Some(Orientation::Rotate90),
            7 => Some(Orientation::Transverse),
            8 => Some(Orientation::Rotate270),
            _ => None,
        };
    }
    None
}

fn read_u16(bytes: &[u8], offset: usize, little: bool) -> Option<u16> {
    let value = bytes.get(offset..offset.checked_add(2)?)?;
    Some(if little {
        u16::from_le_bytes([value[0], value[1]])
    } else {
        u16::from_be_bytes([value[0], value[1]])
    })
}

fn read_u32(bytes: &[u8], offset: usize, little: bool) -> Option<u32> {
    let value = bytes.get(offset..offset.checked_add(4)?)?;
    Some(if little {
        u32::from_le_bytes([value[0], value[1], value[2], value[3]])
    } else {
        u32::from_be_bytes([value[0], value[1], value[2], value[3]])
    })
}

fn is_metadata_marker(marker: u8) -> bool {
    matches!(marker, 0xe1 | 0xe2 | 0xee | 0xfe)
}

fn enforce_limits(
    dimensions: ImageDimensions,
    limits: DecodeLimits,
) -> Result<(), ImageInputError> {
    if dimensions.width() > limits.max_width() {
        return Err(ImageInputError::WidthLimit {
            actual: dimensions.width(),
            limit: limits.max_width(),
        });
    }
    if dimensions.height() > limits.max_height() {
        return Err(ImageInputError::HeightLimit {
            actual: dimensions.height(),
            limit: limits.max_height(),
        });
    }
    let pixels = dimensions
        .pixel_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if pixels > limits.max_pixel_count() {
        return Err(ImageInputError::PixelLimit {
            actual: pixels,
            limit: limits.max_pixel_count(),
        });
    }
    Ok(())
}

fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Jpeg,
        message: message.to_owned(),
    }
}

fn unsupported(reason: UnsupportedImageFeature) -> ImageInputError {
    ImageInputError::UnsupportedFeature {
        format: InputFormat::Jpeg,
        reason,
    }
}
