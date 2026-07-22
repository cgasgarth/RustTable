//! Linear RAW development shared by the native frame and legacy image boundaries.
//!
//! The stage order follows Darktable's `rawprepare` -> demosaic -> input color
//! profile lineage. Rawler supplies the bounded sensor development primitives;
//! display encoding remains an explicit downstream render concern.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use rawler::decoders::RawDecodeParams;
use rawler::imgop::{
    Rect,
    develop::{Intermediate, ProcessingStep, RawDevelop},
};
use rawler::rawsource::RawSource;
use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodeStage, DecodedImage, DecodedImageError, ImageDimensions,
    ImageInputError, InputFormat, Orientation,
};

use super::{
    RawOrientation, backend_decode, raw_input_error, raw_limits, safe_text, selected_probe,
    validate_backend_declaration,
};

pub(super) struct DevelopedRawLinear {
    pub(super) width: u32,
    pub(super) height: u32,
    pub(super) pixels: Vec<[f32; 4]>,
    pub(super) orientation: Orientation,
    pub(super) stages: Vec<DecodeStage>,
}

pub(super) fn developed_raw_preview(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
    let developed = developed_raw_linear(bytes, limits)?;
    let dimensions = ImageDimensions::new(developed.width, developed.height)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_output_limit(dimensions, limits)?;
    DecodedImage::new_with_source_orientation(
        dimensions,
        linear_rgba8(&developed.pixels),
        ColorEncoding::LinearSrgb,
        developed.orientation,
    )
    .map_err(decoded_image_error)
}

pub(super) fn developed_raw_linear(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DevelopedRawLinear, ImageInputError> {
    let raw_limits = raw_limits(limits)?;
    let probe =
        selected_probe(super::RawContainerRegistry::standard(), bytes).map_err(raw_input_error)?;
    let raw_source = RawSource::new_from_shared_vec(Arc::new(bytes.to_vec()));
    let params = RawDecodeParams { image_index: 0 };
    let dummy = backend_decode(&raw_source, &params, true)
        .map_err(|error| raw_input_error(super::map_backend_error(error, &probe)))?;
    validate_backend_declaration(&dummy, raw_limits, &probe).map_err(raw_input_error)?;
    let raw = backend_decode(&raw_source, &params, false)
        .map_err(|error| raw_input_error(super::map_backend_error(error, &probe)))?;
    validate_backend_declaration(&raw, raw_limits, &probe).map_err(raw_input_error)?;
    let orientation = super::orientation(raw.orientation);

    let developed = catch_unwind(AssertUnwindSafe(|| {
        RawDevelop {
            steps: development_steps(raw.active_area, raw.crop_area),
        }
        .develop_intermediate(&raw)
    }))
    .map_err(|_| malformed("RAW development backend panicked"))?
    .map_err(|error| malformed(&format!("RAW development failed: {error}")))?;

    let (width, height, pixels) = linear_rgb(developed)?;
    Ok(DevelopedRawLinear {
        width,
        height,
        pixels: pixels
            .into_iter()
            .map(|pixel| [pixel[0], pixel[1], pixel[2], 1.0])
            .collect(),
        orientation: image_orientation(orientation),
        stages: development_stages(raw.active_area, raw.crop_area),
    })
}

fn development_steps(active_area: Option<Rect>, crop_area: Option<Rect>) -> Vec<ProcessingStep> {
    let can_adapt_default_crop =
        active_area.is_none_or(|active| crop_area.is_none_or(|crop| contains_rect(active, crop)));
    let mut steps = vec![ProcessingStep::Rescale, ProcessingStep::Demosaic];
    if can_adapt_default_crop {
        steps.push(ProcessingStep::CropActiveArea);
    }
    steps.extend([
        ProcessingStep::WhiteBalance,
        ProcessingStep::Calibrate,
        ProcessingStep::CropDefault,
    ]);
    steps
}

fn development_stages(active_area: Option<Rect>, crop_area: Option<Rect>) -> Vec<DecodeStage> {
    let can_adapt_default_crop =
        active_area.is_none_or(|active| crop_area.is_none_or(|crop| contains_rect(active, crop)));
    let mut stages = vec![DecodeStage::RawRescale];
    if can_adapt_default_crop {
        stages.push(DecodeStage::RawActiveAreaCrop);
    }
    stages.extend([
        DecodeStage::RawCfa,
        DecodeStage::RawDemosaic,
        DecodeStage::RawWhiteBalance,
        DecodeStage::RawColorCalibration,
        DecodeStage::RawDefaultCrop,
    ]);
    stages
}

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    inner.p.x >= outer.p.x
        && inner.p.y >= outer.p.y
        && inner.p.x.saturating_add(inner.d.w) <= outer.p.x.saturating_add(outer.d.w)
        && inner.p.y.saturating_add(inner.d.h) <= outer.p.y.saturating_add(outer.d.h)
}

fn linear_rgb(intermediate: Intermediate) -> Result<(u32, u32, Vec<[f32; 3]>), ImageInputError> {
    let (width, height, pixels) = match intermediate {
        Intermediate::Monochrome(pixels) => Ok((
            axis(pixels.width)?,
            axis(pixels.height)?,
            pixels.data.into_iter().map(|value| [value; 3]).collect(),
        )),
        Intermediate::ThreeColor(pixels) => {
            Ok((axis(pixels.width)?, axis(pixels.height)?, pixels.data))
        }
        Intermediate::FourColor(pixels) => Ok((
            axis(pixels.width)?,
            axis(pixels.height)?,
            pixels
                .data
                .into_iter()
                .map(|value| [value[0], value[1].midpoint(value[3]), value[2]])
                .collect(),
        )),
    }?;
    if pixels
        .iter()
        .flat_map(|pixel| pixel.iter())
        .any(|value| !value.is_finite())
    {
        return Err(malformed("RAW development produced a non-finite sample"));
    }
    Ok((width, height, pixels))
}

fn axis(value: usize) -> Result<u32, ImageInputError> {
    u32::try_from(value).map_err(|_| ImageInputError::ArithmeticOverflow)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped to the complete u8 domain before quantization"
)]
pub(super) fn linear_rgba8(pixels: &[[f32; 4]]) -> Vec<u8> {
    pixels
        .iter()
        .flat_map(|pixel| {
            [
                quantize_linear(pixel[0]),
                quantize_linear(pixel[1]),
                quantize_linear(pixel[2]),
                255,
            ]
        })
        .collect()
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped to the complete u8 domain before quantization"
)]
fn quantize_linear(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

const fn image_orientation(orientation: RawOrientation) -> Orientation {
    match orientation {
        RawOrientation::HorizontalFlip => Orientation::FlipHorizontal,
        RawOrientation::Rotate180 => Orientation::Rotate180,
        RawOrientation::VerticalFlip => Orientation::FlipVertical,
        RawOrientation::Transpose => Orientation::Transpose,
        RawOrientation::Rotate90 => Orientation::Rotate90,
        RawOrientation::Transverse => Orientation::Transverse,
        RawOrientation::Rotate270 => Orientation::Rotate270,
        RawOrientation::Normal | RawOrientation::Unknown => Orientation::Normal,
    }
}

fn enforce_output_limit(
    dimensions: ImageDimensions,
    limits: DecodeLimits,
) -> Result<(), ImageInputError> {
    let actual = dimensions
        .decoded_byte_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if actual > limits.max_decoded_bytes() {
        return Err(ImageInputError::DecodedByteLimit {
            actual,
            limit: limits.max_decoded_bytes(),
        });
    }
    Ok(())
}

fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Raw,
        message: safe_text(message),
    }
}

fn decoded_image_error(error: DecodedImageError) -> ImageInputError {
    match error {
        DecodedImageError::ArithmeticOverflow => ImageInputError::ArithmeticOverflow,
        DecodedImageError::ByteLengthMismatch { expected, actual } => {
            ImageInputError::DecodedBufferInvariant { expected, actual }
        }
    }
}

#[cfg(test)]
mod tests {
    use rawler::imgop::{Dim2, Point, develop::ProcessingStep};

    use super::{Rect, contains_rect, development_steps};

    #[test]
    fn preserves_full_sensor_for_a_default_crop_wider_than_active_area() {
        let active = Rect::new(Point::new(0, 0), Dim2::new(5_920, 4_038));
        let crop = Rect::new(Point::new(16, 21), Dim2::new(6_000, 4_000));

        assert!(!contains_rect(active, crop));
        assert!(
            !development_steps(Some(active), Some(crop)).contains(&ProcessingStep::CropActiveArea)
        );
    }

    #[test]
    fn adapts_default_crop_when_it_is_inside_active_area() {
        let active = Rect::new(Point::new(0, 0), Dim2::new(6_000, 4_000));
        let crop = Rect::new(Point::new(16, 21), Dim2::new(5_900, 3_900));

        assert!(contains_rect(active, crop));
        assert!(
            development_steps(Some(active), Some(crop)).contains(&ProcessingStep::CropActiveArea)
        );
    }
}
