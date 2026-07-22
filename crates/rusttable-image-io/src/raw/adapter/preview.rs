//! Linear RAW development shared by the native frame and legacy image boundaries.
//!
//! The stage order follows Darktable's `rawprepare` -> as-shot white balance
//! -> demosaic -> input color profile lineage. Rawler supplies the bounded
//! sensor development primitives; persisted white balance belongs to
//! processing.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use rawler::cfa::{CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R};
use rawler::decoders::RawDecodeParams;
use rawler::imgop::{
    Rect,
    develop::{Intermediate, ProcessingStep, RawDevelop},
};
use rawler::rawimage::{RawImage, RawPhotometricInterpretation};
use rawler::rawsource::RawSource;
use rusttable_color::{BuiltinSpace, Matrix3};
use rusttable_image::{
    BlackWhiteLevels, CfaColor, CfaPattern, CfaPhase, ColorEncoding, DecodeLimits, DecodeStage,
    DecodedImage, DecodedImageError, ImageDimensions, ImageInputError, InputFormat, Orientation,
    RawMosaic, RawMosaicSource, Roi,
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
    pub(super) raw_source: Option<RawMosaicSource>,
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
    let raw_result = super::RawlerRawDecoder::new()
        .decode_bytes(bytes, &super::RawDecodeRequest::new(raw_limits))
        .map_err(raw_input_error)?;
    let raw_source = match raw_result.frame.parts().planes.first() {
        Some(plane) if matches!(&plane.layout, super::RawPlaneLayout::Mosaic(_)) => {
            Some(raw_mosaic_source(&raw_result.frame)?)
        }
        _ => None,
    };
    let orientation = super::orientation(raw.orientation);

    let (width, height, pixels) = if is_xtrans(&raw_result.frame) {
        developed_xtrans_linear(&raw, raw_result.frame.parts())?
    } else {
        let developed = catch_unwind(AssertUnwindSafe(|| {
            RawDevelop {
                steps: development_steps(raw.active_area, raw.crop_area),
            }
            .develop_intermediate(&raw)
        }))
        .map_err(|_| malformed("RAW development backend panicked"))?
        .map_err(|error| malformed(&format!("RAW development failed: {error}")))?;
        linear_rgb(developed)?
    };
    if pixels
        .iter()
        .flat_map(|pixel| pixel.iter())
        .any(|value| !value.is_finite())
    {
        return Err(malformed("RAW development produced a non-finite sample"));
    }
    Ok(DevelopedRawLinear {
        width,
        height,
        pixels: pixels
            .into_iter()
            .map(|pixel| [pixel[0], pixel[1], pixel[2], 1.0])
            .collect(),
        orientation: image_orientation(orientation),
        stages: development_stages(raw.active_area, raw.crop_area),
        raw_source,
    })
}

fn is_xtrans(frame: &super::RawFrame) -> bool {
    matches!(
        frame.parts().planes.first().map(|plane| &plane.layout),
        Some(super::RawPlaneLayout::Mosaic(cfa)) if cfa.width == 6 && cfa.height == 6
    )
}

fn developed_xtrans_linear(
    raw: &RawImage,
    parts: &super::RawFrameParts,
) -> Result<(u32, u32, Vec<[f32; 3]>), ImageInputError> {
    let mut raw = raw.clone();
    raw.apply_scaling()
        .map_err(|error| malformed(&format!("X-Trans raw preparation failed: {error}")))?;
    let RawPhotometricInterpretation::Cfa(config) = &raw.photometric else {
        return Err(malformed("X-Trans RAW photometric metadata is not CFA"));
    };
    let wb = raw.wb_coeffs;
    let samples = raw
        .data
        .as_f32()
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let x = index % raw.width;
            let y = index / raw.width;
            let gain = match config.cfa.cfa_color_at(y, x) as usize {
                CFA_COLOR_R => wb[0],
                CFA_COLOR_G => wb[1],
                CFA_COLOR_B => wb[2],
                _ => 1.0,
            };
            *value * gain
        })
        .collect::<Vec<_>>();
    // X-Trans cameras commonly report an active area narrower than the default
    // crop (the X-Pro2 reports 5920x4038 vs. 6000x4000). Demosaic the complete
    // sensor so the default crop remains valid and phase stays at the sensor origin.
    let pixels = xtrans_demosaic(&samples, raw.width, raw.height, &config.cfa);
    let matrix = camera_to_srgb_matrix(parts)
        .ok_or_else(|| malformed("X-Trans camera color matrix is missing"))?;
    let crop_active_area = raw
        .active_area
        .is_none_or(|active| raw.crop_area.is_none_or(|crop| contains_rect(active, crop)));
    let output_area = if crop_active_area {
        raw.crop_area
            .map(|crop| raw.active_area.map_or(crop, |active| crop.adapt(&active)))
            .or(raw.active_area)
    } else {
        raw.crop_area
    };
    let (pixels, width, height) = if let Some(crop) = output_area {
        let cropped = crop_rgb(&pixels, raw.width, raw.height, crop);
        (cropped, crop.d.w, crop.d.h)
    } else {
        (pixels, raw.width, raw.height)
    };
    let pixels = pixels
        .into_iter()
        .map(|pixel| matrix.apply(pixel))
        .collect::<Vec<_>>();
    Ok((
        u32::try_from(width).map_err(|_| ImageInputError::ArithmeticOverflow)?,
        u32::try_from(height).map_err(|_| ImageInputError::ArithmeticOverflow)?,
        pixels,
    ))
}

fn xtrans_demosaic(
    samples: &[f32],
    width: usize,
    height: usize,
    cfa: &rawler::cfa::CFA,
) -> Vec<[f32; 3]> {
    // Unlike rawler's PPG implementation, this interpolation follows the
    // 6x6 CFA phase and never treats X-Trans as a 2x2 Bayer pattern. The
    // closest samples of each color are distance-weighted; calibration remains
    // a separate matrix step below so the linear boundary is explicit.
    let offsets = xtrans_offsets(cfa);
    let mut output = Vec::with_capacity(width.saturating_mul(height));
    for y in 0..height {
        for x in 0..width {
            let phase = (y % 6) * 6 + (x % 6);
            let mut pixel = [0.0; 3];
            for channel in 0..3 {
                let mut sum = 0.0;
                let mut weight_sum = 0.0;
                for &(dx, dy) in &offsets[phase][channel] {
                    let Some(nx) = x.checked_add_signed(dx) else {
                        continue;
                    };
                    let Some(ny) = y.checked_add_signed(dy) else {
                        continue;
                    };
                    if nx < width && ny < height {
                        let distance =
                            f32::from(u8::try_from(dx * dx + dy * dy).expect("bounded CFA offset"));
                        let weight = 1.0 / distance.max(1.0);
                        sum += samples[ny * width + nx] * weight;
                        weight_sum += weight;
                        if weight_sum >= 4.0 {
                            break;
                        }
                    }
                }
                pixel[channel] = if weight_sum == 0.0 {
                    0.0
                } else {
                    sum / weight_sum
                };
            }
            output.push(pixel);
        }
    }
    output
}

fn xtrans_offsets(cfa: &rawler::cfa::CFA) -> Vec<[Vec<(isize, isize)>; 3]> {
    let mut offsets = Vec::with_capacity(36);
    for phase_y in 0..6 {
        for phase_x in 0..6 {
            let mut channels = [const { Vec::new() }; 3];
            for dy in -3..=3 {
                for dx in -3..=3 {
                    let color = cfa.color_at(
                        (phase_y as isize + dy).rem_euclid(6) as usize,
                        (phase_x as isize + dx).rem_euclid(6) as usize,
                    );
                    if color < 3 {
                        channels[color].push((dx, dy));
                    }
                }
            }
            for channel in &mut channels {
                channel.sort_by_key(|(dx, dy)| dx * dx + dy * dy);
            }
            offsets.push(channels);
        }
    }
    offsets
}

fn crop_rgb(pixels: &[[f32; 3]], width: usize, height: usize, crop: Rect) -> Vec<[f32; 3]> {
    let x = crop.p.x;
    let y = crop.p.y;
    let crop_width = crop.d.w;
    let crop_height = crop.d.h;
    let mut output = Vec::with_capacity(crop_width.saturating_mul(crop_height));
    for row in y..y.saturating_add(crop_height).min(height) {
        let start = row * width + x.min(width);
        let end = start.saturating_add(crop_width).min(row * width + width);
        output.extend_from_slice(&pixels[start..end]);
    }
    output
}

fn camera_to_srgb_matrix(parts: &super::RawFrameParts) -> Option<Matrix3> {
    let matrix = parts
        .color_matrices
        .iter()
        .find(|matrix| matrix.illuminant == super::RawIlluminant::D65)
        .or_else(|| parts.color_matrices.first())?;
    if matrix.rows != 3 || matrix.columns != 3 {
        return None;
    }
    let camera_to_xyz: &[f32; 9] = matrix.coefficients.as_slice().try_into().ok()?;
    let srgb_to_xyz = BuiltinSpace::SrgbD65.to_xyz_matrix()?.rows();
    let mut rgb_to_camera = [0.0_f32; 9];
    for row in 0..3 {
        for column in 0..3 {
            rgb_to_camera[row * 3 + column] = (0..3)
                .map(|index| camera_to_xyz[row * 3 + index] * srgb_to_xyz[index * 3 + column])
                .sum();
        }
        let sum: f32 = rgb_to_camera[row * 3..row * 3 + 3].iter().sum();
        if sum == 0.0 || !sum.is_finite() {
            return None;
        }
        for value in &mut rgb_to_camera[row * 3..row * 3 + 3] {
            *value /= sum;
        }
    }
    Matrix3::new(rgb_to_camera).ok()?.inverse().ok()
}

fn development_steps(active_area: Option<Rect>, crop_area: Option<Rect>) -> Vec<ProcessingStep> {
    let can_adapt_default_crop =
        active_area.is_none_or(|active| crop_area.is_none_or(|crop| contains_rect(active, crop)));
    let mut steps = vec![ProcessingStep::Rescale];
    if can_adapt_default_crop {
        steps.push(ProcessingStep::CropActiveArea);
    }
    steps.extend([
        ProcessingStep::WhiteBalance,
        ProcessingStep::Demosaic,
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
        DecodeStage::RawWhiteBalance,
        DecodeStage::RawCfa,
        DecodeStage::RawDemosaic,
        DecodeStage::RawColorCalibration,
        DecodeStage::RawDefaultCrop,
    ]);
    stages
}

fn raw_mosaic_source(frame: &super::RawFrame) -> Result<RawMosaicSource, ImageInputError> {
    let parts = frame.parts();
    let plane = parts
        .planes
        .first()
        .ok_or_else(|| malformed("RAW frame has no sensor plane"))?;
    let super::RawPlaneLayout::Mosaic(cfa) = &plane.layout else {
        return Err(malformed("RAW temperature requires a CFA sensor plane"));
    };
    let pattern = match (cfa.width, cfa.height) {
        (2, 2) => CfaPattern::Bayer([
            [cfa_color(cfa.pattern[0])?, cfa_color(cfa.pattern[1])?],
            [cfa_color(cfa.pattern[2])?, cfa_color(cfa.pattern[3])?],
        ]),
        (6, 6) => {
            let mut pattern = [[CfaColor::Green; 6]; 6];
            for (row, cells) in pattern.iter_mut().enumerate() {
                for (column, cell) in cells.iter_mut().enumerate() {
                    *cell = cfa_color(cfa.pattern[row * 6 + column])?;
                }
            }
            CfaPattern::XTrans(pattern)
        }
        _ => return Err(malformed("RAW CFA pattern is neither Bayer nor X-Trans")),
    };
    let phase = CfaPhase::new(u32::from(cfa.phase_x), u32::from(cfa.phase_y), pattern);
    let black = level_u16(
        parts
            .black_levels
            .values
            .first()
            .copied()
            .ok_or_else(|| malformed("RAW black level is missing"))?,
        "black",
    )?;
    let white = level_u16(
        parts
            .white_levels
            .first()
            .copied()
            .ok_or_else(|| malformed("RAW white level is missing"))?,
        "white",
    )?;
    let levels = BlackWhiteLevels::new(black, white)
        .map_err(|_| malformed("RAW white level is not above black level"))?;
    let dimensions = ImageDimensions::new(plane.dimensions.width, plane.dimensions.height)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let mosaic = RawMosaic::new(
        dimensions,
        usize::try_from(plane.dimensions.width).map_err(|_| ImageInputError::ArithmeticOverflow)?,
        plane.samples.clone(),
        pattern,
        phase,
        levels,
        image_orientation(parts.orientation),
    )
    .map_err(|error| malformed(&format!("RAW mosaic conversion failed: {error}")))?;
    let active_area = Roi::new(
        parts.active_area.x,
        parts.active_area.y,
        parts.active_area.width,
        parts.active_area.height,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?
    .within(dimensions)
    .map_err(|_| malformed("RAW active area is outside the sensor plane"))?;
    let default_crop = Roi::new(
        parts.crop_area.x,
        parts.crop_area.y,
        parts.crop_area.width,
        parts.crop_area.height,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?
    .within(dimensions)
    .map_err(|_| malformed("RAW default crop is outside the sensor plane"))?;
    Ok(RawMosaicSource::new(mosaic, Some(active_area)).with_default_crop(Some(default_crop)))
}

fn cfa_color(channel: super::RawChannel) -> Result<CfaColor, ImageInputError> {
    match channel {
        super::RawChannel::Red => Ok(CfaColor::Red),
        super::RawChannel::Green | super::RawChannel::FujiGreen => Ok(CfaColor::Green),
        super::RawChannel::Blue => Ok(CfaColor::Blue),
        super::RawChannel::Unknown => Err(malformed("RAW CFA contains an unknown channel")),
        _ => Err(malformed("RAW CFA contains an unsupported channel")),
    }
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the range and non-negative integer value are checked above"
)]
fn level_u16(value: f32, name: &str) -> Result<u16, ImageInputError> {
    if !value.is_finite() || value < 0.0 || value > f32::from(u16::MAX) || value.fract() != 0.0 {
        return Err(malformed(&format!(
            "RAW {name} level is not an integer U16 value"
        )));
    }
    Ok(value as u16)
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

    use super::{Rect, contains_rect, development_steps, xtrans_demosaic};

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

    #[test]
    fn decoder_development_applies_declared_as_shot_white_balance() {
        let steps = development_steps(None, None);
        assert_eq!(
            steps,
            vec![
                ProcessingStep::Rescale,
                ProcessingStep::CropActiveArea,
                ProcessingStep::WhiteBalance,
                ProcessingStep::Demosaic,
                ProcessingStep::Calibrate,
                ProcessingStep::CropDefault
            ]
        );
    }

    #[test]
    fn xtrans_demosaic_preserves_all_three_channels_at_full_resolution() {
        let cfa = rawler::cfa::CFA::new("GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG");
        let samples = (0..36)
            .map(|index| 0.1 + f32::from(u8::try_from(index).expect("small test sample")) / 100.0)
            .collect::<Vec<_>>();
        let pixels = xtrans_demosaic(&samples, 6, 6, &cfa);

        assert_eq!(pixels.len(), 36);
        assert!(
            pixels
                .iter()
                .all(|pixel| { pixel.iter().all(|value| value.is_finite() && *value > 0.0) })
        );
    }
}
