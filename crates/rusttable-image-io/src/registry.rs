use std::io::Cursor;

use image::{ImageFormat, ImageReader};
use rusttable_image::{DecodeLimits, DecodedImage, ImageInputError, ImageProbe, InputFormat};

use crate::input::{enforce_limits, malformed, probe_tiff};

/// Fixed, built-in image decoders selected solely from container signatures.
///
/// The registry is intentionally closed: `RustTable` does not load format
/// plugins, inspect extensions, or fall back to unrelated decoders.
#[derive(Debug, Clone, Copy)]
pub struct ImageDecoderRegistry {
    entries: &'static [DecoderEntry],
}

#[derive(Debug, Clone, Copy)]
struct DecoderEntry {
    matches: fn(&[u8]) -> bool,
    probe: fn(&[u8], DecodeLimits) -> Result<ImageProbe, ImageInputError>,
    decode: fn(&[u8], DecodeLimits) -> Result<DecodedImage, ImageInputError>,
}

static STANDARD_DECODERS: [DecoderEntry; 4] = [
    DecoderEntry {
        matches: is_jpeg,
        probe: probe_jpeg,
        decode: decode_jpeg,
    },
    DecoderEntry {
        matches: is_png,
        probe: probe_png,
        decode: decode_png,
    },
    DecoderEntry {
        matches: is_classic_tiff,
        probe: probe_classic_tiff,
        decode: decode_classic_tiff,
    },
    DecoderEntry {
        matches: is_bigtiff,
        probe: reject_bigtiff,
        decode: reject_bigtiff_decode,
    },
];

impl ImageDecoderRegistry {
    /// Returns the deterministic built-in PNG/JPEG/classic-TIFF registry.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            entries: &STANDARD_DECODERS,
        }
    }

    /// Probes one immutable byte source through signature-only dispatch.
    ///
    /// # Errors
    ///
    /// Returns typed unsupported-signature, malformed-container, feature, or
    /// decode-limit failures without trying another decoder after a match.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<ImageProbe, ImageInputError> {
        let decoder = self.select(bytes)?;
        (decoder.probe)(bytes, limits)
    }

    /// Decodes one immutable byte source through signature-only dispatch.
    ///
    /// # Errors
    ///
    /// Returns typed unsupported-signature, malformed-container, feature, or
    /// decode-limit failures without trying another decoder after a match.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedImage, ImageInputError> {
        let decoder = self.select(bytes)?;
        (decoder.decode)(bytes, limits)
    }

    fn select(&self, bytes: &[u8]) -> Result<&DecoderEntry, ImageInputError> {
        self.entries
            .iter()
            .find(|decoder| (decoder.matches)(bytes))
            .ok_or_else(|| ImageInputError::UnsupportedSignature {
                signature: bytes.iter().copied().take(8).collect(),
            })
    }
}

impl Default for ImageDecoderRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff])
}

fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'])
}

fn is_classic_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[b'I', b'I', 0x2a, 0x00]) || bytes.starts_with(&[b'M', b'M', 0x00, 0x2a])
}

fn is_bigtiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[b'I', b'I', 0x2b, 0x00]) || bytes.starts_with(&[b'M', b'M', 0x00, 0x2b])
}

fn probe_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    probe_standard(bytes, limits, InputFormat::Jpeg)
}

fn probe_png(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    probe_standard(bytes, limits, InputFormat::Png)
}

fn probe_standard(
    bytes: &[u8],
    limits: DecodeLimits,
    format: InputFormat,
) -> Result<ImageProbe, ImageInputError> {
    let dimensions = ImageReader::with_format(Cursor::new(bytes), image_format(format))
        .into_dimensions()
        .map_err(|error| malformed(format, &error))?;
    let dimensions = rusttable_image::ImageDimensions::new(dimensions.0, dimensions.1)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_limits(limits, dimensions)?;
    Ok(ImageProbe::new(format, dimensions))
}

fn probe_classic_tiff(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    probe_tiff(bytes, limits)
}

fn reject_bigtiff(_bytes: &[u8], _limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    Err(ImageInputError::UnsupportedFeature {
        format: InputFormat::Tiff,
        reason: rusttable_image::UnsupportedImageFeature::BigTiff,
    })
}

fn decode_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    decode_standard(bytes, limits, InputFormat::Jpeg)
}

fn decode_png(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    decode_standard(bytes, limits, InputFormat::Png)
}

fn decode_classic_tiff(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let probe = probe_tiff(bytes, limits)?;
    decode_image(bytes, probe, limits, ImageFormat::Tiff)
}

fn decode_standard(
    bytes: &[u8],
    limits: DecodeLimits,
    format: InputFormat,
) -> Result<DecodedImage, ImageInputError> {
    let probe = probe_standard(bytes, limits, format)?;
    decode_image(bytes, probe, limits, image_format(format))
}

fn decode_image(
    bytes: &[u8],
    probe: ImageProbe,
    limits: DecodeLimits,
    format: ImageFormat,
) -> Result<DecodedImage, ImageInputError> {
    let decoded = ImageReader::with_format(Cursor::new(bytes), format)
        .decode()
        .map_err(|error| malformed(probe.format(), &error))?
        .to_rgba8();
    let dimensions = rusttable_image::ImageDimensions::new(decoded.width(), decoded.height())
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_limits(limits, dimensions)?;
    if dimensions != probe.dimensions() {
        return Err(ImageInputError::MalformedInput {
            format: probe.format(),
            message: "decoded dimensions differ from the container probe".to_owned(),
        });
    }
    DecodedImage::new(dimensions, decoded.into_raw()).map_err(|error| match error {
        rusttable_image::DecodedImageError::ArithmeticOverflow => {
            ImageInputError::ArithmeticOverflow
        }
        rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
            ImageInputError::DecodedBufferInvariant { expected, actual }
        }
    })
}

fn reject_bigtiff_decode(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    reject_bigtiff(bytes, limits).map(|_| unreachable!("BigTIFF probe always rejects"))
}

fn image_format(format: InputFormat) -> ImageFormat {
    match format {
        InputFormat::Jpeg => ImageFormat::Jpeg,
        InputFormat::Png => ImageFormat::Png,
        InputFormat::Tiff => ImageFormat::Tiff,
    }
}
