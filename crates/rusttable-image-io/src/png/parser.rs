#![allow(clippy::all, clippy::pedantic)]

use flate2::read::ZlibDecoder;
use rusttable_image::{ColorEncoding, DecodeLimits, ImageDimensions, ImageInputError};
use sha2::{Digest, Sha256};
use std::io::Read;

use super::types::{
    PngAnimation, PngBitDepth, PngChunk, PngChunkInventory, PngColorType, PngDecodeError,
    PngDecodeLimits, PngHeader, PngMetadataInventory, PngPhysicalResolution, PngProfileInventory,
    PngTextInventory,
};

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone)]
pub(crate) struct ParsedPng {
    pub header: PngHeader,
    pub palette: Option<Vec<u8>>,
    pub transparency: Option<Vec<u8>>,
    pub dimensions: ImageDimensions,
}

pub(crate) fn parse(
    bytes: &[u8],
    limits: PngDecodeLimits,
    complete: bool,
) -> Result<ParsedPng, PngDecodeError> {
    if !bytes.starts_with(SIGNATURE) {
        return Err(PngDecodeError::Malformed(
            "missing PNG signature".to_owned(),
        ));
    }
    let mut offset = SIGNATURE.len();
    let mut chunks = PngChunkInventory::default();
    let mut metadata = PngMetadataInventory::default();
    let mut palette = None;
    let mut transparency = None;
    let mut dimensions = None;
    let mut color_type = None;
    let mut bit_depth = None;
    let mut interlaced = false;
    let mut seen_idat = false;
    let mut idat_done = false;
    let mut saw_iend = false;
    let mut saw_actl = false;
    let mut saw_fctl = false;
    let mut saw_fdat = false;
    let mut default_image = false;
    let mut animation = None;
    let mut next_sequence = 0_u32;
    let mut metadata_bytes = 0_u64;
    let mut singleton = std::collections::BTreeSet::<[u8; 4]>::new();
    let mut srgb_intent = None;
    let mut icc_profile = None;

    while offset < bytes.len() {
        if bytes.len() - offset < 8 {
            if !complete {
                break;
            }
            return Err(PngDecodeError::Malformed(
                "truncated PNG chunk header".to_owned(),
            ));
        }
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().expect("length"));
        let kind: [u8; 4] = bytes[offset + 4..offset + 8].try_into().expect("kind");
        validate_chunk_kind(kind)?;
        let length_u64 = u64::from(length);
        if length_u64 > limits.max_chunk_bytes {
            return Err(PngDecodeError::Limit {
                kind: "chunk bytes",
                actual: length_u64,
                limit: limits.max_chunk_bytes,
            });
        }
        let count = u32::try_from(chunks.chunks.len()).unwrap_or(u32::MAX);
        if count >= limits.max_chunks {
            return Err(PngDecodeError::Limit {
                kind: "chunks",
                actual: u64::from(count) + 1,
                limit: u64::from(limits.max_chunks),
            });
        }
        let data_start = offset.checked_add(8).ok_or_else(arithmetic)?;
        let data_end = data_start
            .checked_add(usize::try_from(length).map_err(|_| arithmetic())?)
            .ok_or_else(arithmetic)?;
        let chunk_end = data_end.checked_add(4).ok_or_else(arithmetic)?;
        if chunk_end > bytes.len() {
            return Err(PngDecodeError::Malformed(format!(
                "truncated {} chunk",
                ascii_kind(kind)
            )));
        }
        let data = &bytes[data_start..data_end];
        let stored_crc = u32::from_be_bytes(bytes[data_end..chunk_end].try_into().expect("crc"));
        if crc32(kind, data) != stored_crc {
            return Err(PngDecodeError::Malformed(format!(
                "CRC mismatch in {}",
                ascii_kind(kind)
            )));
        }
        chunks.chunks.push(PngChunk { kind, length });
        offset = chunk_end;

        if kind == *b"IHDR" {
            if chunks.chunks.len() != 1 || data.len() != 13 {
                return Err(PngDecodeError::Malformed(
                    "IHDR must be the first 13-byte chunk".to_owned(),
                ));
            }
            let width = u32::from_be_bytes(data[0..4].try_into().expect("IHDR width"));
            let height = u32::from_be_bytes(data[4..8].try_into().expect("IHDR height"));
            let dims = ImageDimensions::new(width, height).map_err(|_| {
                PngDecodeError::Malformed("PNG dimensions must be nonzero".to_owned())
            })?;
            let ct = decode_color_type(data[9])?;
            let bd = decode_bit_depth(data[8], ct)?;
            if data[10] != 0 || data[11] != 0 || data[12] > 1 {
                return Err(PngDecodeError::Malformed(
                    "unsupported PNG compression, filter, or interlace method".to_owned(),
                ));
            }
            enforce_dimensions(dims, limits)?;
            dimensions = Some(dims);
            color_type = Some(ct);
            bit_depth = Some(bd);
            interlaced = data[12] == 1;
            continue;
        }
        let ct = color_type
            .ok_or_else(|| PngDecodeError::Malformed("chunk precedes IHDR".to_owned()))?;
        let bd = bit_depth.expect("color type implies depth");
        if kind == *b"IEND" {
            if !data.is_empty() || !seen_idat && !saw_fdat && !saw_actl {
                return Err(PngDecodeError::Malformed(
                    "IEND is empty and follows image data".to_owned(),
                ));
            }
            saw_iend = true;
            continue;
        }
        if saw_iend {
            return Err(PngDecodeError::Malformed(
                "trailing data follows IEND".to_owned(),
            ));
        }
        let ancillary_bytes = if kind != *b"IDAT" && kind != *b"fdAT" {
            length_u64
        } else {
            0
        };
        metadata_bytes = metadata_bytes
            .checked_add(ancillary_bytes)
            .ok_or_else(arithmetic)?;
        if metadata_bytes > limits.max_metadata_bytes {
            return Err(PngDecodeError::Limit {
                kind: "metadata bytes",
                actual: metadata_bytes,
                limit: limits.max_metadata_bytes,
            });
        }

        if kind == *b"IDAT" {
            if idat_done {
                return Err(PngDecodeError::Malformed(
                    "IDAT chunks must be consecutive".to_owned(),
                ));
            }
            if saw_fdat {
                return Err(PngDecodeError::Malformed(
                    "IDAT cannot follow APNG frame data".to_owned(),
                ));
            }
            if matches!(ct, PngColorType::Indexed) && palette.is_none() {
                return Err(PngDecodeError::Malformed(
                    "indexed PNG is missing PLTE before IDAT".to_owned(),
                ));
            }
            seen_idat = true;
            default_image = true;
            chunks.compressed_data_bytes = chunks
                .compressed_data_bytes
                .checked_add(length_u64)
                .ok_or_else(arithmetic)?;
            if chunks.compressed_data_bytes > limits.max_compressed_bytes {
                return Err(PngDecodeError::Limit {
                    kind: "compressed bytes",
                    actual: chunks.compressed_data_bytes,
                    limit: limits.max_compressed_bytes,
                });
            }
            continue;
        }
        if seen_idat {
            idat_done = true;
        }
        // Keep accepting late pHYs metadata emitted by established image tools.
        // It cannot affect pixel decoding, and `idat_done` still prevents image
        // data from resuming after the ancillary chunk.
        if seen_idat && kind != *b"fcTL" && kind != *b"fdAT" && kind != *b"pHYs" {
            return Err(PngDecodeError::Malformed(format!(
                "{} is not allowed after IDAT",
                ascii_kind(kind)
            )));
        }
        if kind == *b"PLTE" {
            if singleton.insert(kind) == false {
                return Err(PngDecodeError::Malformed("duplicate PLTE".to_owned()));
            }
            if data.is_empty()
                || data.len() > 768
                || data.len() % 3 != 0
                || matches!(ct, PngColorType::Grayscale | PngColorType::GrayscaleAlpha)
            {
                return Err(PngDecodeError::Malformed("invalid PLTE chunk".to_owned()));
            }
            if matches!(ct, PngColorType::Indexed) && data.len() / 3 > (1_usize << bd.bits()) {
                return Err(PngDecodeError::Malformed(
                    "palette has more entries than the indexed bit depth permits".to_owned(),
                ));
            }
            palette = Some(data.to_vec());
        } else if kind == *b"tRNS" {
            if seen_idat {
                return Err(PngDecodeError::Malformed(
                    "tRNS must precede IDAT".to_owned(),
                ));
            }
            if singleton.insert(kind) == false {
                return Err(PngDecodeError::Malformed("duplicate tRNS".to_owned()));
            }
            match ct {
                PngColorType::Indexed => {
                    let entries = palette
                        .as_ref()
                        .ok_or_else(|| {
                            PngDecodeError::Malformed("indexed tRNS must follow PLTE".to_owned())
                        })?
                        .len()
                        / 3;
                    if data.len() > entries {
                        return Err(PngDecodeError::Malformed(
                            "tRNS has more entries than PLTE".to_owned(),
                        ));
                    }
                }
                PngColorType::Grayscale => {
                    if data.len() != 2 {
                        return Err(PngDecodeError::Malformed(
                            "grayscale tRNS must be two bytes".to_owned(),
                        ));
                    }
                    let value = u16::from_be_bytes([data[0], data[1]]);
                    if bd.bits() < 16 && value >= (1_u16 << bd.bits()) {
                        return Err(PngDecodeError::Malformed(
                            "grayscale tRNS sample exceeds bit depth".to_owned(),
                        ));
                    }
                }
                PngColorType::Rgb => {
                    if data.len() != 6 {
                        return Err(PngDecodeError::Malformed(
                            "RGB tRNS must be six bytes".to_owned(),
                        ));
                    }
                    if bd.bits() == 8
                        && (u16::from_be_bytes([data[0], data[1]]) > 255
                            || u16::from_be_bytes([data[2], data[3]]) > 255
                            || u16::from_be_bytes([data[4], data[5]]) > 255)
                    {
                        return Err(PngDecodeError::Malformed(
                            "RGB tRNS sample exceeds bit depth".to_owned(),
                        ));
                    }
                }
                PngColorType::GrayscaleAlpha | PngColorType::Rgba => {
                    return Err(PngDecodeError::Malformed(
                        "tRNS is forbidden for alpha color types".to_owned(),
                    ));
                }
            }
            transparency = Some(data.to_vec());
        } else if kind == *b"gAMA" {
            check_singleton(&mut singleton, kind)?;
            if data.len() != 4 {
                return Err(PngDecodeError::Malformed(
                    "gAMA must be four bytes".to_owned(),
                ));
            }
            let value = u32::from_be_bytes(data.try_into().expect("gAMA"));
            if value == 0 {
                return Err(PngDecodeError::Malformed("gAMA must be nonzero".to_owned()));
            }
            metadata.gamma = Some(value);
        } else if kind == *b"cHRM" {
            check_singleton(&mut singleton, kind)?;
            if data.len() != 32 {
                return Err(PngDecodeError::Malformed(
                    "cHRM must be 32 bytes".to_owned(),
                ));
            }
            metadata.chromaticities = Some(std::array::from_fn(|index| {
                u32::from_be_bytes(data[index * 4..index * 4 + 4].try_into().expect("cHRM"))
            }));
        } else if kind == *b"sRGB" {
            check_singleton(&mut singleton, kind)?;
            if data.len() != 1 || data[0] > 3 {
                return Err(PngDecodeError::Malformed(
                    "invalid sRGB rendering intent".to_owned(),
                ));
            }
            srgb_intent = Some(data[0]);
            metadata.srgb_intent = srgb_intent;
        } else if kind == *b"iCCP" {
            check_singleton(&mut singleton, kind)?;
            let (keyword, compressed) = split_nul(data, "iCCP")?;
            if compressed.first() != Some(&0) || compressed.len() < 2 {
                return Err(PngDecodeError::Malformed(
                    "invalid iCCP compression".to_owned(),
                ));
            }
            let profile = bounded_zlib(&compressed[1..], limits.max_metadata_bytes)?;
            if profile.is_empty() {
                return Err(PngDecodeError::Malformed("empty iCCP profile".to_owned()));
            }
            let source_color = crate::source_color::embedded_icc(&profile).map_err(|error| {
                PngDecodeError::Malformed(format!("invalid embedded ICC profile: {error}"))
            })?;
            let id = source_color
                .profile()
                .expect("embedded ICC has an identity");
            let inventory = PngProfileInventory {
                bytes: u64::try_from(profile.len()).map_err(|_| arithmetic())?,
                sha256: Sha256::digest(&profile).into(),
                profile_id: id,
                data: profile,
            };
            metadata.icc_profile = Some(inventory.clone());
            icc_profile = Some(inventory);
            let _ = keyword;
        } else if kind == *b"pHYs" {
            check_singleton(&mut singleton, kind)?;
            if data.len() != 9 || data[8] > 1 {
                return Err(PngDecodeError::Malformed("invalid pHYs chunk".to_owned()));
            }
            metadata.physical_resolution = Some(PngPhysicalResolution {
                x_pixels_per_unit: u32::from_be_bytes(data[0..4].try_into().expect("pHYs")),
                y_pixels_per_unit: u32::from_be_bytes(data[4..8].try_into().expect("pHYs")),
                unit_is_meter: data[8] == 1,
            });
        } else if kind == *b"tEXt" || kind == *b"zTXt" || kind == *b"iTXt" {
            let (keyword, text_data) = split_nul(data, "text")?;
            let compressed =
                kind == *b"zTXt" || (kind == *b"iTXt" && text_data.first() == Some(&1));
            if kind == *b"zTXt" && (text_data.first() != Some(&0) || text_data.len() < 2) {
                return Err(PngDecodeError::Malformed(
                    "invalid zTXt compression".to_owned(),
                ));
            }
            if kind == *b"iTXt" && text_data.len() < 2 {
                return Err(PngDecodeError::Malformed("invalid iTXt fields".to_owned()));
            }
            let keyword = String::from_utf8(keyword.to_vec()).map_err(|_| {
                PngDecodeError::Malformed("PNG text keyword is not UTF-8/ASCII".to_owned())
            })?;
            let digest = Sha256::digest(data).into();
            if keyword == "XML:com.adobe.xmp" {
                metadata.xmp_chunks = metadata.xmp_chunks.checked_add(1).ok_or_else(arithmetic)?;
            }
            metadata.text.push(PngTextInventory {
                kind,
                keyword,
                compressed,
                bytes: u64::try_from(data.len()).map_err(|_| arithmetic())?,
                sha256: digest,
            });
        } else if kind == *b"eXIf" {
            check_singleton(&mut singleton, kind)?;
            metadata.exif_bytes = u64::try_from(data.len()).map_err(|_| arithmetic())?;
            metadata.exif_sha256 = Some(Sha256::digest(data).into());
        } else if kind == *b"acTL" {
            if seen_idat {
                return Err(PngDecodeError::Malformed(
                    "acTL must precede IDAT".to_owned(),
                ));
            }
            check_singleton(&mut singleton, kind)?;
            if data.len() != 8 || u32::from_be_bytes(data[0..4].try_into().expect("acTL")) == 0 {
                return Err(PngDecodeError::Malformed("invalid acTL chunk".to_owned()));
            }
            saw_actl = true;
            animation = Some(PngAnimation {
                frame_count: u32::from_be_bytes(data[0..4].try_into().expect("acTL")),
                play_count: u32::from_be_bytes(data[4..8].try_into().expect("acTL")),
                has_default_image: false,
            });
        } else if kind == *b"fcTL" {
            if !saw_actl || data.len() != 26 {
                return Err(PngDecodeError::Malformed(
                    "invalid fcTL sequence".to_owned(),
                ));
            }
            check_sequence(data, &mut next_sequence)?;
            if u32::from_be_bytes(data[4..8].try_into().expect("fcTL")) == 0
                || u32::from_be_bytes(data[8..12].try_into().expect("fcTL")) == 0
                || data[24] > 1
                || data[25] > 1
            {
                return Err(PngDecodeError::Malformed(
                    "invalid APNG frame control".to_owned(),
                ));
            }
            let width = u32::from_be_bytes(data[4..8].try_into().expect("fcTL"));
            let height = u32::from_be_bytes(data[8..12].try_into().expect("fcTL"));
            let x = u32::from_be_bytes(data[12..16].try_into().expect("fcTL"));
            let y = u32::from_be_bytes(data[16..20].try_into().expect("fcTL"));
            let dims = dimensions.expect("IHDR precedes fcTL");
            if x.checked_add(width)
                .is_none_or(|right| right > dims.width())
                || y.checked_add(height)
                    .is_none_or(|bottom| bottom > dims.height())
            {
                return Err(PngDecodeError::Malformed(
                    "APNG frame exceeds canvas".to_owned(),
                ));
            }
            saw_fctl = true;
        } else if kind == *b"fdAT" {
            if !saw_actl || !saw_fctl || data.len() < 4 {
                return Err(PngDecodeError::Malformed(
                    "invalid fdAT sequence".to_owned(),
                ));
            }
            check_sequence(&data[..4], &mut next_sequence)?;
            saw_fdat = true;
            let compressed = u64::try_from(data.len() - 4).map_err(|_| arithmetic())?;
            chunks.compressed_data_bytes = chunks
                .compressed_data_bytes
                .checked_add(compressed)
                .ok_or_else(arithmetic)?;
        } else if kind == *b"IDAT" {
            // handled above
        } else if kind == *b"PLTE" || kind == *b"tRNS" {
            // handled above
        } else if kind[0].is_ascii_uppercase() {
            return Err(PngDecodeError::Malformed(format!(
                "unknown critical chunk {}",
                ascii_kind(kind)
            )));
        }
    }

    let dimensions =
        dimensions.ok_or_else(|| PngDecodeError::Malformed("missing IHDR".to_owned()))?;
    let ct = color_type.expect("dimensions implies color type");
    let bd = bit_depth.expect("dimensions implies bit depth");
    if complete && !saw_iend {
        return Err(PngDecodeError::Malformed("missing IEND".to_owned()));
    }
    if complete && !seen_idat && !saw_fdat && !saw_actl {
        return Err(PngDecodeError::Malformed("missing image data".to_owned()));
    }
    if matches!(ct, PngColorType::Indexed) && palette.is_none() && (complete || seen_idat) {
        return Err(PngDecodeError::Malformed(
            "indexed PNG is missing PLTE".to_owned(),
        ));
    }
    if let Some(palette) = &palette {
        if matches!(ct, PngColorType::Indexed)
            && transparency
                .as_ref()
                .is_some_and(|trns| trns.len() > palette.len() / 3)
        {
            return Err(PngDecodeError::Malformed(
                "indexed transparency exceeds palette".to_owned(),
            ));
        }
    }
    if chunks.compressed_data_bytes > limits.max_compressed_bytes {
        return Err(PngDecodeError::Limit {
            kind: "compressed bytes",
            actual: chunks.compressed_data_bytes,
            limit: limits.max_compressed_bytes,
        });
    }
    if saw_actl && !default_image && complete {
        return Err(PngDecodeError::UnsupportedAnimation);
    }
    if let Some(value) = animation.as_mut() {
        value.has_default_image = default_image;
    }
    if srgb_intent.is_some() && icc_profile.is_some() {
        return Err(PngDecodeError::Malformed(
            "sRGB and iCCP profiles are mutually exclusive".to_owned(),
        ));
    }
    let encoding = if icc_profile.is_some() {
        ColorEncoding::External(icc_profile.expect("profile exists").profile_id)
    } else if srgb_intent.is_some() {
        ColorEncoding::Srgb
    } else {
        ColorEncoding::Unspecified
    };
    let output_bytes = output_bytes(
        dimensions,
        ct,
        bd,
        transparency.is_some(),
        palette.as_deref(),
    )?;
    if output_bytes > limits.max_decoded_bytes {
        return Err(PngDecodeError::Limit {
            kind: "decoded bytes",
            actual: output_bytes,
            limit: limits.max_decoded_bytes,
        });
    }
    let decompressed =
        raw_scanline_bytes(dimensions.width(), dimensions.height(), ct, bd, interlaced)?;
    chunks.decompressed_data_bytes = decompressed;
    if decompressed > limits.max_decompressed_bytes {
        return Err(PngDecodeError::Limit {
            kind: "decompressed bytes",
            actual: decompressed,
            limit: limits.max_decompressed_bytes,
        });
    }
    let has_palette = palette.is_some();
    let has_transparency = transparency.is_some();
    Ok(ParsedPng {
        dimensions,
        palette,
        transparency: transparency.clone(),
        header: PngHeader {
            dimensions,
            color_type: ct,
            bit_depth: bd,
            interlaced,
            has_palette,
            has_transparency,
            chunks,
            metadata,
            animation,
            color_encoding: encoding,
        },
    })
}

pub(crate) fn common_limits(limits: DecodeLimits) -> PngDecodeLimits {
    PngDecodeLimits::from_common(limits)
}

fn decode_color_type(value: u8) -> Result<PngColorType, PngDecodeError> {
    match value {
        0 => Ok(PngColorType::Grayscale),
        2 => Ok(PngColorType::Rgb),
        3 => Ok(PngColorType::Indexed),
        4 => Ok(PngColorType::GrayscaleAlpha),
        6 => Ok(PngColorType::Rgba),
        _ => Err(PngDecodeError::Malformed(
            "unsupported PNG color type".to_owned(),
        )),
    }
}
fn decode_bit_depth(value: u8, color: PngColorType) -> Result<PngBitDepth, PngDecodeError> {
    let depth = match value {
        1 => PngBitDepth::One,
        2 => PngBitDepth::Two,
        4 => PngBitDepth::Four,
        8 => PngBitDepth::Eight,
        16 => PngBitDepth::Sixteen,
        _ => {
            return Err(PngDecodeError::Malformed(
                "unsupported PNG bit depth".to_owned(),
            ));
        }
    };
    if matches!(
        color,
        PngColorType::Rgb | PngColorType::GrayscaleAlpha | PngColorType::Rgba
    ) && value < 8
        || matches!(color, PngColorType::Indexed) && value == 16
    {
        return Err(PngDecodeError::Malformed(
            "illegal PNG color type/bit depth combination".to_owned(),
        ));
    }
    Ok(depth)
}
fn enforce_dimensions(
    dimensions: ImageDimensions,
    limits: PngDecodeLimits,
) -> Result<(), PngDecodeError> {
    if dimensions.width() > limits.max_width {
        return Err(PngDecodeError::Limit {
            kind: "width",
            actual: u64::from(dimensions.width()),
            limit: u64::from(limits.max_width),
        });
    }
    if dimensions.height() > limits.max_height {
        return Err(PngDecodeError::Limit {
            kind: "height",
            actual: u64::from(dimensions.height()),
            limit: u64::from(limits.max_height),
        });
    }
    let pixels = dimensions.pixel_count().map_err(|_| arithmetic())?;
    if pixels > limits.max_pixels {
        return Err(PngDecodeError::Limit {
            kind: "pixels",
            actual: pixels,
            limit: limits.max_pixels,
        });
    }
    Ok(())
}
fn output_bytes(
    dimensions: ImageDimensions,
    color: PngColorType,
    depth: PngBitDepth,
    transparency: bool,
    palette: Option<&[u8]>,
) -> Result<u64, PngDecodeError> {
    let channels: u64 = match color {
        PngColorType::Indexed => {
            if transparency {
                4
            } else {
                3
            }
        }
        PngColorType::Grayscale => {
            if transparency {
                2
            } else {
                1
            }
        }
        PngColorType::GrayscaleAlpha => 2,
        PngColorType::Rgb => {
            if transparency {
                4
            } else {
                3
            }
        }
        PngColorType::Rgba => 4,
    };
    let bytes_per_sample = if matches!(color, PngColorType::Indexed) || depth.bits() < 8 {
        1
    } else {
        u64::from(depth.bits() / 8)
    };
    let value = dimensions
        .pixel_count()
        .map_err(|_| arithmetic())?
        .checked_mul(channels)
        .and_then(|value| value.checked_mul(bytes_per_sample))
        .ok_or_else(arithmetic)?;
    if matches!(color, PngColorType::Indexed) && palette.is_none() && transparency {
        return Err(PngDecodeError::Malformed(
            "indexed transparency has no palette".to_owned(),
        ));
    }
    Ok(value)
}
fn raw_scanline_bytes(
    width: u32,
    height: u32,
    color: PngColorType,
    depth: PngBitDepth,
    interlaced: bool,
) -> Result<u64, PngDecodeError> {
    let samples = u64::from(color.channels());
    let row = |w: u32| -> Result<u64, PngDecodeError> {
        let bits = u64::from(w)
            .checked_mul(samples)
            .and_then(|value| value.checked_mul(u64::from(depth.bits())))
            .ok_or_else(arithmetic)?;
        Ok(1 + bits.div_ceil(8))
    };
    if !interlaced {
        return row(width)?
            .checked_mul(u64::from(height))
            .ok_or_else(arithmetic);
    }
    let x_starts = [0_u32, 4, 0, 2, 0, 1, 0];
    let y_starts = [0_u32, 0, 4, 0, 2, 0, 1];
    let x_steps = [8_u32, 8, 4, 4, 2, 2, 1];
    let y_steps = [8_u32, 8, 8, 4, 4, 2, 2];
    let mut total = 0_u64;
    for pass in 0..7 {
        let pw = width.saturating_sub(x_starts[pass]).div_ceil(x_steps[pass]);
        let ph = height
            .saturating_sub(y_starts[pass])
            .div_ceil(y_steps[pass]);
        if pw == 0 || ph == 0 {
            continue;
        }
        total = total
            .checked_add(row(pw)?.checked_mul(u64::from(ph)).ok_or_else(arithmetic)?)
            .ok_or_else(arithmetic)?;
    }
    Ok(total)
}
fn validate_chunk_kind(kind: [u8; 4]) -> Result<(), PngDecodeError> {
    if !kind.iter().all(u8::is_ascii_alphabetic) || !kind[2].is_ascii_uppercase() {
        return Err(PngDecodeError::Malformed(
            "invalid PNG chunk type".to_owned(),
        ));
    }
    Ok(())
}
fn check_singleton(
    singleton: &mut std::collections::BTreeSet<[u8; 4]>,
    kind: [u8; 4],
) -> Result<(), PngDecodeError> {
    if !singleton.insert(kind) {
        Err(PngDecodeError::Malformed(format!(
            "duplicate {} chunk",
            ascii_kind(kind)
        )))
    } else {
        Ok(())
    }
}
fn check_sequence(data: &[u8], next: &mut u32) -> Result<(), PngDecodeError> {
    let sequence = u32::from_be_bytes(data[0..4].try_into().expect("sequence"));
    if sequence != *next {
        return Err(PngDecodeError::Malformed(
            "APNG sequence number is not monotonic".to_owned(),
        ));
    }
    *next = next.checked_add(1).ok_or_else(arithmetic)?;
    Ok(())
}
fn split_nul<'a>(data: &'a [u8], label: &str) -> Result<(&'a [u8], &'a [u8]), PngDecodeError> {
    let index = data
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| PngDecodeError::Malformed(format!("{label} is missing a separator")))?;
    if index == 0 {
        return Err(PngDecodeError::Malformed(format!(
            "{label} keyword is empty"
        )));
    }
    Ok((&data[..index], &data[index + 1..]))
}
fn bounded_zlib(data: &[u8], limit: u64) -> Result<Vec<u8>, PngDecodeError> {
    let max = usize::try_from(limit).map_err(|_| arithmetic())?;
    let decoder = ZlibDecoder::new(data);
    let mut output = Vec::new();
    output
        .try_reserve(max.min(64 * 1024))
        .map_err(|_| PngDecodeError::Limit {
            kind: "profile allocation",
            actual: limit.saturating_add(1),
            limit,
        })?;
    decoder
        .take(limit.saturating_add(1))
        .read_to_end(&mut output)
        .map_err(|error| {
            PngDecodeError::Malformed(format!("profile decompression failed: {error}"))
        })?;
    if u64::try_from(output.len()).unwrap_or(u64::MAX) > limit {
        return Err(PngDecodeError::Limit {
            kind: "profile decompressed bytes",
            actual: limit.saturating_add(1),
            limit,
        });
    }
    Ok(output)
}
fn ascii_kind(kind: [u8; 4]) -> String {
    String::from_utf8_lossy(&kind).into_owned()
}
fn arithmetic() -> PngDecodeError {
    PngDecodeError::Input(ImageInputError::ArithmeticOverflow)
}

fn crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for byte in kind.into_iter().chain(data.iter().copied()) {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
