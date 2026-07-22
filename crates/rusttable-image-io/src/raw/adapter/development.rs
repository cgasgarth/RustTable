//! Native and compatibility frame construction for developed RAW samples.

use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncoding, DecodeLimits, DecodeReceipt, DecodedFrame,
    ImageDimensions, ImageInputError, InputFormat, OwnedImage, PixelFormat, SampleType,
    SourceColor, StorageLayout,
};

use super::preview;

pub(crate) fn decode_linear_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    let developed = preview::developed_raw_linear(bytes, limits)?;
    let dimensions = ImageDimensions::new(developed.width, developed.height)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let pixel_count = dimensions
        .pixel_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let actual = pixel_count
        .checked_mul(16)
        .ok_or(ImageInputError::ArithmeticOverflow)?;
    if actual > limits.max_typed_frame_bytes() {
        return Err(ImageInputError::DecodedByteLimit {
            actual,
            limit: limits.max_typed_frame_bytes(),
        });
    }
    let pixels = developed
        .pixels
        .into_iter()
        .flat_map(|pixel| pixel.into_iter().flat_map(f32::to_ne_bytes))
        .collect::<Vec<_>>();
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let descriptor = rusttable_image::ImageDescriptor::new(
        dimensions,
        format,
        ColorEncoding::LinearSrgb,
        developed.orientation,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let owned = OwnedImage::new(descriptor.clone(), pixels).map_err(|_| {
        ImageInputError::DecodedBufferInvariant {
            expected: u64::try_from(descriptor.byte_length()).unwrap_or(u64::MAX),
            actual: 0,
        }
    })?;
    let source_color = SourceColor::declared(ColorEncoding::LinearSrgb).map_err(|_| {
        ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            message: "RAW linear source color could not be declared".to_owned(),
        }
    })?;
    let source_bytes =
        u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let receipt = DecodeReceipt::new_with_source_color(
        InputFormat::Raw,
        source_bytes,
        descriptor,
        source_color,
    )
    .map_err(|reason| ImageInputError::DecodeContract { reason })?
    .with_processing_stages(developed.stages);
    DecodedFrame::new(owned, receipt).map_err(|reason| ImageInputError::DecodeContract { reason })
}

pub(crate) fn decode_legacy_frame(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedFrame, ImageInputError> {
    let developed = preview::developed_raw_linear(bytes, limits)?;
    let dimensions = ImageDimensions::new(developed.width, developed.height)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let pixels = preview::linear_rgba8(&developed.pixels);
    let format = PixelFormat::rgba8();
    let descriptor = rusttable_image::ImageDescriptor::new(
        dimensions,
        format,
        ColorEncoding::LinearSrgb,
        developed.orientation,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let owned = OwnedImage::new(descriptor.clone(), pixels).map_err(|_| {
        ImageInputError::DecodedBufferInvariant {
            expected: u64::try_from(descriptor.byte_length()).unwrap_or(u64::MAX),
            actual: 0,
        }
    })?;
    let source_color = SourceColor::declared(ColorEncoding::LinearSrgb).map_err(|_| {
        ImageInputError::MalformedInput {
            format: InputFormat::Raw,
            message: "RAW linear source color could not be declared".to_owned(),
        }
    })?;
    let source_bytes =
        u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let receipt = DecodeReceipt::new_with_source_color(
        InputFormat::Raw,
        source_bytes,
        descriptor,
        source_color,
    )
    .map_err(|reason| ImageInputError::DecodeContract { reason })?
    .with_processing_stages(developed.stages);
    DecodedFrame::new(owned, receipt).map_err(|reason| ImageInputError::DecodeContract { reason })
}
