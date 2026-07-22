#![allow(clippy::all, clippy::pedantic)]

use std::io::{BufReader, Cursor};
use std::panic::{AssertUnwindSafe, catch_unwind};

use png::{BitDepth, ColorType, Decoder, Limits, Transformations};
use rusttable_image::{DecodeLimits, ImageInputError, ImageProbe, InputFormat};
use sha2::{Digest, Sha256};

use super::parser::{ParsedPng, common_limits, parse};
use super::types::{
    PNG_BACKEND_ID, PngBitDepth, PngColorType, PngDecodeError, PngDecodeLimits, PngDecodeMode,
    PngDecodeReceipt, PngDecodeRequest, PngDecodeResult, PngHeader, PngPixelData,
};
use crate::raw::{RawByteSource, RawSourceError, SliceRawSource};

const COPY_CHUNK_BYTES: usize = 64 * 1024;

/// Strict bounded PNG decoder.
#[derive(Debug, Clone, Copy, Default)]
pub struct PngDecoder;

impl PngDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Probes a PNG header and validates every complete chunk available to the probe.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: PngDecodeLimits,
    ) -> Result<PngHeader, PngDecodeError> {
        Ok(parse(bytes, limits, false)?.header)
    }

    /// Fully validates PNG structure, CRCs, ordering, inventories, and limits.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: PngDecodeLimits,
    ) -> Result<PngHeader, PngDecodeError> {
        Ok(parse(bytes, limits, true)?.header)
    }

    /// Decodes an immutable byte slice without publishing partial output.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &PngDecodeRequest,
    ) -> Result<PngDecodeResult, PngDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Copies a bounded random-access source and rejects source mutation before decoding.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &PngDecodeRequest,
    ) -> Result<PngDecodeResult, PngDecodeError> {
        check_cancel(request)?;
        if matches!(request.mode, PngDecodeMode::Region { .. }) {
            return Err(PngDecodeError::UnsupportedRegion);
        }
        let snapshot = Snapshot::read(source, request)?;
        check_cancel(request)?;
        let parsed = parse(&snapshot.bytes, request.limits, true)?;
        let header = parsed.header.clone();
        let header_only = matches!(request.mode, PngDecodeMode::Header);
        let pixels = if header_only {
            None
        } else {
            Some(decode_pixels(&snapshot.bytes, &parsed, request.limits)?)
        };
        check_cancel(request)?;
        let image = match (&pixels, header_only) {
            (Some(pixels), false) => Some(pixels.to_owned_image(header.color_encoding)?),
            _ => None,
        };
        let output_bytes = pixels
            .as_ref()
            .map_or(0, |pixels| pixels.sample_bytes().len());
        let receipt = PngDecodeReceipt {
            backend: PNG_BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            output_bytes: u64::try_from(output_bytes)
                .map_err(|_| PngDecodeError::Input(ImageInputError::ArithmeticOverflow))?,
            color_type: header.color_type,
            bit_depth: header.bit_depth,
            interlaced: header.interlaced,
            palette: header.has_palette,
            transparency: header.has_transparency,
            chunk_count: u32::try_from(header.chunks.chunks.len())
                .map_err(|_| PngDecodeError::Input(ImageInputError::ArithmeticOverflow))?,
            decompressed_bytes: header.chunks.decompressed_data_bytes,
            animation: header.animation,
            header_only,
        };
        Ok(PngDecodeResult {
            header,
            pixels,
            image,
            receipt,
        })
    }
}

pub(crate) fn is_png_signature(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

pub(crate) fn decode_png_probe(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    // The registry's historical bounded probe accepts an IHDR-only prefix.  A
    // complete source always takes the strict CRC/order path below.
    if bytes.len() == 33
        && bytes.get(8..12) == Some(&13_u32.to_be_bytes())
        && bytes.get(12..16) == Some(b"IHDR")
    {
        let dimensions = rusttable_image::ImageDimensions::new(
            u32::from_be_bytes(bytes[16..20].try_into().expect("IHDR width")),
            u32::from_be_bytes(bytes[20..24].try_into().expect("IHDR height")),
        )
        .map_err(|_| ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message: "PNG dimensions must be nonzero".to_owned(),
        })?;
        crate::input::enforce_limits(limits, dimensions)?;
        return Ok(ImageProbe::new(InputFormat::Png, dimensions));
    }
    let header = PngDecoder::new()
        .probe_bytes(bytes, common_limits(limits))
        .map_err(map_error)?;
    Ok(ImageProbe::new(InputFormat::Png, header.dimensions))
}

pub(crate) fn decode_legacy_rgba8(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<(rusttable_image::ImageDimensions, Vec<u8>), ImageInputError> {
    let request = PngDecodeRequest::new(common_limits(limits));
    let result = PngDecoder::new()
        .decode_bytes(bytes, &request)
        .map_err(map_error)?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed("PNG full decode returned no pixels"))?;
    let dimensions = pixels.dimensions();
    let rgba = pixels.to_rgba8();
    let expected = dimensions
        .decoded_byte_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if u64::try_from(rgba.len()).unwrap_or(u64::MAX) != expected {
        return Err(ImageInputError::DecodedBufferInvariant {
            expected,
            actual: u64::try_from(rgba.len()).unwrap_or(u64::MAX),
        });
    }
    Ok((dimensions, rgba))
}

fn decode_pixels(
    bytes: &[u8],
    parsed: &ParsedPng,
    limits: PngDecodeLimits,
) -> Result<PngPixelData, PngDecodeError> {
    let max = usize::try_from(limits.max_decompressed_bytes.min(usize::MAX as u64))
        .map_err(|_| PngDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    let mut decoder =
        Decoder::new_with_limits(BufReader::new(Cursor::new(bytes)), Limits { bytes: max });
    decoder.set_transformations(Transformations::IDENTITY);
    let mut reader = catch_unwind(AssertUnwindSafe(|| decoder.read_info()))
        .map_err(|_| {
            PngDecodeError::Backend("decoder panicked while reading validated PNG".to_owned())
        })?
        .map_err(|error| PngDecodeError::Backend(error.to_string()))?;
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| PngDecodeError::Limit {
            kind: "backend buffer bytes",
            actual: limits.max_decoded_bytes.saturating_add(1),
            limit: limits.max_decoded_bytes,
        })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_size)
        .map_err(|_| PngDecodeError::Input(ImageInputError::AllocationFailure))?;
    output.resize(output_size, 0);
    let info = catch_unwind(AssertUnwindSafe(|| reader.next_frame(&mut output)))
        .map_err(|_| {
            PngDecodeError::Backend("decoder panicked while decoding validated PNG".to_owned())
        })?
        .map_err(|error| match error {
            png::DecodingError::LimitsExceeded => PngDecodeError::Limit {
                kind: "backend decompressed bytes",
                actual: limits.max_decompressed_bytes.saturating_add(1),
                limit: limits.max_decompressed_bytes,
            },
            other => PngDecodeError::Backend(other.to_string()),
        })?;
    if info.width != parsed.dimensions.width() || info.height != parsed.dimensions.height() {
        return Err(PngDecodeError::Malformed(
            "decoded dimensions differ from IHDR".to_owned(),
        ));
    }
    let expected_color = match parsed.header.color_type {
        PngColorType::Grayscale => ColorType::Grayscale,
        PngColorType::GrayscaleAlpha => ColorType::GrayscaleAlpha,
        PngColorType::Indexed => ColorType::Indexed,
        PngColorType::Rgb => ColorType::Rgb,
        PngColorType::Rgba => ColorType::Rgba,
    };
    let expected_depth = match parsed.header.bit_depth {
        PngBitDepth::One => BitDepth::One,
        PngBitDepth::Two => BitDepth::Two,
        PngBitDepth::Four => BitDepth::Four,
        PngBitDepth::Eight => BitDepth::Eight,
        PngBitDepth::Sixteen => BitDepth::Sixteen,
    };
    if info.color_type != expected_color || info.bit_depth != expected_depth {
        return Err(PngDecodeError::Malformed(
            "backend changed PNG sample format".to_owned(),
        ));
    }
    convert_samples(&output[..info.buffer_size()], parsed)
}

fn convert_samples(output: &[u8], parsed: &ParsedPng) -> Result<PngPixelData, PngDecodeError> {
    let dimensions = parsed.dimensions;
    let depth = parsed.header.bit_depth.bits();
    match parsed.header.color_type {
        PngColorType::Indexed => {
            let palette = parsed.palette.as_deref().ok_or_else(|| {
                PngDecodeError::Malformed("indexed PNG has no palette".to_owned())
            })?;
            let alpha = parsed.transparency.as_deref();
            let mut samples = Vec::with_capacity(
                dimensions.pixel_count().unwrap_or(0) as usize
                    * if alpha.is_some() { 4 } else { 3 },
            );
            let mut index = 0;
            for _ in 0..dimensions.height() {
                let row_bytes = packed_row_len(dimensions.width(), depth);
                let row = output.get(index..index + row_bytes).ok_or_else(|| {
                    PngDecodeError::Malformed("indexed row is truncated".to_owned())
                })?;
                index += row_bytes;
                for x in 0..dimensions.width() {
                    let palette_index = packed_sample(row, x as usize, depth);
                    let start = usize::from(palette_index)
                        .checked_mul(3)
                        .ok_or_else(arithmetic)?;
                    let rgb = palette.get(start..start + 3).ok_or_else(|| {
                        PngDecodeError::Malformed("palette index is out of range".to_owned())
                    })?;
                    samples.extend_from_slice(rgb);
                    if let Some(alpha) = alpha {
                        samples.push(
                            alpha
                                .get(usize::from(palette_index))
                                .copied()
                                .unwrap_or(255),
                        );
                    }
                }
            }
            if alpha.is_some() {
                Ok(PngPixelData::RgbaU8 {
                    dimensions,
                    samples,
                })
            } else {
                Ok(PngPixelData::RgbU8 {
                    dimensions,
                    samples,
                })
            }
        }
        PngColorType::Grayscale => {
            if depth == 16 {
                let values = be_u16_samples(output)?;
                let trns = transparent_gray(parsed.transparency.as_deref());
                if let Some(value) = trns {
                    let mut samples = Vec::with_capacity(values.len() * 2);
                    for sample in values {
                        samples.extend_from_slice(&[
                            sample,
                            if sample == value { 0 } else { u16::MAX },
                        ]);
                    }
                    Ok(PngPixelData::GrayAU16 {
                        dimensions,
                        samples,
                    })
                } else {
                    Ok(PngPixelData::GrayU16 {
                        dimensions,
                        samples: values,
                    })
                }
            } else if let Some(value) = transparent_gray(parsed.transparency.as_deref()) {
                Ok(PngPixelData::GrayAU8 {
                    dimensions,
                    samples: unpack_gray_with_transparency(output, dimensions, depth, value),
                })
            } else {
                Ok(PngPixelData::GrayU8 {
                    dimensions,
                    samples: unpack_gray(output, dimensions, depth),
                })
            }
        }
        PngColorType::GrayscaleAlpha => {
            if depth == 16 {
                Ok(PngPixelData::GrayAU16 {
                    dimensions,
                    samples: be_u16_samples(output)?,
                })
            } else {
                Ok(PngPixelData::GrayAU8 {
                    dimensions,
                    samples: output.to_vec(),
                })
            }
        }
        PngColorType::Rgb => {
            if depth == 16 {
                let values = be_u16_samples(output)?;
                if let Some(trns) = transparent_rgb(parsed.transparency.as_deref()) {
                    let mut samples = Vec::with_capacity(values.len() / 3 * 4);
                    for pixel in values.chunks_exact(3) {
                        samples.extend_from_slice(pixel);
                        samples.push(if pixel == trns { 0 } else { u16::MAX });
                    }
                    Ok(PngPixelData::RgbaU16 {
                        dimensions,
                        samples,
                    })
                } else {
                    Ok(PngPixelData::RgbU16 {
                        dimensions,
                        samples: values,
                    })
                }
            } else {
                if let Some(trns) = transparent_rgb(parsed.transparency.as_deref()) {
                    let mut samples = Vec::with_capacity(output.len() / 3 * 4);
                    for pixel in output.chunks_exact(3) {
                        samples.extend_from_slice(pixel);
                        samples.push(
                            if pixel
                                == trns
                                    .iter()
                                    .map(|value| *value as u8)
                                    .collect::<Vec<_>>()
                                    .as_slice()
                            {
                                0
                            } else {
                                255
                            },
                        );
                    }
                    Ok(PngPixelData::RgbaU8 {
                        dimensions,
                        samples,
                    })
                } else {
                    Ok(PngPixelData::RgbU8 {
                        dimensions,
                        samples: output.to_vec(),
                    })
                }
            }
        }
        PngColorType::Rgba => {
            if depth == 16 {
                Ok(PngPixelData::RgbaU16 {
                    dimensions,
                    samples: be_u16_samples(output)?,
                })
            } else {
                Ok(PngPixelData::RgbaU8 {
                    dimensions,
                    samples: output.to_vec(),
                })
            }
        }
    }
}

fn packed_row_len(width: u32, depth: u8) -> usize {
    (usize::try_from(u64::from(width) * u64::from(depth)).unwrap_or(usize::MAX) + 7) / 8
}
fn packed_sample(row: &[u8], index: usize, depth: u8) -> u8 {
    if depth == 8 {
        return row[index];
    }
    let bit = index * usize::from(depth);
    let byte = row[bit / 8];
    let shift = 8 - usize::from(depth) - bit % 8;
    (byte >> shift) & ((1_u8 << depth) - 1)
}
fn unpack_gray(output: &[u8], dimensions: rusttable_image::ImageDimensions, depth: u8) -> Vec<u8> {
    if depth == 8 {
        return output.to_vec();
    }
    let mut values = Vec::with_capacity(dimensions.pixel_count().unwrap_or(0) as usize);
    let max = u16::from((1_u16 << depth) - 1);
    let row_len = packed_row_len(dimensions.width(), depth);
    for row in output.chunks_exact(row_len) {
        for x in 0..dimensions.width() {
            let sample = u16::from(packed_sample(row, x as usize, depth));
            values.push(((sample * 255) / max) as u8);
        }
    }
    values
}

fn unpack_gray_with_transparency(
    output: &[u8],
    dimensions: rusttable_image::ImageDimensions,
    depth: u8,
    transparent: u16,
) -> Vec<u8> {
    if depth == 8 {
        let mut samples = Vec::with_capacity(output.len() * 2);
        for &sample in output {
            samples.extend_from_slice(&[
                sample,
                if u16::from(sample) == transparent {
                    0
                } else {
                    255
                },
            ]);
        }
        return samples;
    }
    let mut samples = Vec::with_capacity(dimensions.pixel_count().unwrap_or(0) as usize * 2);
    let max = (1_u16 << depth) - 1;
    let row_len = packed_row_len(dimensions.width(), depth);
    for row in output.chunks_exact(row_len) {
        for x in 0..dimensions.width() {
            let raw = u16::from(packed_sample(row, x as usize, depth));
            samples.extend_from_slice(&[
                u8::try_from(raw * 255 / max).expect("sub-byte sample expands to U8"),
                if raw == transparent { 0 } else { 255 },
            ]);
        }
    }
    samples
}
fn be_u16_samples(output: &[u8]) -> Result<Vec<u16>, PngDecodeError> {
    let (chunks, remainder) = output.as_chunks::<2>();
    if !remainder.is_empty() {
        return Err(PngDecodeError::Malformed(
            "16-bit PNG output has an odd byte length".to_owned(),
        ));
    }
    Ok(chunks
        .iter()
        .map(|bytes| u16::from_be_bytes(*bytes))
        .collect())
}
fn transparent_gray(bytes: Option<&[u8]>) -> Option<u16> {
    bytes
        .and_then(|bytes| bytes.get(..2))
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
}
fn transparent_rgb(bytes: Option<&[u8]>) -> Option<[u16; 3]> {
    bytes.and_then(|bytes| bytes.get(..6)).map(|bytes| {
        [
            u16::from_be_bytes([bytes[0], bytes[1]]),
            u16::from_be_bytes([bytes[2], bytes[3]]),
            u16::from_be_bytes([bytes[4], bytes[5]]),
        ]
    })
}
fn check_cancel(request: &PngDecodeRequest) -> Result<(), PngDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(PngDecodeError::Cancelled)
    } else {
        Ok(())
    }
}

struct Snapshot {
    bytes: Vec<u8>,
    source_bytes: u64,
    sha256: [u8; 32],
}
impl Snapshot {
    fn read<S: RawByteSource + ?Sized>(
        source: &S,
        request: &PngDecodeRequest,
    ) -> Result<Self, PngDecodeError> {
        let length = source.len().map_err(PngDecodeError::Source)?;
        if length == 0 {
            return Err(PngDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(PngDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(PngDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| PngDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| PngDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(request)?;
            let offset = u64::try_from(
                index
                    .checked_mul(COPY_CHUNK_BYTES)
                    .ok_or_else(|| PngDecodeError::Source(RawSourceError::LengthConversion))?,
            )
            .map_err(|_| PngDecodeError::Source(RawSourceError::LengthConversion))?;
            source
                .read_exact_at(offset, chunk)
                .map_err(PngDecodeError::Source)?;
        }
        if source.revision().map_err(PngDecodeError::Source)? != revision {
            return Err(PngDecodeError::Source(RawSourceError::Changed));
        }
        Ok(Self {
            source_bytes: length,
            sha256: Sha256::digest(&bytes).into(),
            bytes,
        })
    }
}

fn map_error(error: PngDecodeError) -> ImageInputError {
    match error {
        PngDecodeError::Input(error) => error,
        PngDecodeError::Limit {
            kind: "width",
            actual,
            limit,
        } => ImageInputError::WidthLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        PngDecodeError::Limit {
            kind: "height",
            actual,
            limit,
        } => ImageInputError::HeightLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        PngDecodeError::Limit {
            kind: "pixels",
            actual,
            limit,
        } => ImageInputError::PixelLimit { actual, limit },
        PngDecodeError::Limit {
            kind: "decoded bytes",
            actual,
            limit,
        } => ImageInputError::DecodedByteLimit { actual, limit },
        PngDecodeError::Source(RawSourceError::TooLarge { actual, limit }) => {
            ImageInputError::SourceTooLarge { actual, limit }
        }
        PngDecodeError::Source(RawSourceError::Empty) => malformed("PNG source is empty"),
        PngDecodeError::Malformed(message) | PngDecodeError::Backend(message) => {
            malformed(&message)
        }
        other => malformed(&other.to_string()),
    }
}
fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Png,
        message: message.to_owned(),
    }
}
fn arithmetic() -> PngDecodeError {
    PngDecodeError::Input(ImageInputError::ArithmeticOverflow)
}
