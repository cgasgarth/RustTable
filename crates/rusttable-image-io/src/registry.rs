use std::io::Cursor;

use image::{ImageFormat, ImageReader};
use rusttable_image::{
    DecodeLimits, DecodedImage, DecoderDescriptor, DecoderIdentity, ImageDimensions,
    ImageInputError, ImageProbe, InputFormat,
};

use crate::input::{enforce_limits, probe_tiff, validate_tiff};
use crate::jpeg::JpegDecoder;
use crate::png::{decode_legacy_rgba8, decode_png_probe, is_png_signature};
use crate::raw::{decode_raw, is_raw, probe_raw};

/// Maximum amount of a source that any format probe may inspect.
pub const PROBE_BUDGET_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeOutcome {
    NoMatch,
    Match {
        decoder: DecoderDescriptor,
        probe: ImageProbe,
    },
    MalformedRecognized {
        decoder: DecoderDescriptor,
        error: ImageInputError,
    },
}

#[derive(Debug, Clone, Copy)]
struct DecoderEntry {
    descriptor: DecoderDescriptor,
    matches: fn(&[u8]) -> bool,
    full_source_probe: bool,
    probe: fn(&[u8], DecodeLimits) -> Result<ImageProbe, ImageInputError>,
    decode: fn(&[u8], DecodeLimits) -> Result<DecodedImage, ImageInputError>,
}

macro_rules! builtin_decoders {
    (
        $count:expr;
        $(
            $format:ident {
                id: $id:literal,
                implementation: $implementation:literal,
                matches: $matches:path,
                probe: $probe:path,
                decode: $decode:path,
                full_source_probe: $full_source_probe:expr
            }
        ),+ $(,)?
    ) => {
        static STANDARD_DESCRIPTORS: [DecoderDescriptor; $count] = [
            $(DecoderDescriptor::new(
                DecoderIdentity::new($id, 1, $implementation),
                InputFormat::$format,
            )),+
        ];

        static STANDARD_DECODERS: [DecoderEntry; $count] = [
            $(DecoderEntry {
                descriptor: DecoderDescriptor::new(
                    DecoderIdentity::new($id, 1, $implementation),
                    InputFormat::$format,
                ),
                matches: $matches,
                full_source_probe: $full_source_probe,
                probe: $probe,
                decode: $decode,
            }),+
        ];
    };
}

builtin_decoders! {
    4;
    Jpeg {
        id: "rusttable.decoder.jpeg.v1",
        implementation: "image-jpeg-0.25",
        matches: is_jpeg,
        probe: probe_jpeg,
        decode: decode_jpeg,
        full_source_probe: false
    },
    Png {
        id: "rusttable.decoder.png.v1",
        implementation: "png-0.18.1-pure-rust",
        matches: is_png_signature,
        probe: decode_png_probe,
        decode: decode_png,
        full_source_probe: false
    },
    Raw {
        id: "rusttable.decoder.raw.v1",
        implementation: "rawler-0.7.2",
        matches: is_raw,
        probe: probe_raw,
        decode: decode_raw,
        full_source_probe: true
    },
    Tiff {
        id: "rusttable.decoder.tiff.v1",
        implementation: "image-tiff-0.25",
        matches: is_tiff,
        probe: probe_classic_tiff,
        decode: decode_classic_tiff,
        full_source_probe: false
    }
}

/// Fixed, built-in image decoders selected solely from container signatures.
///
/// The registry is intentionally closed: `RustTable` does not load format
/// plugins, inspect extensions, or fall back to unrelated decoders.
#[derive(Debug, Clone, Copy)]
pub struct ImageDecoderRegistry {
    entries: &'static [DecoderEntry],
}

impl ImageDecoderRegistry {
    /// Returns the deterministic built-in PNG/JPEG/RAW/classic-TIFF registry.
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            entries: &STANDARD_DECODERS,
        }
    }

    /// Returns the stable identities registered by the standard registry.
    #[must_use]
    pub const fn descriptors(&self) -> &'static [DecoderDescriptor] {
        &STANDARD_DESCRIPTORS
    }

    /// Performs bounded signature/container probing without decoding pixels.
    #[must_use]
    pub fn probe(&self, bytes: &[u8], limits: DecodeLimits) -> ProbeOutcome {
        let window = &bytes[..bytes.len().min(PROBE_BUDGET_BYTES)];
        let Some(decoder) = self
            .entries
            .iter()
            .find(|decoder| (decoder.matches)(window))
        else {
            return ProbeOutcome::NoMatch;
        };
        let source = if decoder.full_source_probe {
            bytes
        } else {
            window
        };
        match (decoder.probe)(source, limits) {
            Ok(probe) => ProbeOutcome::Match {
                decoder: decoder.descriptor,
                probe,
            },
            Err(error) => ProbeOutcome::MalformedRecognized {
                decoder: decoder.descriptor,
                error,
            },
        }
    }

    /// Returns the selected decoder identity after signature selection.
    ///
    /// # Errors
    ///
    /// Returns `UnsupportedSignature` when no built-in signature matches.
    pub fn select(&self, bytes: &[u8]) -> Result<DecoderDescriptor, ImageInputError> {
        match self.probe(
            bytes,
            DecodeLimits::new(u64::MAX, u32::MAX, u32::MAX, u64::MAX / 4, u64::MAX / 4)
                .unwrap_or_else(|_| unreachable!("selection limits are representable")),
        ) {
            ProbeOutcome::NoMatch => Err(unsupported_signature(bytes)),
            ProbeOutcome::Match { decoder, .. }
            | ProbeOutcome::MalformedRecognized { decoder, .. } => Ok(decoder),
        }
    }

    /// Probes one immutable byte source through signature-first dispatch.
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
        match self.probe(bytes, limits) {
            ProbeOutcome::NoMatch => Err(unsupported_signature(bytes)),
            ProbeOutcome::Match { probe, .. } => Ok(probe),
            ProbeOutcome::MalformedRecognized { error, .. } => Err(error),
        }
    }

    /// Decodes one immutable byte source through signature-first dispatch.
    ///
    /// The bounded structural probe runs before the backend and a backend
    /// failure never causes another decoder to be tried.
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
        let decoder = match self.probe(bytes, limits) {
            ProbeOutcome::NoMatch => return Err(unsupported_signature(bytes)),
            ProbeOutcome::MalformedRecognized { error, .. } => return Err(error),
            ProbeOutcome::Match { decoder, .. } => decoder,
        };
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.descriptor == decoder)
            .ok_or_else(|| ImageInputError::MalformedInput {
                format: decoder.format(),
                message: "selected decoder disappeared from the registry".to_owned(),
            })?;
        (entry.decode)(bytes, limits)
    }
}

impl Default for ImageDecoderRegistry {
    fn default() -> Self {
        Self::standard()
    }
}

fn unsupported_signature(bytes: &[u8]) -> ImageInputError {
    ImageInputError::UnsupportedSignature {
        signature: bytes.iter().copied().take(8).collect(),
    }
}

fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8])
}

fn is_tiff(bytes: &[u8]) -> bool {
    bytes.starts_with(&[b'I', b'I', 0x2a, 0x00])
        || bytes.starts_with(&[b'M', b'M', 0x00, 0x2a])
        || bytes.starts_with(&[b'I', b'I', 0x2b, 0x00])
        || bytes.starts_with(&[b'M', b'M', 0x00, 0x2b])
}

fn probe_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    let header = crate::jpeg::probe_bounded(bytes, limits, bytes.len() >= PROBE_BUDGET_BYTES)?;
    Ok(ImageProbe::new(InputFormat::Jpeg, header.dimensions))
}

fn probe_classic_tiff(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    probe_tiff(bytes, limits)
}

fn decode_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let decoder = JpegDecoder::new();
    let (dimensions, pixels) = decoder.decode_rgba8(bytes, limits)?;
    rusttable_image::DecodedImage::new_with_color_encoding(
        dimensions,
        pixels,
        rusttable_image::ColorEncoding::Unspecified,
    )
    .map_err(|error| match error {
        rusttable_image::DecodedImageError::ArithmeticOverflow => {
            ImageInputError::ArithmeticOverflow
        }
        rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
            ImageInputError::DecodedBufferInvariant { expected, actual }
        }
    })
}

fn decode_png(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let (dimensions, pixels) = decode_legacy_rgba8(bytes, limits)?;
    rusttable_image::DecodedImage::new_with_color_encoding(
        dimensions,
        pixels,
        rusttable_image::ColorEncoding::Unspecified,
    )
    .map_err(|error| match error {
        rusttable_image::DecodedImageError::ArithmeticOverflow => {
            ImageInputError::ArithmeticOverflow
        }
        rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
            ImageInputError::DecodedBufferInvariant { expected, actual }
        }
    })
}

fn decode_classic_tiff(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    validate_tiff(bytes)?;
    let probe = probe_tiff(bytes, limits)?;
    decode_image(bytes, probe, limits, ImageFormat::Tiff)
}

fn decode_image(
    bytes: &[u8],
    probe: ImageProbe,
    limits: DecodeLimits,
    format: ImageFormat,
) -> Result<DecodedImage, ImageInputError> {
    let decoded = ImageReader::with_format(Cursor::new(bytes), format)
        .decode()
        .map_err(|error| ImageInputError::MalformedInput {
            format: probe.format(),
            message: error.to_string(),
        })?
        .to_rgba8();
    let dimensions = ImageDimensions::new(decoded.width(), decoded.height())
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_limits(limits, dimensions)?;
    if dimensions != probe.dimensions() {
        return Err(ImageInputError::MalformedInput {
            format: probe.format(),
            message: "decoded dimensions differ from the container probe".to_owned(),
        });
    }
    DecodedImage::new_with_color_encoding(
        dimensions,
        decoded.into_raw(),
        rusttable_image::ColorEncoding::Unspecified,
    )
    .map_err(|error| match error {
        rusttable_image::DecodedImageError::ArithmeticOverflow => {
            ImageInputError::ArithmeticOverflow
        }
        rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
            ImageInputError::DecodedBufferInvariant { expected, actual }
        }
    })
}
