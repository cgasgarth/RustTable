use rusttable_color::{Primaries, TransferFunction, WhitePoint};
use rusttable_image::{
    DecodeLimits, DecodeReceipt, DecodedFrame, DecodedImage, DecoderDescriptor, DecoderIdentity,
    ImageInputError, ImageProbe, InputFormat, SourceColor, SourceColorEvidence,
    SourceColorFallback,
};

use crate::jpeg::JpegDecoder;
use crate::jpegxl::{
    decode_jpegxl_frame, decode_jpegxl_probe, decode_legacy_rgba8 as decode_jpegxl,
    is_jpegxl_signature,
};
use crate::openexr::{
    ExrDecodeRequest, ExrSampleData, decode_exr_probe, decode_legacy_rgba8 as decode_exr_rgba8,
    is_exr_signature,
};
use crate::png::{decode_png_probe, is_png_signature};
use crate::raw::{decode_raw, decode_raw_frame as decode_native_raw_frame, is_raw, probe_raw};
use crate::tiff::{
    TiffDecodeRequest, TiffPhotometric, TiffSampleData, TiffStorageLayout,
    decode_legacy_rgba8 as decode_tiff_rgba8, decode_tiff_probe, is_tiff_signature,
};
use crate::webp::{
    decode_legacy_rgba8 as decode_webp, decode_webp_frame, decode_webp_probe, is_webp_signature,
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
    7;
    Jpeg {
        id: "rusttable.decoder.jpeg.v1",
        implementation: "image-jpeg-0.25",
        matches: is_jpeg,
                probe: probe_jpeg,
                decode: decode_jpeg,
        decode_frame: decode_jpeg_frame,
        full_source_probe: false
    },
    JpegXl {
        id: "rusttable.decoder.jpeg-xl.v1",
        implementation: "jxl-oxide-0.12.6-pure-rust",
        matches: is_jpegxl_signature,
        probe: decode_jpegxl_probe,
        decode: decode_jpegxl,
        decode_frame: decode_jpegxl_frame,
        full_source_probe: true
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
    },
    Webp {
        id: "rusttable.decoder.webp.v1",
        implementation: "image-webp-0.2.4-pure-rust",
        matches: is_webp_signature,
        probe: decode_webp_probe,
        decode: decode_webp,
        decode_frame: decode_webp_frame,
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
    /// Returns the deterministic built-in raster and camera-RAW registry.
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
    let (dimensions, orientation, pixels) = decoder.decode_rgba8(bytes, limits)?;
    let header = decoder.inspect_bytes(bytes, limits)?;
    let (source_color, _) = jpeg_source_color(&header)?;
    rusttable_image::DecodedImage::new_with_source_orientation(
        dimensions,
        pixels,
        source_color.encoding(),
        orientation,
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
    let decoder = JpegDecoder::new();
    let (dimensions, orientation, pixels) = decoder.decode_rgba8(bytes, limits)?;
    let header = decoder.inspect_bytes(bytes, limits)?;
    let (source_color, icc) = jpeg_source_color(&header)?;
    let image = owned_image(
        dimensions,
        rusttable_image::PixelFormat::rgba8(),
        source_color,
        orientation,
        pixels,
    )?;
    frame(bytes, InputFormat::Jpeg, image, source_color, icc)
}

fn decode_png(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let result = crate::png::PngDecoder::new()
        .decode_bytes(
            bytes,
            &crate::png::PngDecodeRequest::new(crate::png::PngDecodeLimits::from_common(limits)),
        )
        .map_err(|error| source_color_error(InputFormat::Png, error))?;
    let pixels = result
        .pixels
        .ok_or_else(|| ImageInputError::MalformedInput {
            format: InputFormat::Png,
            message: "PNG full decode returned no typed pixels".to_owned(),
        })?;
    let dimensions = pixels.dimensions();
    let rgba = pixels.to_rgba8();
    let (source_color, _) = png_source_color(&result.header)?;
    rusttable_image::DecodedImage::new_with_color_encoding(
        dimensions,
        rgba,
        source_color.encoding(),
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
    let (source_color, icc) = png_source_color(&result.header)?;
    let image = recolor_owned(image, source_color)?;
    frame(bytes, InputFormat::Png, image, source_color, icc)
}

fn decode_exr(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let (dimensions, pixels) = decode_exr_rgba8(bytes, limits)?;
    let result = crate::openexr::ExrDecoder::new()
        .decode_bytes(
            bytes,
            &ExrDecodeRequest::new(crate::openexr::ExrDecodeLimits::from_common(limits)).header(),
        )
        .map_err(|error| source_color_error(InputFormat::OpenExr, error))?;
    let (source_color, _) = exr_source_color(&result.part.metadata)?;
    DecodedImage::new_with_color_encoding(dimensions, pixels, source_color.encoding()).map_err(
        |error| match error {
            rusttable_image::DecodedImageError::ArithmeticOverflow => {
                ImageInputError::ArithmeticOverflow
            }
            rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
                ImageInputError::DecodedBufferInvariant { expected, actual }
            }
        },
    )
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
    let (source_color, icc) = exr_source_color(&result.part.metadata)?;
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
        source_color,
        rusttable_image::Orientation::Normal,
        bytes,
    )?;
    frame(source, InputFormat::OpenExr, image, source_color, icc)
}

fn decode_tiff(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedImage, ImageInputError> {
    let (dimensions, orientation, pixels) = decode_tiff_rgba8(bytes, limits)?;
    let result = crate::tiff::TiffDecoder::new()
        .decode_bytes(
            bytes,
            &TiffDecodeRequest::new(crate::tiff::TiffDecodeLimits::from_common(limits)),
        )
        .map_err(|error| source_color_error(InputFormat::Tiff, error))?;
    let samples = result
        .pixels
        .as_ref()
        .ok_or_else(|| ImageInputError::MalformedInput {
            format: InputFormat::Tiff,
            message: "TIFF full decode returned no typed pixels".to_owned(),
        })?;
    let (source_color, _) = tiff_source_color(bytes, &result.page, &samples.samples)?;
    DecodedImage::new_with_source_orientation(
        dimensions,
        pixels,
        source_color.encoding(),
        orientation,
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
    let (source_color, icc) = tiff_source_color(bytes, &result.page, &pixels.samples)?;
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
        source_color,
        result.page.orientation,
        sample_bytes,
    )?;
    frame(bytes, InputFormat::Tiff, image, source_color, icc)
}

fn decode_raw_frame(bytes: &[u8], limits: DecodeLimits) -> Result<DecodedFrame, ImageInputError> {
    decode_native_raw_frame(bytes, limits)
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

fn jpeg_source_color(
    header: &crate::jpeg::JpegHeader,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    let segments = header
        .metadata
        .iter()
        .filter(|segment| segment.is_profile())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return Ok((
            SourceColor::fallback(SourceColorFallback::EncodedSrgb),
            None,
        ));
    }
    let mut count = None;
    let mut parts = Vec::<Option<&[u8]>>::new();
    for segment in segments {
        if segment.data.len() < 14 {
            return Err(malformed_color(
                InputFormat::Jpeg,
                "truncated JPEG ICC segment",
            ));
        }
        let sequence = usize::from(segment.data[12]);
        let segment_count = usize::from(segment.data[13]);
        if sequence == 0 || segment_count == 0 || sequence > segment_count {
            return Err(malformed_color(
                InputFormat::Jpeg,
                "invalid JPEG ICC sequence",
            ));
        }
        if count.is_some_and(|value| value != segment_count) {
            return Err(malformed_color(
                InputFormat::Jpeg,
                "inconsistent JPEG ICC segment counts",
            ));
        }
        count = Some(segment_count);
        if parts.is_empty() {
            parts.resize(segment_count, None);
        }
        let slot = parts
            .get_mut(sequence - 1)
            .ok_or_else(|| malformed_color(InputFormat::Jpeg, "invalid JPEG ICC sequence"))?;
        if slot.replace(&segment.data[14..]).is_some() {
            return Err(malformed_color(
                InputFormat::Jpeg,
                "duplicate JPEG ICC segment",
            ));
        }
    }
    if parts.iter().any(Option::is_none) {
        return Err(malformed_color(
            InputFormat::Jpeg,
            "incomplete JPEG ICC profile",
        ));
    }
    let icc = parts
        .into_iter()
        .flatten()
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    let source_color = crate::source_color::embedded_icc(&icc)
        .map_err(|error| source_color_error(InputFormat::Jpeg, error))?;
    Ok((source_color, Some(icc)))
}

fn png_source_color(
    header: &crate::png::PngHeader,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    let metadata = &header.metadata;
    if let Some(profile) = &metadata.icc_profile {
        let source_color = crate::source_color::embedded_icc(&profile.data)
            .map_err(|error| source_color_error(InputFormat::Png, error))?;
        return Ok((source_color, Some(profile.data.clone())));
    }
    if metadata.srgb_intent.is_some() {
        return SourceColor::declared(rusttable_image::ColorEncoding::SrgbD65)
            .map(|color| (color, None))
            .map_err(|error| source_color_error(InputFormat::Png, error));
    }
    if metadata.chromaticities.is_some() || metadata.gamma.is_some() {
        let primaries = metadata
            .chromaticities
            .map_or(Ok(Primaries::srgb()), |values| {
                let value = |index: usize| png_scaled_integer(values[index]);
                let white = WhitePoint::custom(value(0)?, value(1)?)
                    .map_err(|_| malformed_color(InputFormat::Png, "invalid PNG white point"))?;
                Primaries::new(
                    (value(2)?, value(3)?),
                    (value(4)?, value(5)?),
                    (value(6)?, value(7)?),
                    white,
                )
                .map_err(|_| malformed_color(InputFormat::Png, "invalid PNG chromaticities"))
            })?;
        let transfer = metadata.gamma.map_or(Ok(TransferFunction::Srgb), |gamma| {
            if gamma == 0 {
                return Err(malformed_color(InputFormat::Png, "zero PNG gamma"));
            }
            TransferFunction::gamma(1.0 / png_scaled_integer(gamma)?)
                .map_err(|_| malformed_color(InputFormat::Png, "invalid PNG gamma"))
        })?;
        let evidence = if metadata.chromaticities.is_some() {
            SourceColorEvidence::EmbeddedChromaticities
        } else {
            SourceColorEvidence::EmbeddedContainerMetadata
        };
        let source_color =
            crate::source_color::embedded_chromaticities(primaries, transfer, evidence)
                .map_err(|error| source_color_error(InputFormat::Png, error))?;
        return Ok((source_color, None));
    }
    Ok((
        SourceColor::fallback(SourceColorFallback::EncodedSrgb),
        None,
    ))
}

fn png_scaled_integer(value: u32) -> Result<f32, ImageInputError> {
    let whole = u16::try_from(value / 1_000)
        .map_err(|_| malformed_color(InputFormat::Png, "PNG color value is out of range"))?;
    let remainder = u16::try_from(value % 1_000)
        .map_err(|_| malformed_color(InputFormat::Png, "PNG color value is out of range"))?;
    Ok((f32::from(whole) * 1_000.0 + f32::from(remainder)) / 100_000.0)
}

fn exr_source_color(
    metadata: &crate::openexr::ExrMetadataInventory,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    if let Some(profile) = &metadata.icc {
        let source_color = crate::source_color::embedded_icc(&profile.data)
            .map_err(|error| source_color_error(InputFormat::OpenExr, error))?;
        return Ok((source_color, Some(profile.data.clone())));
    }
    if let Some(chromaticities) = &metadata.chromaticities {
        let white = WhitePoint::custom(chromaticities.white[0], chromaticities.white[1])
            .map_err(|_| malformed_color(InputFormat::OpenExr, "invalid EXR white point"))?;
        let primaries = Primaries::new(
            (chromaticities.red[0], chromaticities.red[1]),
            (chromaticities.green[0], chromaticities.green[1]),
            (chromaticities.blue[0], chromaticities.blue[1]),
            white,
        )
        .map_err(|_| malformed_color(InputFormat::OpenExr, "invalid EXR chromaticities"))?;
        let source_color = crate::source_color::embedded_chromaticities(
            primaries,
            TransferFunction::Linear,
            SourceColorEvidence::EmbeddedChromaticities,
        )
        .map_err(|error| source_color_error(InputFormat::OpenExr, error))?;
        return Ok((source_color, None));
    }
    Ok((
        SourceColor::fallback(SourceColorFallback::LinearRec709),
        None,
    ))
}

fn tiff_source_color(
    bytes: &[u8],
    page: &crate::tiff::TiffPage,
    samples: &TiffSampleData,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    let fallback = if matches!(samples, TiffSampleData::F16(_) | TiffSampleData::F32(_)) {
        SourceColorFallback::LinearRec709
    } else {
        SourceColorFallback::EncodedSrgb
    };
    if let Some(location) = &page.metadata.icc {
        let start =
            usize::try_from(location.offset).map_err(|_| ImageInputError::ArithmeticOverflow)?;
        let length =
            usize::try_from(location.length).map_err(|_| ImageInputError::ArithmeticOverflow)?;
        let end = start
            .checked_add(length)
            .ok_or(ImageInputError::ArithmeticOverflow)?;
        let icc = bytes
            .get(start..end)
            .ok_or_else(|| malformed_color(InputFormat::Tiff, "TIFF ICC tag is out of bounds"))?
            .to_vec();
        if let Ok(source_color) = crate::source_color::embedded_icc(&icc) {
            return Ok((source_color, Some(icc)));
        }
    }
    Ok((SourceColor::fallback(fallback), None))
}

fn recolor_owned(
    image: rusttable_image::OwnedImage,
    source_color: SourceColor,
) -> Result<rusttable_image::OwnedImage, ImageInputError> {
    let descriptor = image.descriptor();
    let dimensions = descriptor.dimensions();
    let format = descriptor.format();
    let orientation = descriptor.orientation();
    owned_image(
        dimensions,
        format,
        source_color,
        orientation,
        image.into_bytes(),
    )
}

fn source_color_error(format: InputFormat, error: impl std::fmt::Display) -> ImageInputError {
    ImageInputError::MalformedInput {
        format,
        message: format!("source color evidence is invalid: {error}"),
    }
}

fn malformed_color(format: InputFormat, message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format,
        message: message.to_owned(),
    }
}

fn owned_image(
    dimensions: rusttable_image::ImageDimensions,
    format: rusttable_image::PixelFormat,
    source_color: SourceColor,
    orientation: rusttable_image::Orientation,
    bytes: Vec<u8>,
) -> Result<rusttable_image::OwnedImage, ImageInputError> {
    let descriptor = rusttable_image::ImageDescriptor::new(
        dimensions,
        format,
        source_color.encoding(),
        orientation,
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
    source_color: SourceColor,
    embedded_icc: Option<Vec<u8>>,
) -> Result<DecodedFrame, ImageInputError> {
    let source_bytes =
        u64::try_from(source.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let receipt = DecodeReceipt::new_with_source_color(
        format,
        source_bytes,
        image.descriptor().clone(),
        source_color,
    )
    .map_err(|reason| ImageInputError::DecodeContract { reason })?;
    let frame = DecodedFrame::new(image, receipt)
        .map_err(|reason| ImageInputError::DecodeContract { reason })?;
    match embedded_icc {
        Some(icc) => frame
            .with_embedded_icc(icc)
            .map_err(|reason| ImageInputError::DecodeContract { reason }),
        None => Ok(frame),
    }
}
