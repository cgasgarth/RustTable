use rusttable_color::IccColorSpace;
use rusttable_image::{
    DecodeLimits, DecodeReceipt, DecodedFrame, DecodedImage, ImageDescriptor, ImageInputError,
    ImageProbe, InputFormat, Orientation, OwnedImage, SourceColor, SourceColorFallback,
    UnsupportedImageFeature,
};

use super::{WebPDataLocation, WebPDecodeError, WebPDecodeLimits, WebPDecodeRequest, WebPDecoder};
use crate::raw::RawSourceError;

pub(crate) fn decode_webp_probe(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let header = WebPDecoder::new()
        .probe_bytes(bytes, WebPDecodeLimits::from_common(limits))
        .map_err(map_error)?;
    Ok(ImageProbe::new(InputFormat::Webp, header.dimensions))
}

pub(crate) fn decode_legacy_rgba8(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let result = decode_core(bytes, limits)?;
    let (source_color, _) = source_color(bytes, result.header.metadata.icc_profile.as_ref())?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed("full WebP decode returned no pixels"))?;
    DecodedImage::new_with_color_encoding(
        pixels.dimensions(),
        pixels.to_rgba8(),
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

pub(crate) fn decode_webp_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    let result = decode_core(bytes, limits)?;
    let (source_color, icc) = source_color(bytes, result.header.metadata.icc_profile.as_ref())?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed("full WebP decode returned no typed pixels"))?;
    let descriptor = ImageDescriptor::new(
        pixels.dimensions(),
        pixels.format(),
        source_color.encoding(),
        Orientation::Normal,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let samples = pixels.into_samples();
    let actual = u64::try_from(samples.len()).unwrap_or(u64::MAX);
    let image = OwnedImage::new(descriptor, samples).map_err(|error| {
        ImageInputError::DecodedBufferInvariant {
            expected: u64::try_from(required_bytes(&error)).unwrap_or(u64::MAX),
            actual,
        }
    })?;
    let source_bytes =
        u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let receipt = DecodeReceipt::new_with_source_color(
        InputFormat::Webp,
        source_bytes,
        image.descriptor().clone(),
        source_color,
    )
    .map_err(|reason| ImageInputError::DecodeContract { reason })?;
    let frame = DecodedFrame::new(image, receipt)
        .map_err(|reason| ImageInputError::DecodeContract { reason })?;
    match icc {
        Some(profile) => frame
            .with_embedded_icc(profile)
            .map_err(|reason| ImageInputError::DecodeContract { reason }),
        None => Ok(frame),
    }
}

fn decode_core(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<super::WebPDecodeResult, ImageInputError> {
    WebPDecoder::new()
        .decode_bytes(
            bytes,
            &WebPDecodeRequest::new(WebPDecodeLimits::from_common(limits)),
        )
        .map_err(map_error)
}

fn source_color(
    bytes: &[u8],
    profile: Option<&super::WebPMetadataChunk>,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    let Some(profile) = profile else {
        return Ok((
            SourceColor::fallback(SourceColorFallback::EncodedSrgb),
            None,
        ));
    };
    let icc = extract(bytes, profile.location)?.to_vec();
    let color =
        crate::source_color::embedded_icc_profile_authoritative(&icc, IccColorSpace::Rgb)
            .map_err(|error| malformed(&format!("invalid embedded WebP ICC profile: {error}")))?;
    Ok((color, Some(icc)))
}

fn extract(bytes: &[u8], location: WebPDataLocation) -> Result<&[u8], ImageInputError> {
    let start =
        usize::try_from(location.offset).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let length =
        usize::try_from(location.length).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let end = start
        .checked_add(length)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    bytes
        .get(start..end)
        .ok_or_else(|| malformed("inventoried WebP metadata is outside the source"))
}

fn map_error(error: WebPDecodeError) -> ImageInputError {
    match error {
        WebPDecodeError::Cancelled => malformed("WebP decode cancelled"),
        WebPDecodeError::UnsupportedAnimation => ImageInputError::UnsupportedFeature {
            format: InputFormat::Webp,
            reason: UnsupportedImageFeature::Animation,
        },
        WebPDecodeError::UnsupportedRegion => ImageInputError::UnsupportedFeature {
            format: InputFormat::Webp,
            reason: UnsupportedImageFeature::Region,
        },
        WebPDecodeError::Malformed(message) | WebPDecodeError::Backend(message) => {
            malformed(&message)
        }
        WebPDecodeError::Limit {
            kind: "source bytes" | "declared source bytes" | "chunk bytes",
            actual,
            limit,
        } => ImageInputError::SourceTooLarge { actual, limit },
        WebPDecodeError::Limit {
            kind: "width",
            actual,
            limit,
        } => ImageInputError::WidthLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        WebPDecodeError::Limit {
            kind: "height",
            actual,
            limit,
        } => ImageInputError::HeightLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        WebPDecodeError::Limit {
            kind: "pixels",
            actual,
            limit,
        } => ImageInputError::PixelLimit { actual, limit },
        WebPDecodeError::Limit {
            kind: "decoded bytes" | "output bytes" | "backend output bytes",
            actual,
            limit,
        } => ImageInputError::DecodedByteLimit { actual, limit },
        WebPDecodeError::Limit {
            kind,
            actual,
            limit,
        } => malformed(&format!("{kind} {actual} exceeds limit {limit}")),
        WebPDecodeError::AllocationFailure
        | WebPDecodeError::Source(RawSourceError::AllocationFailure) => {
            ImageInputError::AllocationFailure
        }
        WebPDecodeError::Source(RawSourceError::TooLarge { actual, limit }) => {
            ImageInputError::SourceTooLarge { actual, limit }
        }
        WebPDecodeError::Source(RawSourceError::LengthConversion) => {
            ImageInputError::ArithmeticOverflow
        }
        WebPDecodeError::Source(RawSourceError::Empty) => malformed("WebP source is empty"),
        WebPDecodeError::Source(RawSourceError::Changed) => ImageInputError::Io {
            message: "WebP source changed while taking a stable snapshot".to_owned(),
        },
        WebPDecodeError::Source(RawSourceError::Read { offset, requested }) => {
            ImageInputError::Io {
                message: format!("WebP source read failed at {offset} for {requested} bytes"),
            }
        }
    }
}

fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Webp,
        message: message.to_owned(),
    }
}

fn required_bytes(error: &rusttable_image::ImageViewError) -> usize {
    match error {
        rusttable_image::ImageViewError::BufferTooShort { required, .. } => *required,
        _ => 0,
    }
}
