use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io::Cursor;

use exr::meta::attribute::{AttributeValue, LevelMode, SampleType, Text};
use exr::meta::{BlockDescription, MetaData, compute_level_count};
use sha2::{Digest, Sha256};

use super::types::{
    ExrBlobMetadata, ExrChannel, ExrChannelRole, ExrChromaticities, ExrCompression, ExrDecodeError,
    ExrDecodeLimits, ExrHeader, ExrLayerView, ExrLevelMode, ExrMetadataInventory, ExrPart,
    ExrSampleType, ExrStorage, ExrWindow,
};

const MAGIC: [u8; 4] = [0x76, 0x2f, 0x31, 0x01];
const KNOWN_FLAGS: u32 = 0x0000_1e00;
const MULTIPART_FLAG: u32 = 0x0000_1000;

pub(crate) struct Inspection {
    pub header: ExrHeader,
    pub meta: MetaData,
}

pub(crate) fn inspect(bytes: &[u8], limits: ExrDecodeLimits) -> Result<Inspection, ExrDecodeError> {
    enforce_limit("source bytes", bytes.len() as u64, limits.max_source_bytes)?;
    let preflight = preflight_headers(bytes, limits)?;
    let reader = exr::block::read(Cursor::new(bytes), true).map_err(map_backend)?;
    let meta = reader.into_meta_data();
    if meta.headers.len() != preflight.part_attributes.len() {
        return Err(ExrDecodeError::Malformed(
            "header count changed during validated parsing".to_owned(),
        ));
    }
    validate_chunk_tables(bytes, preflight.headers_end, &meta, limits)?;
    let mut parts = Vec::new();
    parts
        .try_reserve_exact(meta.headers.len())
        .map_err(|_| ExrDecodeError::AllocationFailure)?;
    for (index, header) in meta.headers.iter().enumerate() {
        parts.push(normalize_part(
            index,
            header,
            preflight.part_attributes[index],
            limits,
        )?);
    }
    let default_part = parts
        .iter()
        .find(|part| !part.deep && part.layers.iter().any(is_complete_group))
        .map(|part| part.index);
    Ok(Inspection {
        header: ExrHeader {
            version: preflight.version,
            flags: preflight.flags,
            parts,
            default_part,
        },
        meta,
    })
}

struct Preflight {
    version: u8,
    flags: u32,
    headers_end: usize,
    part_attributes: Vec<u32>,
}

fn preflight_headers(bytes: &[u8], limits: ExrDecodeLimits) -> Result<Preflight, ExrDecodeError> {
    if !bytes.starts_with(&MAGIC) {
        return Err(ExrDecodeError::NotOpenExr);
    }
    let word = read_u32(bytes, 4)?;
    let version = (word & 0xff) as u8;
    let flags = word & !0xff;
    if !(1..=2).contains(&version) {
        return Err(ExrDecodeError::UnsupportedVersion(version));
    }
    if flags & !KNOWN_FLAGS != 0 {
        return Err(ExrDecodeError::UnsupportedFlags(flags));
    }
    let multipart = flags & MULTIPART_FLAG != 0;
    let mut cursor = 8usize;
    let mut part_attributes = Vec::new();
    loop {
        enforce_limit("header bytes", cursor as u64, limits.max_header_bytes)?;
        if multipart && bytes.get(cursor) == Some(&0) {
            cursor += 1;
            break;
        }
        enforce_limit(
            "part count",
            part_attributes.len() as u64 + 1,
            u64::from(limits.max_parts),
        )?;
        let mut names = HashSet::new();
        let mut count = 0u32;
        loop {
            let name = read_cstring(bytes, &mut cursor)?;
            if name.is_empty() {
                break;
            }
            count = count
                .checked_add(1)
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            enforce_limit(
                "attribute count",
                u64::from(count),
                u64::from(limits.max_attributes_per_part),
            )?;
            if !names.insert(name.clone()) {
                return Err(ExrDecodeError::Malformed(format!(
                    "duplicate attribute name {name}"
                )));
            }
            let _kind = read_cstring(bytes, &mut cursor)?;
            let size = read_i32(bytes, cursor)?;
            cursor = cursor
                .checked_add(4)
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            let size = usize::try_from(size)
                .map_err(|_| ExrDecodeError::Malformed("negative attribute size".to_owned()))?;
            enforce_limit("metadata bytes", size as u64, limits.max_metadata_bytes)?;
            cursor = cursor
                .checked_add(size)
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            if cursor > bytes.len() {
                return Err(ExrDecodeError::Malformed(
                    "attribute payload is truncated".to_owned(),
                ));
            }
        }
        part_attributes.push(count);
        if !multipart {
            break;
        }
    }
    enforce_limit("header bytes", cursor as u64, limits.max_header_bytes)?;
    Ok(Preflight {
        version,
        flags,
        headers_end: cursor,
        part_attributes,
    })
}

fn normalize_part(
    index: usize,
    header: &exr::meta::header::Header,
    attribute_count: u32,
    limits: ExrDecodeLimits,
) -> Result<ExrPart, ExrDecodeError> {
    enforce_limit(
        "attribute count",
        u64::from(attribute_count),
        u64::from(limits.max_attributes_per_part),
    )?;
    enforce_limit(
        "channel count",
        header.channels.list.len() as u64,
        u64::from(limits.max_channels_per_part),
    )?;
    let data_window = window(
        header.own_attributes.layer_position.x(),
        header.own_attributes.layer_position.y(),
        header.layer_size.width(),
        header.layer_size.height(),
    )?;
    let display = header.shared_attributes.display_window;
    let display_window = window(
        display.position.x(),
        display.position.y(),
        display.size.width(),
        display.size.height(),
    )?;
    validate_window(data_window, limits)?;
    validate_window(display_window, limits)?;
    let (storage, level_count) = storage_and_levels(header, limits)?;
    enforce_limit("chunk count", header.chunk_count as u64, limits.max_chunks)?;
    let compression = compression(index, header.compression)?;
    let views = declared_views(header);
    let part_view = header.own_attributes.view_name.as_ref().map(text);
    let channels = header
        .channels
        .list
        .iter()
        .map(|channel| {
            let name = text(&channel.name);
            let (layer, view, role) = classify_channel(&name, &views, part_view.as_deref());
            Ok(ExrChannel {
                name,
                layer,
                view,
                role,
                sample_type: sample_type(channel.sample_type),
                x_sampling: u32::try_from(channel.sampling.x())
                    .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                y_sampling: u32::try_from(channel.sampling.y())
                    .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
            })
        })
        .collect::<Result<Vec<_>, ExrDecodeError>>()?;
    let layers = layer_views(&channels);
    let mut normalized_views = views;
    normalized_views.extend(channels.iter().map(|channel| channel.view.clone()));
    normalized_views.sort();
    normalized_views.dedup();
    normalized_views.retain(|view| !view.is_empty());
    let metadata = metadata(header, limits)?;
    let decompressed = logical_bytes(header)?;
    enforce_limit(
        "decompressed bytes",
        decompressed,
        limits.max_decompressed_bytes,
    )?;
    Ok(ExrPart {
        index,
        name: header
            .own_attributes
            .layer_name
            .as_ref()
            .map_or_else(String::new, text),
        deep: header.deep,
        compression,
        data_window,
        display_window,
        storage,
        level_count,
        chunk_count: header.chunk_count as u64,
        channels,
        layers,
        views: normalized_views,
        metadata,
    })
}

fn storage_and_levels(
    header: &exr::meta::header::Header,
    limits: ExrDecodeLimits,
) -> Result<(ExrStorage, [u32; 2]), ExrDecodeError> {
    match header.blocks {
        BlockDescription::ScanLines => Ok((ExrStorage::ScanLines, [1, 1])),
        BlockDescription::Tiles(tile) => {
            let x = compute_level_count(tile.rounding_mode, header.layer_size.width());
            let y = compute_level_count(tile.rounding_mode, header.layer_size.height());
            let count = match tile.level_mode {
                LevelMode::Singular => [1, 1],
                LevelMode::MipMap => {
                    let both = x.max(y);
                    [both, both]
                }
                LevelMode::RipMap => [x, y],
            };
            for value in count {
                enforce_limit(
                    "levels per axis",
                    value as u64,
                    u64::from(limits.max_levels_per_axis),
                )?;
            }
            Ok((
                ExrStorage::Tiles {
                    width: u32::try_from(tile.tile_size.width())
                        .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                    height: u32::try_from(tile.tile_size.height())
                        .map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                    levels: match tile.level_mode {
                        LevelMode::Singular => ExrLevelMode::Singular,
                        LevelMode::MipMap => ExrLevelMode::MipMap,
                        LevelMode::RipMap => ExrLevelMode::RipMap,
                    },
                },
                [
                    u32::try_from(count[0]).map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                    u32::try_from(count[1]).map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
                ],
            ))
        }
    }
}

fn metadata(
    header: &exr::meta::header::Header,
    limits: ExrDecodeLimits,
) -> Result<ExrMetadataInventory, ExrDecodeError> {
    let mut names = Vec::new();
    let mut bytes = 0u64;
    let mut icc = None;
    let mut xmp = None;
    for (name, value) in header
        .shared_attributes
        .other
        .iter()
        .chain(header.own_attributes.other.iter())
    {
        let name = text(name);
        let value_bytes = value.byte_size() as u64;
        bytes = bytes
            .checked_add(value_bytes)
            .ok_or(ExrDecodeError::ArithmeticOverflow)?;
        enforce_limit("metadata bytes", bytes, limits.max_metadata_bytes)?;
        let lower = name.to_ascii_lowercase();
        if lower.contains("icc") {
            icc = blob(&name, value);
        } else if lower.contains("xmp") {
            xmp = blob(&name, value);
        }
        names.push(name);
    }
    names.sort();
    Ok(ExrMetadataInventory {
        chromaticities: header
            .shared_attributes
            .chromaticities
            .map(|value| ExrChromaticities {
                red: [value.red.x(), value.red.y()],
                green: [value.green.x(), value.green.y()],
                blue: [value.blue.x(), value.blue.y()],
                white: [value.white.x(), value.white.y()],
            }),
        adopted_neutral: header
            .own_attributes
            .adopted_neutral
            .map(|value| [value.x(), value.y()]),
        white_luminance: header.own_attributes.white_luminance,
        icc,
        xmp,
        attribute_names: names,
        attribute_bytes: bytes,
    })
}

fn blob(name: &str, value: &AttributeValue) -> Option<ExrBlobMetadata> {
    let (type_name, bytes): (String, &[u8]) = match value {
        AttributeValue::Bytes { type_hint, bytes } => (text(type_hint), bytes),
        AttributeValue::Custom { kind, bytes } => (text(kind), bytes),
        AttributeValue::Text(value) => ("string".to_owned(), value.as_slice()),
        _ => return None,
    };
    Some(ExrBlobMetadata {
        attribute: name.to_owned(),
        type_name,
        bytes: bytes.len() as u64,
        sha256: Sha256::digest(bytes).into(),
    })
}

fn layer_views(channels: &[ExrChannel]) -> Vec<ExrLayerView> {
    let mut groups: BTreeMap<(String, String), Vec<&ExrChannel>> = BTreeMap::new();
    for channel in channels.iter().filter(|channel| channel.role.is_some()) {
        groups
            .entry((channel.layer.clone(), channel.view.clone()))
            .or_default()
            .push(channel);
    }
    groups
        .into_iter()
        .map(|((layer, view), channels)| {
            let roles: BTreeSet<_> = channels.iter().filter_map(|channel| channel.role).collect();
            ExrLayerView {
                layer,
                view,
                channels: channels
                    .iter()
                    .map(|channel| channel.name.clone())
                    .collect(),
                has_rgb: roles.contains(&ExrChannelRole::Red)
                    && roles.contains(&ExrChannelRole::Green)
                    && roles.contains(&ExrChannelRole::Blue),
                has_luminance: roles.contains(&ExrChannelRole::Luminance),
                has_alpha: roles.contains(&ExrChannelRole::Alpha),
            }
        })
        .collect()
}

fn classify_channel(
    name: &str,
    views: &[String],
    part_view: Option<&str>,
) -> (String, String, Option<ExrChannelRole>) {
    let mut pieces = name.split('.').map(str::to_owned).collect::<Vec<_>>();
    let role = pieces.last().and_then(|value| match value.as_str() {
        "R" => Some(ExrChannelRole::Red),
        "G" => Some(ExrChannelRole::Green),
        "B" => Some(ExrChannelRole::Blue),
        "A" => Some(ExrChannelRole::Alpha),
        "Y" => Some(ExrChannelRole::Luminance),
        _ => None,
    });
    if role.is_none() {
        return (
            String::new(),
            part_view.unwrap_or_default().to_owned(),
            None,
        );
    }
    pieces.pop();
    let view = if let Some(view) = part_view {
        view.to_owned()
    } else if let Some(position) = pieces
        .iter()
        .rposition(|piece| views.iter().any(|view| view == piece))
    {
        pieces.remove(position)
    } else {
        String::new()
    };
    (pieces.join("."), view, role)
}

fn declared_views(header: &exr::meta::header::Header) -> Vec<String> {
    let mut result = header
        .own_attributes
        .multi_view_names
        .as_ref()
        .map_or_else(Vec::new, |views| views.iter().map(text).collect());
    if let Some(view) = &header.own_attributes.view_name {
        result.push(text(view));
    }
    result.sort();
    result.dedup();
    result
}

fn logical_bytes(header: &exr::meta::header::Header) -> Result<u64, ExrDecodeError> {
    if header.deep {
        return Ok(0);
    }
    let level_pixels =
        |width: usize, height: usize| -> Result<u64, ExrDecodeError> {
            let base = exr::math::Vec2(width, height);
            match header.blocks {
                BlockDescription::ScanLines => Ok(base.area() as u64),
                BlockDescription::Tiles(tile) => match tile.level_mode {
                    LevelMode::Singular => Ok(base.area() as u64),
                    LevelMode::MipMap => exr::meta::mip_map_levels(tile.rounding_mode, base)
                        .try_fold(0u64, |sum, (_, size)| {
                            sum.checked_add(size.area() as u64)
                                .ok_or(ExrDecodeError::ArithmeticOverflow)
                        }),
                    LevelMode::RipMap => exr::meta::rip_map_levels(tile.rounding_mode, base)
                        .try_fold(0u64, |sum, (_, size)| {
                            sum.checked_add(size.area() as u64)
                                .ok_or(ExrDecodeError::ArithmeticOverflow)
                        }),
                },
            }
        };
    header.channels.list.iter().try_fold(0u64, |sum, channel| {
        let pixels = level_pixels(
            header.layer_size.width() / channel.sampling.x(),
            header.layer_size.height() / channel.sampling.y(),
        )?;
        sum.checked_add(
            pixels
                .checked_mul(sample_type(channel.sample_type).bytes())
                .ok_or(ExrDecodeError::ArithmeticOverflow)?,
        )
        .ok_or(ExrDecodeError::ArithmeticOverflow)
    })
}

fn validate_chunk_tables(
    bytes: &[u8],
    headers_end: usize,
    meta: &MetaData,
    limits: ExrDecodeLimits,
) -> Result<(), ExrDecodeError> {
    let total = meta.headers.iter().try_fold(0usize, |sum, header| {
        sum.checked_add(header.chunk_count)
            .ok_or(ExrDecodeError::ArithmeticOverflow)
    })?;
    enforce_limit("chunk count", total as u64, limits.max_chunks)?;
    let table_bytes = total
        .checked_mul(8)
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    let chunks_start = headers_end
        .checked_add(table_bytes)
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    if chunks_start > bytes.len() {
        return Err(ExrDecodeError::Malformed(
            "chunk table is truncated".to_owned(),
        ));
    }
    let multipart = meta.requirements.has_multiple_layers;
    let mut table_cursor = headers_end;
    let mut ranges = Vec::new();
    ranges
        .try_reserve_exact(total)
        .map_err(|_| ExrDecodeError::AllocationFailure)?;
    for (part, header) in meta.headers.iter().enumerate() {
        for _ in 0..header.chunk_count {
            let offset = usize::try_from(read_u64(bytes, table_cursor)?)
                .map_err(|_| ExrDecodeError::ArithmeticOverflow)?;
            table_cursor += 8;
            if offset < chunks_start || offset >= bytes.len() {
                return Err(ExrDecodeError::Malformed(
                    "missing or out-of-bounds chunk offset".to_owned(),
                ));
            }
            let prefix = match (multipart, header.blocks) {
                (false, BlockDescription::ScanLines) => 8,
                (true, BlockDescription::ScanLines) => 12,
                (false, BlockDescription::Tiles(_)) => 20,
                (true, BlockDescription::Tiles(_)) => 24,
            };
            if multipart {
                let chunk_part = read_i32(bytes, offset)?;
                if chunk_part
                    != i32::try_from(part).map_err(|_| ExrDecodeError::ArithmeticOverflow)?
                {
                    return Err(ExrDecodeError::Malformed(
                        "chunk table points to a different part".to_owned(),
                    ));
                }
            }
            let size_offset = offset
                .checked_add(prefix - 4)
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            let payload = usize::try_from(read_i32(bytes, size_offset)?).map_err(|_| {
                ExrDecodeError::Malformed("negative compressed chunk size".to_owned())
            })?;
            let end = offset
                .checked_add(prefix)
                .and_then(|value| value.checked_add(payload))
                .ok_or(ExrDecodeError::ArithmeticOverflow)?;
            if end > bytes.len() {
                return Err(ExrDecodeError::Malformed(
                    "chunk payload is truncated".to_owned(),
                ));
            }
            ranges.push(offset..end);
        }
    }
    ranges.sort_unstable_by_key(|range| range.start);
    if ranges.windows(2).any(|pair| pair[0].end > pair[1].start) {
        return Err(ExrDecodeError::Malformed(
            "overlapping OpenEXR chunks".to_owned(),
        ));
    }
    Ok(())
}

fn compression(
    index: usize,
    value: exr::prelude::Compression,
) -> Result<ExrCompression, ExrDecodeError> {
    Ok(match value {
        exr::prelude::Compression::Uncompressed => ExrCompression::None,
        exr::prelude::Compression::RLE => ExrCompression::Rle,
        exr::prelude::Compression::ZIP1 => ExrCompression::Zips,
        exr::prelude::Compression::ZIP16 => ExrCompression::Zip,
        exr::prelude::Compression::PIZ => ExrCompression::Piz,
        exr::prelude::Compression::PXR24 => ExrCompression::Pxr24,
        exr::prelude::Compression::B44 => ExrCompression::B44,
        exr::prelude::Compression::B44A => ExrCompression::B44A,
        exr::prelude::Compression::DWAA(_) => ExrCompression::Dwaa,
        exr::prelude::Compression::DWAB(_) => ExrCompression::Dwab,
        other => {
            return Err(ExrDecodeError::UnsupportedCompression {
                part: index,
                compression: format!("{other:?}"),
            });
        }
    })
}

fn sample_type(value: SampleType) -> ExrSampleType {
    match value {
        SampleType::F16 => ExrSampleType::F16,
        SampleType::F32 => ExrSampleType::F32,
        SampleType::U32 => ExrSampleType::U32,
    }
}

fn validate_window(window: ExrWindow, limits: ExrDecodeLimits) -> Result<(), ExrDecodeError> {
    enforce_limit(
        "width",
        u64::from(window.width),
        u64::from(limits.max_width),
    )?;
    enforce_limit(
        "height",
        u64::from(window.height),
        u64::from(limits.max_height),
    )?;
    enforce_limit(
        "pixel count",
        u64::from(window.width)
            .checked_mul(u64::from(window.height))
            .ok_or(ExrDecodeError::ArithmeticOverflow)?,
        limits.max_pixels,
    )
}

fn window(x: i32, y: i32, width: usize, height: usize) -> Result<ExrWindow, ExrDecodeError> {
    Ok(ExrWindow {
        x,
        y,
        width: u32::try_from(width).map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
        height: u32::try_from(height).map_err(|_| ExrDecodeError::ArithmeticOverflow)?,
    })
}

fn text(value: &Text) -> String {
    value
        .as_slice()
        .iter()
        .map(|byte| char::from(*byte))
        .collect()
}

fn read_cstring(bytes: &[u8], cursor: &mut usize) -> Result<String, ExrDecodeError> {
    let start = *cursor;
    let relative = bytes
        .get(start..)
        .and_then(|remaining| remaining.iter().position(|byte| *byte == 0))
        .ok_or_else(|| ExrDecodeError::Malformed("unterminated header text".to_owned()))?;
    if relative > 255 {
        return Err(ExrDecodeError::Malformed(
            "header text exceeds 255 bytes".to_owned(),
        ));
    }
    let end = start
        .checked_add(relative)
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    *cursor = end
        .checked_add(1)
        .ok_or(ExrDecodeError::ArithmeticOverflow)?;
    Ok(bytes[start..end]
        .iter()
        .map(|byte| char::from(*byte))
        .collect())
}

fn read_i32(bytes: &[u8], offset: usize) -> Result<i32, ExrDecodeError> {
    Ok(i32::from_le_bytes(read_array(bytes, offset)?))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ExrDecodeError> {
    Ok(u32::from_le_bytes(read_array(bytes, offset)?))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, ExrDecodeError> {
    Ok(u64::from_le_bytes(read_array(bytes, offset)?))
}

fn read_array<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N], ExrDecodeError> {
    bytes
        .get(
            offset
                ..offset
                    .checked_add(N)
                    .ok_or(ExrDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| ExrDecodeError::Malformed("OpenEXR structure is truncated".to_owned()))?
        .try_into()
        .map_err(|_| ExrDecodeError::Malformed("OpenEXR structure is truncated".to_owned()))
}

fn enforce_limit(kind: &'static str, actual: u64, limit: u64) -> Result<(), ExrDecodeError> {
    if actual > limit {
        Err(ExrDecodeError::Limit {
            kind,
            actual,
            limit,
        })
    } else {
        Ok(())
    }
}

fn map_backend(error: exr::error::Error) -> ExrDecodeError {
    match error {
        exr::error::Error::NotSupported(message) => ExrDecodeError::Backend(message.into_owned()),
        exr::error::Error::Invalid(message) => ExrDecodeError::Malformed(message.into_owned()),
        exr::error::Error::Io(error) => ExrDecodeError::Malformed(error.to_string()),
        exr::error::Error::Aborted => ExrDecodeError::Cancelled,
    }
}

pub(crate) fn is_complete_group(group: &ExrLayerView) -> bool {
    group.has_rgb || group.has_luminance
}
