//! Bounded RGB sample decoding for operations that need source precision.

use png::{BitDepth, ColorType, Decoder, Limits};
use rusttable_image::{DecodeLimits, ImageDimensions, ImageInputError, UnsupportedImageFeature};
use std::io::Cursor;

/// An RGB image whose samples retain the PNG source bit depth.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedRgbSamples {
    dimensions: ImageDimensions,
    bit_depth: u8,
    samples: Vec<u16>,
}

impl DecodedRgbSamples {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
}

/// Decodes a bounded, non-interlaced RGB PNG without reducing 16-bit samples.
///
/// PNGs with grayscale, palette, or alpha channels are rejected instead of
/// being implicitly converted into a mask source.
///
/// # Errors
///
/// Returns a typed error when the PNG is malformed, unsupported, or exceeds
/// the supplied source, dimension, pixel, or decoded-byte limits.
#[allow(clippy::too_many_lines)]
pub fn decode_png_rgb_samples(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedRgbSamples, ImageInputError> {
    let source_bytes =
        u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if source_bytes > limits.max_source_bytes() {
        return Err(ImageInputError::SourceTooLarge {
            actual: source_bytes,
            limit: limits.max_source_bytes(),
        });
    }
    let (dimensions, bit_depth) = inspect_rgb_png(bytes)?;
    enforce_limits(limits, dimensions)?;
    let pixels = usize::try_from(
        dimensions
            .pixel_count()
            .map_err(|_| ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let bytes_per_sample = match bit_depth {
        8 => 1,
        16 => 2,
        _ => return Err(unsupported(UnsupportedImageFeature::BitDepth)),
    };
    let expected_bytes = pixels
        .checked_mul(3)
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let decoded_limit = limits
        .max_decoded_bytes()
        .checked_mul(2)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let expected_bytes_u64 =
        u64::try_from(expected_bytes).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if expected_bytes_u64 > decoded_limit {
        return Err(ImageInputError::DecodedByteLimit {
            actual: expected_bytes_u64,
            limit: decoded_limit,
        });
    }

    let decoder = Decoder::new_with_limits(
        Cursor::new(bytes),
        Limits {
            bytes: usize::try_from(decoded_limit)
                .map_err(|_| ImageInputError::ArithmeticOverflow)?,
        },
    );
    let mut reader = decoder
        .read_info()
        .map_err(|error| map_png_error(&error, decoded_limit))?;
    let (info_color_type, info_bit_depth, interlaced, has_transparency, has_animation) = {
        let info = reader.info();
        (
            info.color_type,
            info.bit_depth,
            info.interlaced,
            info.trns.is_some(),
            info.animation_control.is_some(),
        )
    };
    if info_color_type != ColorType::Rgb
        || info_bit_depth
            != if bit_depth == 8 {
                BitDepth::Eight
            } else {
                BitDepth::Sixteen
            }
        || interlaced
        || has_transparency
        || has_animation
    {
        return Err(unsupported(UnsupportedImageFeature::ColorModel));
    }
    let output_size = reader
        .output_buffer_size()
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    if output_size != expected_bytes {
        return Err(malformed("decoded PNG row size differs from its header"));
    }
    let mut output = vec![0; output_size];
    let frame = reader
        .next_frame(&mut output)
        .map_err(|error| map_png_error(&error, decoded_limit))?;
    if frame.width != dimensions.width()
        || frame.height != dimensions.height()
        || frame.color_type != info_color_type
        || frame.bit_depth != info_bit_depth
    {
        return Err(malformed("decoded PNG dimensions or sample format changed"));
    }
    let samples: Vec<u16> = if bit_depth == 8 {
        output.into_iter().map(u16::from).collect()
    } else {
        let (samples, remainder) = output.as_chunks::<2>();
        debug_assert!(remainder.is_empty());
        samples
            .iter()
            .map(|sample| u16::from_be_bytes([sample[0], sample[1]]))
            .collect()
    };
    let expected_samples = pixels
        .checked_mul(3)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    if samples.len() != expected_samples {
        return Err(ImageInputError::DecodedBufferInvariant {
            expected: u64::try_from(expected_samples)
                .map_err(|_| ImageInputError::ArithmeticOverflow)?,
            actual: u64::try_from(samples.len())
                .map_err(|_| ImageInputError::ArithmeticOverflow)?,
        });
    }
    Ok(DecodedRgbSamples {
        dimensions,
        bit_depth,
        samples,
    })
}

fn inspect_rgb_png(bytes: &[u8]) -> Result<(ImageDimensions, u8), ImageInputError> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !bytes.starts_with(SIGNATURE) {
        return Err(malformed("missing PNG signature"));
    }
    let ihdr = bytes
        .get(8..33)
        .ok_or_else(|| malformed("truncated PNG IHDR"))?;
    if u32::from_be_bytes(ihdr[0..4].try_into().expect("fixed IHDR length")) != 13
        || &ihdr[4..8] != b"IHDR"
    {
        return Err(malformed("invalid PNG IHDR"));
    }
    let dimensions = ImageDimensions::new(
        u32::from_be_bytes(ihdr[8..12].try_into().expect("fixed IHDR length")),
        u32::from_be_bytes(ihdr[12..16].try_into().expect("fixed IHDR length")),
    )
    .map_err(|_| malformed("PNG dimensions must be nonzero"))?;
    let bit_depth = ihdr[16];
    if !matches!(bit_depth, 8 | 16) {
        return Err(unsupported(UnsupportedImageFeature::BitDepth));
    }
    if ihdr[17] != 2 {
        return Err(unsupported(UnsupportedImageFeature::ColorModel));
    }
    if ihdr[18] != 0 || ihdr[19] != 0 || ihdr[20] != 0 {
        return Err(malformed(
            "unsupported PNG compression, filter, or interlace",
        ));
    }
    Ok((dimensions, bit_depth))
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
    Ok(())
}

fn malformed(message: impl Into<String>) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: rusttable_image::InputFormat::Png,
        message: message.into(),
    }
}

fn unsupported(reason: UnsupportedImageFeature) -> ImageInputError {
    ImageInputError::UnsupportedFeature {
        format: rusttable_image::InputFormat::Png,
        reason,
    }
}

fn map_png_error(error: &png::DecodingError, limit: u64) -> ImageInputError {
    if matches!(error, png::DecodingError::LimitsExceeded) {
        ImageInputError::DecodedByteLimit {
            actual: limit.saturating_add(1),
            limit,
        }
    } else {
        malformed(format!("PNG decode failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use png::{BitDepth, ColorType, Encoder};

    fn limits() -> DecodeLimits {
        DecodeLimits::new(16 * 1024, 8, 8, 64, 256).expect("test limits")
    }

    fn png_bytes(depth: BitDepth, color: ColorType, samples: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = Encoder::new(Cursor::new(&mut bytes), 2, 1);
        encoder.set_color(color);
        encoder.set_depth(depth);
        let mut writer = encoder.write_header().expect("PNG header");
        writer.write_image_data(samples).expect("PNG samples");
        writer.finish().expect("PNG finish");
        bytes
    }

    #[test]
    fn preserves_rgb_8_bit_samples() {
        let decoded = decode_png_rgb_samples(
            &png_bytes(BitDepth::Eight, ColorType::Rgb, &[0, 127, 255, 9, 8, 7]),
            limits(),
        )
        .expect("RGB8");
        assert_eq!(decoded.bit_depth(), 8);
        assert_eq!(decoded.samples(), &[0, 127, 255, 9, 8, 7]);
    }

    #[test]
    fn preserves_rgb_16_bit_samples() {
        let samples = [
            0x00, 0x01, 0x12, 0x34, 0xff, 0xff, 0xab, 0xcd, 0x00, 0x00, 0x80, 0x00,
        ];
        let decoded = decode_png_rgb_samples(
            &png_bytes(BitDepth::Sixteen, ColorType::Rgb, &samples),
            limits(),
        )
        .expect("RGB16");
        assert_eq!(decoded.bit_depth(), 16);
        assert_eq!(decoded.samples(), &[1, 0x1234, 0xffff, 0xabcd, 0, 0x8000]);
    }

    #[test]
    fn rejects_alpha_and_limits_before_decode() {
        let alpha = png_bytes(BitDepth::Eight, ColorType::Rgba, &[0, 1, 2, 3, 4, 5, 6, 7]);
        assert!(matches!(
            decode_png_rgb_samples(&alpha, limits()),
            Err(ImageInputError::UnsupportedFeature {
                reason: UnsupportedImageFeature::ColorModel,
                ..
            })
        ));

        let constrained = DecodeLimits::new(16 * 1024, 1, 1, 1, 4).expect("limits");
        let rgb = png_bytes(BitDepth::Eight, ColorType::Rgb, &[0, 1, 2, 3, 4, 5]);
        assert!(matches!(
            decode_png_rgb_samples(&rgb, constrained),
            Err(ImageInputError::WidthLimit { .. })
        ));
        let truncated = decode_png_rgb_samples(&rgb[..rgb.len() - 12], limits());
        assert!(matches!(
            truncated,
            Err(ImageInputError::MalformedInput { .. })
        ));
    }
}
