//! Camera-RAW dispatch and bounded display demosaicing.

use std::io::Cursor;

use rawler::rawsource::RawSource;
use rawler::{RawImage as RawlerImage, RawImageData as RawlerImageData};
use rawloader::{RawImage as RawLoaderImage, RawImageData as RawLoaderImageData};
use rusttable_image::{
    DecodeLimits, DecodedImage, ImageDimensions, ImageInputError, ImageProbe, InputFormat,
};

/// Returns whether the bounded header has a camera-RAW container signature.
pub(crate) fn is_raw(bytes: &[u8]) -> bool {
    if has_ascii(bytes, b"RustTable-DNG-v1") {
        return false;
    }
    bytes.starts_with(b"FUJIFILMCCD-RAW")
        || bytes.starts_with(b"Fujifilm")
        || has_ascii(bytes, b"NIKON")
        || has_ascii(bytes, b"SONY")
        || has_ascii(bytes, b"Panasonic")
        || has_ascii(bytes, b"OLYMPUS")
        || has_ascii(bytes, b"Canon")
        || has_dng_version_tag(bytes)
}

pub(crate) fn probe_raw(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    let image = decode_dummy(bytes)?;
    let dimensions = rendered_dimensions(&image)?;
    enforce_limits(limits, dimensions)?;
    Ok(ImageProbe::new(InputFormat::Raw, dimensions))
}

pub(crate) fn decode_raw(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let image = decode_full(bytes)?;
    let dimensions = rendered_dimensions(&image)?;
    enforce_limits(limits, dimensions)?;
    let output_len = checked_rgba_len(dimensions)?;
    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(output_len)
        .map_err(|_| ImageInputError::AllocationFailure)?;
    pixels.extend(demosaic(&image, dimensions));
    DecodedImage::new_with_color_encoding(dimensions, pixels, rusttable_image::ColorEncoding::Srgb)
        .map_err(|error| match error {
            rusttable_image::DecodedImageError::ArithmeticOverflow => {
                ImageInputError::ArithmeticOverflow
            }
            rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
                ImageInputError::DecodedBufferInvariant { expected, actual }
            }
        })
}

fn decode_dummy(bytes: &[u8]) -> Result<DecodedRaw, ImageInputError> {
    if is_fujifilm_raf(bytes) {
        rawler::decode_dummy(&RawSource::new_from_slice(bytes))
            .map(convert_rawler_image)
            .map_err(|error| rawler_error(&error))
    } else {
        rawloader::decode_dummy(&mut Cursor::new(bytes))
            .map(convert_rawloader_image)
            .map_err(|error| raw_error(&error))
    }
}

fn decode_full(bytes: &[u8]) -> Result<DecodedRaw, ImageInputError> {
    if is_fujifilm_raf(bytes) {
        rawler::decode(
            &RawSource::new_from_slice(bytes),
            &rawler::decoders::RawDecodeParams::default(),
        )
        .map(convert_rawler_image)
        .map_err(|error| rawler_error(&error))
    } else {
        rawloader::decode(&mut Cursor::new(bytes))
            .map(convert_rawloader_image)
            .map_err(|error| raw_error(&error))
    }
}

fn raw_error(error: &rawloader::RawLoaderError) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Raw,
        message: error.to_string(),
    }
}

fn rawler_error(error: &rawler::RawlerError) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Raw,
        message: error.to_string(),
    }
}

fn is_fujifilm_raf(bytes: &[u8]) -> bool {
    bytes.starts_with(b"FUJIFILMCCD-RAW") || bytes.starts_with(b"Fujifilm")
}

struct DecodedRaw {
    width: usize,
    height: usize,
    cpp: usize,
    wb_coeffs: [f32; 4],
    blacklevels: [f32; 4],
    whitelevels: [f32; 4],
    cfa: CfaPattern,
    orientation: (bool, bool, bool),
    data: RawData,
}

enum RawData {
    Integer(Vec<u16>),
    Float(Vec<f32>),
}

struct CfaPattern {
    width: usize,
    height: usize,
    colors: Vec<usize>,
}

impl CfaPattern {
    fn from_rawloader(cfa: &rawloader::CFA) -> Self {
        Self {
            width: cfa.width,
            height: cfa.height,
            colors: (0..cfa.height)
                .flat_map(|row| (0..cfa.width).map(move |column| cfa.color_at(row, column)))
                .collect(),
        }
    }

    fn from_rawler(cfa: &rawler::CFA) -> Self {
        Self {
            width: cfa.width,
            height: cfa.height,
            colors: (0..cfa.height)
                .flat_map(|row| (0..cfa.width).map(move |column| cfa.color_at(row, column)))
                .collect(),
        }
    }

    fn color_at(&self, row: usize, column: usize) -> usize {
        if self.width == 0 || self.height == 0 {
            return 0;
        }
        self.colors[(row % self.height) * self.width + (column % self.width)]
    }
}

fn convert_rawloader_image(image: RawLoaderImage) -> DecodedRaw {
    let RawLoaderImage {
        width,
        height,
        cpp,
        wb_coeffs,
        whitelevels,
        blacklevels,
        cfa,
        orientation,
        data,
        ..
    } = image;
    DecodedRaw {
        width,
        height,
        cpp,
        wb_coeffs,
        blacklevels: blacklevels.map(f32::from),
        whitelevels: whitelevels.map(f32::from),
        cfa: CfaPattern::from_rawloader(&cfa),
        orientation: orientation.to_flips(),
        data: match data {
            RawLoaderImageData::Integer(data) => RawData::Integer(data),
            RawLoaderImageData::Float(data) => RawData::Float(data),
        },
    }
}

fn convert_rawler_image(image: RawlerImage) -> DecodedRaw {
    let RawlerImage {
        camera,
        width,
        height,
        cpp,
        wb_coeffs,
        whitelevel,
        blacklevel,
        orientation,
        data,
        ..
    } = image;
    DecodedRaw {
        width,
        height,
        cpp,
        wb_coeffs,
        blacklevels: blacklevel.as_bayer_array(),
        whitelevels: whitelevel.as_bayer_array(),
        cfa: CfaPattern::from_rawler(&camera.cfa),
        orientation: orientation.to_flips(),
        data: match data {
            RawlerImageData::Integer(data) => RawData::Integer(data),
            RawlerImageData::Float(data) => RawData::Float(data),
        },
    }
}

fn rendered_dimensions(image: &DecodedRaw) -> Result<ImageDimensions, ImageInputError> {
    let width = u32::try_from(image.width).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let height = u32::try_from(image.height).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let (transpose, _, _) = image.orientation;
    ImageDimensions::new(
        if transpose { height } else { width },
        if transpose { width } else { height },
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn checked_rgba_len(dimensions: ImageDimensions) -> Result<usize, ImageInputError> {
    usize::try_from(
        dimensions
            .decoded_byte_count()
            .map_err(|_| ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn enforce_limits(
    limits: DecodeLimits,
    dimensions: ImageDimensions,
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
    let decoded = dimensions
        .decoded_byte_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if decoded > limits.max_decoded_bytes() {
        return Err(ImageInputError::DecodedByteLimit {
            actual: decoded,
            limit: limits.max_decoded_bytes(),
        });
    }
    Ok(())
}

fn demosaic(image: &DecodedRaw, dimensions: ImageDimensions) -> Vec<u8> {
    let source_width = image.width;
    let source_height = image.height;
    let (transpose, horizontal, vertical) = image.orientation;
    let data = match &image.data {
        RawData::Integer(data) => data,
        RawData::Float(data) => return demosaic_float(image, dimensions, data),
    };
    let mut output =
        Vec::with_capacity(dimensions.width() as usize * dimensions.height() as usize * 4);
    for output_y in 0..dimensions.height() as usize {
        for output_x in 0..dimensions.width() as usize {
            let (source_x, source_y) = source_coordinate(
                output_x,
                output_y,
                source_width,
                source_height,
                transpose,
                horizontal,
                vertical,
            );
            for channel in 0..3 {
                let sample = if image.cpp == 1 {
                    mosaic_sample(image, data, source_x, source_y, channel)
                } else {
                    interleaved_sample(image, data, source_x, source_y, channel)
                };
                output.push(normalize_integer(sample, image, channel));
            }
            output.push(255);
        }
    }
    output
}

fn demosaic_float(image: &DecodedRaw, dimensions: ImageDimensions, data: &[f32]) -> Vec<u8> {
    let source_width = image.width;
    let source_height = image.height;
    let (transpose, horizontal, vertical) = image.orientation;
    let mut output =
        Vec::with_capacity(dimensions.width() as usize * dimensions.height() as usize * 4);
    for output_y in 0..dimensions.height() as usize {
        for output_x in 0..dimensions.width() as usize {
            let (source_x, source_y) = source_coordinate(
                output_x,
                output_y,
                source_width,
                source_height,
                transpose,
                horizontal,
                vertical,
            );
            for channel in 0..3 {
                let index = (source_y * source_width + source_x) * image.cpp
                    + channel.min(image.cpp.saturating_sub(1));
                let value = data.get(index).copied().unwrap_or_default();
                output.push(to_u8(value));
            }
            output.push(255);
        }
    }
    output
}

fn source_coordinate(
    output_x: usize,
    output_y: usize,
    width: usize,
    height: usize,
    transpose: bool,
    horizontal: bool,
    vertical: bool,
) -> (usize, usize) {
    let (x, y) = if transpose {
        (output_y, output_x)
    } else {
        (output_x, output_y)
    };
    (
        if horizontal { width - 1 - x } else { x },
        if vertical { height - 1 - y } else { y },
    )
}

fn interleaved_sample(image: &DecodedRaw, data: &[u16], x: usize, y: usize, channel: usize) -> u16 {
    let index = (y * image.width + x) * image.cpp + channel.min(image.cpp.saturating_sub(1));
    data.get(index).copied().unwrap_or_default()
}

fn mosaic_sample(image: &DecodedRaw, data: &[u16], x: usize, y: usize, channel: usize) -> u16 {
    let radius = image.cfa.width.max(image.cfa.height).max(2);
    let mut total = 0_u64;
    let mut count = 0_u64;
    for distance in 0..=radius {
        for candidate_y in y.saturating_sub(distance)..=(y + distance).min(image.height - 1) {
            for candidate_x in x.saturating_sub(distance)..=(x + distance).min(image.width - 1) {
                if image.cfa.color_at(candidate_y, candidate_x).min(2) != channel {
                    continue;
                }
                let index = candidate_y * image.width + candidate_x;
                total += u64::from(data.get(index).copied().unwrap_or_default());
                count += 1;
            }
        }
        if count != 0 {
            break;
        }
    }
    u16::try_from(total / count.max(1)).unwrap_or(u16::MAX)
}

fn normalize_integer(value: u16, image: &DecodedRaw, channel: usize) -> u8 {
    let black = image.blacklevels[channel];
    let white = image.whitelevels[channel].max(black + 1.0);
    let normalized = (f32::from(value) - black).max(0.0) / (white - black);
    let gain = image.wb_coeffs[channel];
    let balanced = if gain.is_finite() && gain > 0.0 {
        normalized * gain
    } else {
        normalized
    };
    to_u8(balanced.clamp(0.0, 1.0).powf(1.0 / 2.2))
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn to_u8(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn has_ascii(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|window| window == needle)
}

fn has_dng_version_tag(bytes: &[u8]) -> bool {
    let Some(order) = bytes.get(0..2) else {
        return false;
    };
    let little = order == b"II";
    if !little && order != b"MM" {
        return false;
    }
    let Some(magic) = read_u16(bytes, 2, little) else {
        return false;
    };
    if magic != 42 {
        return false;
    }
    let Some(ifd_offset) = read_u32(bytes, 4, little) else {
        return false;
    };
    let Some(count) = read_u16(bytes, ifd_offset as usize, little) else {
        return false;
    };
    (0..usize::from(count)).any(|index| {
        let offset = ifd_offset as usize + 2 + index * 12;
        read_u16(bytes, offset, little) == Some(50706)
    })
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
        u32::from_le_bytes(value.try_into().ok()?)
    } else {
        u32::from_be_bytes(value.try_into().ok()?)
    })
}
