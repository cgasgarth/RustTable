//! Direct PNG output responsibility from `src/imageio/format/png.c`.
//!
//! The native writer emits a fixed PNG container, configurable zlib
//! compression, and network-order 16-bit samples. This leaf keeps the
//! container operation available without changing the shared output
//! orchestration; routing it into that orchestration remains a separate seam.

use std::fmt;
use std::io::Write;

use flate2::{Compression, write::ZlibEncoder};

use super::types::{PngPixelData, PngSampleLayout};

/// Deterministic options for the bounded PNG leaf encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngEncodeOptions {
    /// zlib compression level used by the native writer's compression setting.
    pub compression_level: u8,
    /// Maximum complete PNG size accepted by this leaf.
    pub max_encoded_bytes: u64,
}

impl PngEncodeOptions {
    #[must_use]
    pub const fn new(compression_level: u8, max_encoded_bytes: u64) -> Self {
        Self {
            compression_level,
            max_encoded_bytes,
        }
    }
}

impl Default for PngEncodeOptions {
    fn default() -> Self {
        Self {
            compression_level: 5,
            max_encoded_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

/// Typed PNG encoding failures that never publish a partial buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PngEncodeError {
    InvalidCompression { actual: u8 },
    InvalidLimit,
    ArithmeticOverflow,
    SampleCount { expected: u64, actual: u64 },
    AllocationFailure,
    OutputTooLarge { actual: u64, limit: u64 },
    Compression(String),
}

impl fmt::Display for PngEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCompression { actual } => {
                write!(formatter, "PNG compression level {actual} is outside 0..=9")
            }
            Self::InvalidLimit => formatter.write_str("PNG encoded-byte limit must be nonzero"),
            Self::ArithmeticOverflow => formatter.write_str("PNG size arithmetic overflowed"),
            Self::SampleCount { expected, actual } => write!(
                formatter,
                "PNG sample count {actual} does not match expected count {expected}"
            ),
            Self::AllocationFailure => formatter.write_str("PNG output allocation failed"),
            Self::OutputTooLarge { actual, limit } => {
                write!(formatter, "PNG output {actual} exceeds limit {limit}")
            }
            Self::Compression(message) => write!(formatter, "PNG compression failed: {message}"),
        }
    }
}

impl std::error::Error for PngEncodeError {}

/// Stateless, deterministic PNG encoder for the typed PNG sample variants.
#[derive(Debug, Clone, Copy, Default)]
pub struct PngEncoder;

impl PngEncoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Encodes one complete PNG buffer with filter type zero for every row.
    ///
    /// Samples remain in their declared channel order. Sixteen-bit samples are
    /// written most-significant byte first as required by PNG, regardless of
    /// the host's native byte order.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the samples are inconsistent, compression or
    /// size settings are invalid, allocation fails, or the output limit would
    /// be exceeded.
    pub fn encode(
        &self,
        pixels: &PngPixelData,
        options: PngEncodeOptions,
    ) -> Result<Vec<u8>, PngEncodeError> {
        if options.compression_level > 9 {
            return Err(PngEncodeError::InvalidCompression {
                actual: options.compression_level,
            });
        }
        if options.max_encoded_bytes == 0 {
            return Err(PngEncodeError::InvalidLimit);
        }
        let dimensions = pixels.dimensions();
        let width =
            usize::try_from(dimensions.width()).map_err(|_| PngEncodeError::ArithmeticOverflow)?;
        let height =
            usize::try_from(dimensions.height()).map_err(|_| PngEncodeError::ArithmeticOverflow)?;
        let channels = pixels.layout().channels();
        let expected_samples = dimensions
            .pixel_count()
            .map_err(|_| PngEncodeError::ArithmeticOverflow)?
            .checked_mul(u64::try_from(channels).map_err(|_| PngEncodeError::ArithmeticOverflow)?)
            .ok_or(PngEncodeError::ArithmeticOverflow)?;
        let actual_samples =
            u64::try_from(pixels.sample_count()).map_err(|_| PngEncodeError::ArithmeticOverflow)?;
        if actual_samples != expected_samples {
            return Err(PngEncodeError::SampleCount {
                expected: expected_samples,
                actual: actual_samples,
            });
        }
        let row_samples = width
            .checked_mul(channels)
            .ok_or(PngEncodeError::ArithmeticOverflow)?;
        let sample_bytes = usize::from(matches!(
            pixels.sample_type(),
            rusttable_image::SampleType::U16
        )) + 1;
        let raw_capacity = height
            .checked_mul(
                1usize
                    .checked_add(
                        row_samples
                            .checked_mul(sample_bytes)
                            .ok_or(PngEncodeError::ArithmeticOverflow)?,
                    )
                    .ok_or(PngEncodeError::ArithmeticOverflow)?,
            )
            .ok_or(PngEncodeError::ArithmeticOverflow)?;

        let mut raw = Vec::new();
        raw.try_reserve_exact(raw_capacity)
            .map_err(|_| PngEncodeError::AllocationFailure)?;
        match pixels {
            PngPixelData::GrayU8 { samples, .. }
            | PngPixelData::GrayAU8 { samples, .. }
            | PngPixelData::RgbU8 { samples, .. }
            | PngPixelData::RgbaU8 { samples, .. } => {
                append_u8_rows(&mut raw, samples, row_samples);
            }
            PngPixelData::GrayU16 { samples, .. }
            | PngPixelData::GrayAU16 { samples, .. }
            | PngPixelData::RgbU16 { samples, .. }
            | PngPixelData::RgbaU16 { samples, .. } => {
                append_u16_rows(&mut raw, samples, row_samples);
            }
        }

        let mut compressed = Vec::new();
        let mut compressor = ZlibEncoder::new(
            &mut compressed,
            Compression::new(u32::from(options.compression_level)),
        );
        compressor
            .write_all(&raw)
            .map_err(|error| PngEncodeError::Compression(error.to_string()))?;
        compressor
            .finish()
            .map_err(|error| PngEncodeError::Compression(error.to_string()))?;

        let color_type = match pixels.layout() {
            PngSampleLayout::Gray => 0,
            PngSampleLayout::GrayA => 4,
            PngSampleLayout::Rgb => 2,
            PngSampleLayout::Rgba => 6,
        };
        let bit_depth = if sample_bytes == 1 { 8 } else { 16 };
        let mut ihdr = [0_u8; 13];
        ihdr[0..4].copy_from_slice(&dimensions.width().to_be_bytes());
        ihdr[4..8].copy_from_slice(&dimensions.height().to_be_bytes());
        ihdr[8..13].copy_from_slice(&[bit_depth, color_type, 0, 0, 0]);

        let mut output = Vec::new();
        output
            .try_reserve_exact(8)
            .map_err(|_| PngEncodeError::AllocationFailure)?;
        output.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        append_chunk(&mut output, *b"IHDR", &ihdr, options.max_encoded_bytes)?;
        append_chunk(
            &mut output,
            *b"IDAT",
            &compressed,
            options.max_encoded_bytes,
        )?;
        append_chunk(&mut output, *b"IEND", &[], options.max_encoded_bytes)?;
        Ok(output)
    }
}

fn append_u8_rows(output: &mut Vec<u8>, samples: &[u8], row_samples: usize) {
    for row in samples.chunks_exact(row_samples) {
        output.push(0);
        output.extend_from_slice(row);
    }
}

fn append_u16_rows(output: &mut Vec<u8>, samples: &[u16], row_samples: usize) {
    for row in samples.chunks_exact(row_samples) {
        output.push(0);
        for &sample in row {
            output.extend_from_slice(&sample.to_be_bytes());
        }
    }
}

fn append_chunk(
    output: &mut Vec<u8>,
    kind: [u8; 4],
    data: &[u8],
    limit: u64,
) -> Result<(), PngEncodeError> {
    let length = u32::try_from(data.len()).map_err(|_| PngEncodeError::ArithmeticOverflow)?;
    let next = output
        .len()
        .checked_add(12)
        .and_then(|length| length.checked_add(data.len()))
        .ok_or(PngEncodeError::ArithmeticOverflow)?;
    let next_u64 = u64::try_from(next).unwrap_or(u64::MAX);
    if next_u64 > limit {
        return Err(PngEncodeError::OutputTooLarge {
            actual: next_u64,
            limit,
        });
    }
    output
        .try_reserve(12usize.saturating_add(data.len()))
        .map_err(|_| PngEncodeError::AllocationFailure)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(&kind);
    output.extend_from_slice(data);
    output.extend_from_slice(&crc32(kind, data).to_be_bytes());
    Ok(())
}

fn crc32(kind: [u8; 4], data: &[u8]) -> u32 {
    let mut crc = u32::MAX;
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
