use rusttable_image::{
    DecodeLimits, DecodeReceipt, DecodedFrame, DecodedImage, DecoderDescriptor, DecoderIdentity,
    ImageInputError, ImageProbe, InputFormat,
};

use crate::jpeg::JpegDecoder;
use crate::openexr::{
    ExrDecodeRequest, ExrSampleData, decode_exr_probe, decode_legacy_rgba8 as decode_exr_rgba8,
    is_exr_signature,
};
use crate::png::{decode_legacy_rgba8, decode_png_probe, is_png_signature};
use crate::raw::{decode_raw, is_raw, probe_raw};
use crate::tiff::{
    TiffDecodeRequest, TiffPhotometric, TiffSampleData, TiffStorageLayout,
    decode_legacy_rgba8 as decode_tiff_rgba8, decode_tiff_probe, is_tiff_signature,
};

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
    decode_frame: fn(&[u8], DecodeLimits) -> Result<DecodedFrame, ImageInputError>,
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
                decode_frame: $decode_frame:path,
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
                decode_frame: $decode_frame,
            }),+
        ];
    };
}

builtin_decoders! {
    5;
    Jpeg {
        id: "rusttable.decoder.jpeg.v1",
        implementation: "image-jpeg-0.25",
        matches: is_jpeg,
                probe: probe_jpeg,
                decode: decode_jpeg,
        decode_frame: decode_jpeg_frame,
        full_source_probe: false
    },
    Png {
        id: "rusttable.decoder.png.v1",
        implementation: "png-0.18.1-pure-rust",
        matches: is_png_signature,
        probe: decode_png_probe,
        decode: decode_png,
        decode_frame: decode_png_frame,
        full_source_probe: false
    },
    OpenExr {
        id: "rusttable.decoder.openexr.v1",
        implementation: "exr-1.74.2-pure-rust",
        matches: is_exr_signature,
        probe: decode_exr_probe,
        decode: decode_exr,
        decode_frame: decode_exr_frame,
        full_source_probe: true
    },
    Raw {
        id: "rusttable.decoder.raw.v1",
        implementation: "rawler-0.7.2",
        matches: is_raw,
        probe: probe_raw,
        decode: decode_raw,
        decode_frame: decode_raw_frame,
        full_source_probe: true
    },
    Tiff {
        id: "rusttable.decoder.tiff.v1",
        implementation: "tiff-0.11.3-pure-rust",
        matches: is_tiff_signature,
        probe: decode_tiff_probe,
        decode: decode_tiff,
        decode_frame: decode_tiff_frame,
        full_source_probe: true
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

    /// Decodes one source without projecting its native sample representation.
    ///
    /// # Errors
    ///
    /// Returns the selected decoder's typed probe, decode, limit, or contract
    /// failure.
    pub fn decode_frame_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<DecodedFrame, ImageInputError> {
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
        (entry.decode_frame)(bytes, limits)
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

fn probe_jpeg(bytes: &[u8], limits: DecodeLimits) -> Result<ImageProbe, ImageInputError> {
    let header = crate::jpeg::probe_bounded(bytes, limits, bytes.len() >= PROBE_BUDGET_BYTES)?;
    Ok(ImageProbe::new(InputFormat::Jpeg, header.dimensions))
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

fn decode_jpeg_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    let image = decode_jpeg(bytes, limits)?.into_owned();
    frame(bytes, InputFormat::Jpeg, image)
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

fn decode_png_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    let result = crate::png::PngDecoder::new()
        .decode_bytes(
            bytes,
            &crate::png::PngDecodeRequest::new(crate::png::PngDecodeLimits::from_common(limits)),
        )
        .map_err(|error| ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message: error.to_string(),
        })?;
    let image = result
        .image
        .ok_or_else(|| ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message: "PNG full decode returned no typed image".to_owned(),
        })?;
    frame(bytes, InputFormat::Png, image)
}

fn decode_exr(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let (dimensions, pixels) = decode_exr_rgba8(bytes, limits)?;
    DecodedImage::new_with_color_encoding(
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

fn decode_exr_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    let source = bytes;
    let result = crate::openexr::ExrDecoder::new()
        .decode_bytes(
            bytes,
            &ExrDecodeRequest::new(crate::openexr::ExrDecodeLimits::from_common(limits)),
        )
        .map_err(|error| ImageInputError::MalformedInput {
            format: InputFormat::OpenExr,
            message: error.to_string(),
        })?;
    let pixels = result
        .pixels
        .ok_or_else(|| ImageInputError::MalformedInput {
            format: InputFormat::OpenExr,
            message: "OpenEXR full decode returned no typed pixels".to_owned(),
        })?;
    let format = rusttable_image::PixelFormat::new(
        match pixels.sample_type {
            crate::openexr::ExrSampleType::F16 => rusttable_image::SampleType::F16,
            crate::openexr::ExrSampleType::F32 => rusttable_image::SampleType::F32,
            crate::openexr::ExrSampleType::U32 => {
                return Err(ImageInputError::UnsupportedFeature {
                    format: InputFormat::OpenExr,
                    reason: rusttable_image::UnsupportedImageFeature::SampleFormat,
                });
            }
        },
        pixels.layout,
        if result.receipt.alpha == crate::openexr::ExrAlphaAssociation::Associated {
            rusttable_image::AlphaMode::Premultiplied
        } else if pixels.layout.channels() == 4 {
            rusttable_image::AlphaMode::Straight
        } else {
            rusttable_image::AlphaMode::None
        },
        rusttable_image::ByteOrder::Native,
        rusttable_image::StorageLayout::Interleaved,
    )
    .map_err(|_| ImageInputError::MalformedInput {
        format: InputFormat::OpenExr,
        message: "invalid OpenEXR channel format".to_owned(),
    })?;
    let bytes = match pixels.samples {
        ExrSampleData::F16(values) => values.into_iter().flat_map(u16::to_ne_bytes).collect(),
        ExrSampleData::F32(values) => values.into_iter().flat_map(f32::to_ne_bytes).collect(),
    };
    let image = owned_image(
        pixels.dimensions,
        format,
        rusttable_image::ColorEncoding::LinearSrgb,
        bytes,
    )?;
    frame(source, InputFormat::OpenExr, image)
}

fn decode_tiff(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let (dimensions, pixels) = decode_tiff_rgba8(bytes, limits)?;
    DecodedImage::new_with_color_encoding(
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

fn decode_tiff_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    let result = crate::tiff::TiffDecoder::new()
        .decode_bytes(
            bytes,
            &TiffDecodeRequest::new(crate::tiff::TiffDecodeLimits::from_common(limits)),
        )
        .map_err(|error| ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            message: error.to_string(),
        })?;
    let pixels = result
        .pixels
        .ok_or_else(|| ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            message: "TIFF full decode returned no typed pixels".to_owned(),
        })?;
    let sample_type = match &pixels.samples {
        TiffSampleData::U8(_) => rusttable_image::SampleType::U8,
        TiffSampleData::U16(_) => rusttable_image::SampleType::U16,
        TiffSampleData::F16(_) => rusttable_image::SampleType::F16,
        TiffSampleData::F32(_) => rusttable_image::SampleType::F32,
        _ => {
            return Err(ImageInputError::UnsupportedFeature {
                format: InputFormat::Tiff,
                reason: rusttable_image::UnsupportedImageFeature::SampleFormat,
            });
        }
    };
    let channels = match result.page.photometric {
        TiffPhotometric::WhiteIsZero | TiffPhotometric::BlackIsZero => {
            rusttable_image::ChannelLayout::Gray
        }
        TiffPhotometric::Rgb => {
            if result.page.alpha.is_empty() {
                rusttable_image::ChannelLayout::Rgb
            } else {
                rusttable_image::ChannelLayout::Rgba
            }
        }
        _ => {
            return Err(ImageInputError::UnsupportedFeature {
                format: InputFormat::Tiff,
                reason: rusttable_image::UnsupportedImageFeature::ColorModel,
            });
        }
    };
    let alpha = if channels.channels() == 4 {
        rusttable_image::AlphaMode::Straight
    } else {
        rusttable_image::AlphaMode::None
    };
    let format = rusttable_image::PixelFormat::new(
        sample_type,
        channels,
        alpha,
        rusttable_image::ByteOrder::Native,
        match pixels.storage {
            TiffStorageLayout::Chunky => rusttable_image::StorageLayout::Interleaved,
            TiffStorageLayout::Planar => {
                return Err(ImageInputError::UnsupportedFeature {
                    format: InputFormat::Tiff,
                    reason: rusttable_image::UnsupportedImageFeature::PlanarConfiguration,
                });
            }
        },
    )
    .map_err(|_| ImageInputError::MalformedInput {
        format: InputFormat::Tiff,
        message: "invalid TIFF channel format".to_owned(),
    })?;
    let sample_bytes = tiff_sample_bytes(pixels.samples);
    let image = owned_image(
        pixels.dimensions,
        format,
        rusttable_image::ColorEncoding::Unspecified,
        sample_bytes,
    )?;
    frame(bytes, InputFormat::Tiff, image)
}

fn decode_raw_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    let image = decode_raw(bytes, limits)?.into_owned();
    frame(bytes, InputFormat::Raw, image)
}

fn tiff_sample_bytes(samples: TiffSampleData) -> Vec<u8> {
    match samples {
        TiffSampleData::U8(values) => values,
        TiffSampleData::U16(values) | TiffSampleData::F16(values) => {
            values.into_iter().flat_map(u16::to_ne_bytes).collect()
        }
        TiffSampleData::F32(values) => values.into_iter().flat_map(f32::to_ne_bytes).collect(),
        TiffSampleData::U32(values) => values.into_iter().flat_map(u32::to_ne_bytes).collect(),
        TiffSampleData::I8(values) => values.into_iter().map(i8::cast_unsigned).collect(),
        TiffSampleData::I16(values) => values.into_iter().flat_map(i16::to_ne_bytes).collect(),
        TiffSampleData::I32(values) => values.into_iter().flat_map(i32::to_ne_bytes).collect(),
    }
}

fn owned_image(
    dimensions: rusttable_image::ImageDimensions,
    format: rusttable_image::PixelFormat,
    encoding: rusttable_image::ColorEncoding,
    bytes: Vec<u8>,
) -> Result<rusttable_image::OwnedImage, ImageInputError> {
    let descriptor = rusttable_image::ImageDescriptor::new(
        dimensions,
        format,
        encoding,
        rusttable_image::Orientation::Normal,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    rusttable_image::OwnedImage::new(descriptor, bytes).map_err(|_| {
        ImageInputError::DecodedBufferInvariant {
            expected: 0,
            actual: 0,
        }
    })
}

fn frame(
    source: &[u8],
    format: rusttable_image::InputFormat,
    image: rusttable_image::OwnedImage,
) -> Result<DecodedFrame, ImageInputError> {
    let source_bytes =
        u64::try_from(source.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let receipt = DecodeReceipt::new(format, source_bytes, image.descriptor().clone())
        .map_err(|reason| ImageInputError::DecodeContract { reason })?;
    DecodedFrame::new(image, receipt).map_err(|reason| ImageInputError::DecodeContract { reason })
}
