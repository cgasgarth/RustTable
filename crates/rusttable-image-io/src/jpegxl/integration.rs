use rusttable_color::{IccColorSpace, Primaries, TransferFunction, WhitePoint};
use rusttable_image::{
    ByteOrder, DecodeLimits, DecodeReceipt, DecodedFrame, DecodedImage, ImageDescriptor,
    ImageInputError, ImageProbe, InputFormat, Orientation, OwnedImage, PixelFormat, SampleType,
    SourceColor, SourceColorEvidence, StorageLayout, UnsupportedImageFeature,
};

use super::{
    JxlColorEncoding, JxlDecodeError, JxlDecodeLimits, JxlDecodeRequest, JxlDecoder, JxlPixelData,
    JxlPrimaries, JxlStructuredColor, JxlTransferFunction, JxlWhitePoint,
};
use crate::raw::RawSourceError;

pub(crate) fn decode_jpegxl_probe(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let dimensions = JxlDecoder::new()
        .probe_bytes(bytes, JxlDecodeLimits::from_common(limits))
        .map_err(map_error)?;
    Ok(ImageProbe::new(InputFormat::JpegXl, dimensions))
}

pub(crate) fn decode_legacy_rgba8(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let result = decode_core(bytes, limits)?;
    let (source_color, _) = source_color(&result.header.color)?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed("full JPEG XL decode returned no pixels"))?;
    let dimensions = pixels.dimensions;
    let rgba = to_rgba8(&pixels)?;
    DecodedImage::new_with_color_encoding(dimensions, rgba, source_color.encoding()).map_err(
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

pub(crate) fn decode_jpegxl_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    let result = decode_core(bytes, limits)?;
    let (source_color, icc) = source_color(&result.header.color)?;
    let pixels = result
        .pixels
        .ok_or_else(|| malformed("full JPEG XL decode returned no typed pixels"))?;
    let format = PixelFormat::new(
        SampleType::F32,
        pixels.layout,
        pixels.alpha,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .map_err(|_| malformed("JPEG XL pixel format is internally inconsistent"))?;
    let descriptor = ImageDescriptor::new(
        pixels.dimensions,
        format,
        source_color.encoding(),
        Orientation::Normal,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let samples = f32_bytes(&pixels.samples)?;
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
        InputFormat::JpegXl,
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
) -> Result<super::JxlDecodeResult, ImageInputError> {
    JxlDecoder::new()
        .decode_bytes(
            bytes,
            &JxlDecodeRequest::new(JxlDecodeLimits::from_common(limits)),
        )
        .map_err(map_error)
}

pub(super) fn source_color(
    color: &JxlColorEncoding,
) -> Result<(SourceColor, Option<Vec<u8>>), ImageInputError> {
    match color {
        JxlColorEncoding::Icc(profile) => {
            let expected = match profile.color_space {
                super::JxlColorSpace::Gray => IccColorSpace::Gray,
                super::JxlColorSpace::Rgb => IccColorSpace::Rgb,
            };
            let source =
                crate::source_color::embedded_icc_profile_authoritative(&profile.bytes, expected)
                    .map_err(|error| {
                    malformed(&format!("invalid embedded JPEG XL ICC profile: {error}"))
                })?;
            Ok((source, Some(profile.bytes.clone())))
        }
        JxlColorEncoding::Structured(color) => {
            let primaries = structured_primaries(color)?;
            let transfer = transfer(color.transfer)?;
            let source = crate::source_color::embedded_chromaticities(
                primaries,
                transfer,
                SourceColorEvidence::DeclaredEncoding,
            )
            .map_err(|error| {
                malformed(&format!(
                    "declared JPEG XL color encoding is invalid: {error:?}"
                ))
            })?;
            Ok((source, None))
        }
    }
}

fn structured_primaries(color: &JxlStructuredColor) -> Result<Primaries, ImageInputError> {
    let white =
        match color.white_point {
            JxlWhitePoint::D65 => WhitePoint::D65,
            JxlWhitePoint::EqualEnergy => WhitePoint::custom(1.0 / 3.0, 1.0 / 3.0)
                .map_err(|_| malformed("invalid equal-energy white point"))?,
            JxlWhitePoint::Dci => WhitePoint::custom(0.314, 0.351)
                .map_err(|_| malformed("invalid DCI white point"))?,
            JxlWhitePoint::Custom([x, y]) => WhitePoint::custom(x, y)
                .map_err(|_| malformed("invalid custom JPEG XL white point"))?,
        };
    let (red, green, blue) = match color.primaries {
        JxlPrimaries::Srgb => ((0.64, 0.33), (0.3, 0.6), (0.15, 0.06)),
        JxlPrimaries::Bt2100 => ((0.708, 0.292), (0.17, 0.797), (0.131, 0.046)),
        JxlPrimaries::P3 => ((0.68, 0.32), (0.265, 0.69), (0.15, 0.06)),
        JxlPrimaries::Custom { red, green, blue } => {
            ((red[0], red[1]), (green[0], green[1]), (blue[0], blue[1]))
        }
    };
    Primaries::new(red, green, blue, white)
        .map_err(|_| malformed("invalid JPEG XL primary chromaticities"))
}

fn transfer(value: JxlTransferFunction) -> Result<TransferFunction, ImageInputError> {
    match value {
        JxlTransferFunction::Gamma(value) => {
            TransferFunction::gamma(value).map_err(|_| malformed("invalid JPEG XL gamma transfer"))
        }
        JxlTransferFunction::Bt709 => Ok(TransferFunction::Rec709),
        JxlTransferFunction::Linear => Ok(TransferFunction::Linear),
        JxlTransferFunction::Srgb => Ok(TransferFunction::Srgb),
        JxlTransferFunction::Pq => Ok(TransferFunction::Pq),
        JxlTransferFunction::Dci => {
            TransferFunction::gamma(2.6).map_err(|_| malformed("invalid DCI gamma transfer"))
        }
        JxlTransferFunction::Hlg => Ok(TransferFunction::Hlg),
    }
}

fn to_rgba8(pixels: &JxlPixelData) -> Result<Vec<u8>, ImageInputError> {
    let count = pixels
        .dimensions
        .pixel_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let bytes = count
        .checked_mul(4)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let length = usize::try_from(bytes).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(length)
        .map_err(|_| ImageInputError::AllocationFailure)?;
    let channels = pixels.layout.channels();
    for source in pixels.samples.chunks_exact(channels) {
        let (gray, red, green, blue, alpha) = match pixels.layout {
            rusttable_image::ChannelLayout::Gray => (Some(source[0]), 0.0, 0.0, 0.0, 1.0),
            rusttable_image::ChannelLayout::GrayA => (Some(source[0]), 0.0, 0.0, 0.0, source[1]),
            rusttable_image::ChannelLayout::Rgb => (None, source[0], source[1], source[2], 1.0),
            rusttable_image::ChannelLayout::Rgba => {
                (None, source[0], source[1], source[2], source[3])
            }
            rusttable_image::ChannelLayout::Bayer | rusttable_image::ChannelLayout::XTrans => {
                return Err(malformed("JPEG XL produced a mosaic layout"));
            }
        };
        let alpha = alpha.clamp(0.0, 1.0);
        let unassociate = |value: f32| {
            if pixels.alpha == rusttable_image::AlphaMode::Premultiplied && alpha > 0.0 {
                value / alpha
            } else {
                value
            }
        };
        let (red, green, blue) = if let Some(gray) = gray {
            (gray, gray, gray)
        } else {
            (red, green, blue)
        };
        output.extend([
            quantize(unassociate(red)),
            quantize(unassociate(green)),
            quantize(unassociate(blue)),
            quantize(alpha),
        ]);
    }
    Ok(output)
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn quantize(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn f32_bytes(samples: &[f32]) -> Result<Vec<u8>, ImageInputError> {
    let length = samples
        .len()
        .checked_mul(size_of::<f32>())
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length)
        .map_err(|_| ImageInputError::AllocationFailure)?;
    for sample in samples {
        bytes.extend_from_slice(&sample.to_ne_bytes());
    }
    Ok(bytes)
}

fn map_error(error: JxlDecodeError) -> ImageInputError {
    match error {
        JxlDecodeError::UnsupportedAnimation { .. } => ImageInputError::UnsupportedFeature {
            format: InputFormat::JpegXl,
            reason: UnsupportedImageFeature::Animation,
        },
        JxlDecodeError::UnsupportedColorSpace | JxlDecodeError::UnsupportedBlackChannel => {
            ImageInputError::UnsupportedFeature {
                format: InputFormat::JpegXl,
                reason: UnsupportedImageFeature::ColorModel,
            }
        }
        JxlDecodeError::InvalidRegion => ImageInputError::UnsupportedFeature {
            format: InputFormat::JpegXl,
            reason: UnsupportedImageFeature::Region,
        },
        JxlDecodeError::Limit {
            kind: "source bytes",
            actual,
            limit,
        }
        | JxlDecodeError::Source(RawSourceError::TooLarge { actual, limit }) => {
            ImageInputError::SourceTooLarge { actual, limit }
        }
        JxlDecodeError::Limit {
            kind: "width",
            actual,
            limit,
        } => ImageInputError::WidthLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        JxlDecodeError::Limit {
            kind: "height",
            actual,
            limit,
        } => ImageInputError::HeightLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        JxlDecodeError::Limit {
            kind: "pixels",
            actual,
            limit,
        } => ImageInputError::PixelLimit { actual, limit },
        JxlDecodeError::Limit {
            kind: "decoded bytes",
            actual,
            limit,
        } => ImageInputError::DecodedByteLimit { actual, limit },
        JxlDecodeError::Limit {
            kind: "probe bytes",
            limit,
            ..
        } => ImageInputError::ProbeBudgetExceeded { limit },
        JxlDecodeError::AllocationFailure
        | JxlDecodeError::Source(RawSourceError::AllocationFailure) => {
            ImageInputError::AllocationFailure
        }
        JxlDecodeError::ArithmeticOverflow
        | JxlDecodeError::Source(RawSourceError::LengthConversion) => {
            ImageInputError::ArithmeticOverflow
        }
        JxlDecodeError::Source(RawSourceError::Changed) => ImageInputError::Io {
            message: "JPEG XL source changed while taking a stable snapshot".to_owned(),
        },
        JxlDecodeError::Source(RawSourceError::Read { offset, requested }) => ImageInputError::Io {
            message: format!("JPEG XL source read failed at {offset} for {requested} bytes"),
        },
        JxlDecodeError::Source(RawSourceError::Empty) => malformed("JPEG XL source is empty"),
        JxlDecodeError::NotJpegXl => ImageInputError::UnsupportedSignature {
            signature: Vec::new(),
        },
        other => malformed(&other.to_string()),
    }
}

fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::JpegXl,
        message: message.to_owned(),
    }
}

fn required_bytes(error: &rusttable_image::ImageViewError) -> usize {
    match error {
        rusttable_image::ImageViewError::BufferTooShort { required, .. } => *required,
        _ => 0,
    }
}
