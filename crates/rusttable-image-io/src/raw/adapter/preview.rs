//! Display-safe RAW development for the legacy decoded-image boundary.
//!
//! The stage order follows Darktable's `rawprepare` -> demosaic -> input color
//! profile -> tone/display transform lineage. Rawler supplies the bounded
//! sensor development primitives; `RustTable` owns the deterministic display
//! tone map and explicit sRGB output contract used by GTK previews.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use rawler::decoders::RawDecodeParams;
use rawler::imgop::{
    Rect,
    develop::{Intermediate, ProcessingStep, RawDevelop},
};
use rawler::rawsource::RawSource;
use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodedImage, DecodedImageError, ImageDimensions, ImageInputError,
    InputFormat, Orientation,
};

use super::{
    RawOrientation, backend_decode, raw_input_error, raw_limits, safe_text, selected_probe,
    validate_backend_declaration,
};

pub(super) fn developed_raw_preview(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<DecodedImage, ImageInputError> {
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
    let pixels = display_map(&pixels)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let dimensions =
        ImageDimensions::new(width, height).map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_output_limit(dimensions, limits)?;
    DecodedImage::new_with_source_orientation(
        dimensions,
        pixels,
        ColorEncoding::Srgb,
        image_orientation(orientation),
    )
    .map_err(decoded_image_error)
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

fn contains_rect(outer: Rect, inner: Rect) -> bool {
    inner.p.x >= outer.p.x
        && inner.p.y >= outer.p.y
        && inner.p.x.saturating_add(inner.d.w) <= outer.p.x.saturating_add(outer.d.w)
        && inner.p.y.saturating_add(inner.d.h) <= outer.p.y.saturating_add(outer.d.h)
}

fn linear_rgb(intermediate: Intermediate) -> Result<(u32, u32, Vec<[f32; 3]>), ImageInputError> {
    match intermediate {
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
    }
}

fn axis(value: usize) -> Result<u32, ImageInputError> {
    u32::try_from(value).map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn display_map(linear: &[[f32; 3]]) -> Vec<[u8; 4]> {
    let mut luminance = linear
        .iter()
        .step_by((linear.len() / 65_536).max(1))
        .map(|pixel| scene_luminance(*pixel))
        .filter(|value| value.is_finite() && *value > 0.000_001)
        .collect::<Vec<_>>();
    luminance.sort_by(f32::total_cmp);
    let reference = luminance
        .get(luminance.len().saturating_mul(3) / 5)
        .copied()
        .unwrap_or(0.18);
    let gain = (0.22 / reference.max(0.000_001)).clamp(0.25, 64.0);

    linear
        .iter()
        .map(|pixel| {
            let source_luminance = scene_luminance(*pixel).max(0.0);
            let mapped_luminance = 1.0 - (-source_luminance * gain).exp();
            let scale = if source_luminance > 0.000_001 {
                mapped_luminance / source_luminance
            } else {
                gain
            };
            [
                encode_srgb(pixel[0] * scale),
                encode_srgb(pixel[1] * scale),
                encode_srgb(pixel[2] * scale),
                255,
            ]
        })
        .collect()
}

fn scene_luminance(pixel: [f32; 3]) -> f32 {
    pixel[0].max(0.0) * 0.212_6 + pixel[1].max(0.0) * 0.715_2 + pixel[2].max(0.0) * 0.072_2
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is rounded and clamped to the complete u8 domain before quantization"
)]
fn encode_srgb(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    (encoded * 255.0).round().clamp(0.0, 255.0) as u8
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
