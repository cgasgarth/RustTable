use super::{
    FINALSCALE_MAX_DIMENSION, FinalScaleLimits, FinalScalePlan, FinalScalePlanError,
    RenderSizeRequest,
};
use crate::RasterDimensions;
use rusttable_image::Roi;

pub(crate) struct ResolvedDimensions {
    pub(crate) output_dimensions: RasterDimensions,
    pub(crate) upscale_suppressed: bool,
}

pub(crate) fn resolve_dimensions(
    source: RasterDimensions,
    request: &RenderSizeRequest,
    allow_upscale: bool,
    limits: FinalScaleLimits,
) -> Result<ResolvedDimensions, FinalScalePlanError> {
    request.validate()?;
    let (requested_width, requested_height) = requested_dimensions(source, request)?;
    let (width, height, suppressed) = if allow_upscale {
        (requested_width, requested_height, false)
    } else {
        let scale = (f64::from(requested_width) / f64::from(source.width()))
            .min(f64::from(requested_height) / f64::from(source.height()));
        if requested_width > source.width() || requested_height > source.height() {
            let scale = scale.min(1.0);
            let (width, height) = scaled_pair(source, scale)?;
            (width, height, true)
        } else {
            (requested_width, requested_height, false)
        }
    };
    validate_dimensions(width, height, limits)?;
    Ok(ResolvedDimensions {
        output_dimensions: RasterDimensions::new(width, height)
            .map_err(|_| FinalScalePlanError::ArithmeticOverflow)?,
        upscale_suppressed: suppressed,
    })
}

fn requested_dimensions(
    source: RasterDimensions,
    request: &RenderSizeRequest,
) -> Result<(u32, u32), FinalScalePlanError> {
    let (width, height) = match request {
        RenderSizeRequest::Original => (source.width(), source.height()),
        RenderSizeRequest::Exact { width, height } => (*width, *height),
        RenderSizeRequest::FitWithin { width, height } => {
            let scale = (f64::from(*width) / f64::from(source.width()))
                .min(f64::from(*height) / f64::from(source.height()));
            scaled_pair(source, scale)?
        }
        RenderSizeRequest::LongEdge(edge) => {
            let source_long = source.width().max(source.height());
            scaled_pair(source, f64::from(*edge) / f64::from(source_long))?
        }
        RenderSizeRequest::ShortEdge(edge) => {
            let source_short = source.width().min(source.height());
            scaled_pair(source, f64::from(*edge) / f64::from(source_short))?
        }
        RenderSizeRequest::Megapixels(megapixels) => {
            let target_pixels = f64::from(megapixels.get()) * 1_000_000.0;
            megapixel_dimensions(source, target_pixels)?
        }
        RenderSizeRequest::Print {
            width_mm,
            height_mm,
            dpi,
        } => (
            round_dimension(f64::from(width_mm.get()) / 25.4 * f64::from(dpi.get()))?,
            round_dimension(f64::from(height_mm.get()) / 25.4 * f64::from(dpi.get()))?,
        ),
        RenderSizeRequest::PipelineScale(scale) => scaled_pair(source, f64::from(scale.get()))?,
    };
    Ok((width.max(1), height.max(1)))
}

fn megapixel_dimensions(
    source: RasterDimensions,
    target_pixels: f64,
) -> Result<(u32, u32), FinalScalePlanError> {
    if target_pixels < 1.0 || !target_pixels.is_finite() {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    let scale = (target_pixels / source.pixel_count() as f64).sqrt();
    let rounded = scaled_pair(source, scale)?;
    let rounded_pixels = u64::from(rounded.0) * u64::from(rounded.1);
    if rounded_pixels <= target_pixels.floor() as u64 {
        return Ok(rounded);
    }

    let target = target_pixels.floor() as u64;
    [
        (
            u32::try_from(target / u64::from(rounded.1)).unwrap_or(u32::MAX),
            rounded.1,
        ),
        (
            rounded.0,
            u32::try_from(target / u64::from(rounded.0)).unwrap_or(u32::MAX),
        ),
    ]
    .into_iter()
    .filter(|(width, height)| *width > 0 && *height > 0)
    .filter(|(width, height)| u64::from(*width) * u64::from(*height) <= target)
    .min_by(|left, right| {
        aspect_error(source, *left)
            .total_cmp(&aspect_error(source, *right))
            .then_with(|| {
                (u64::from(right.0) * u64::from(right.1))
                    .cmp(&(u64::from(left.0) * u64::from(left.1)))
            })
    })
    .ok_or(FinalScalePlanError::ArithmeticOverflow)
}

fn aspect_error(source: RasterDimensions, candidate: (u32, u32)) -> f64 {
    (f64::from(candidate.0) / f64::from(candidate.1)
        - f64::from(source.width()) / f64::from(source.height()))
    .abs()
}

fn scaled_pair(source: RasterDimensions, scale: f64) -> Result<(u32, u32), FinalScalePlanError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    Ok((
        round_dimension(f64::from(source.width()) * scale)?,
        round_dimension(f64::from(source.height()) * scale)?,
    ))
}

fn round_dimension(value: f64) -> Result<u32, FinalScalePlanError> {
    if !value.is_finite() || value <= 0.0 || value > f64::from(FINALSCALE_MAX_DIMENSION) {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    Ok((value.floor() as u32 + u32::from(value.fract() >= 0.5)).max(1))
}

fn validate_dimensions(
    width: u32,
    height: u32,
    limits: FinalScaleLimits,
) -> Result<(), FinalScalePlanError> {
    if width == 0
        || height == 0
        || width > FINALSCALE_MAX_DIMENSION
        || height > FINALSCALE_MAX_DIMENSION
    {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(FinalScalePlanError::ArithmeticOverflow)?;
    if pixels > limits.max_pixels() {
        return Err(FinalScalePlanError::OutputTooLarge {
            pixels,
            limit: limits.max_pixels(),
        });
    }
    let bytes = usize::try_from(pixels)
        .ok()
        .and_then(|value| value.checked_mul(16))
        .ok_or(FinalScalePlanError::ArithmeticOverflow)?;
    if bytes > limits.max_bytes() {
        return Err(FinalScalePlanError::OutputTooManyBytes {
            bytes,
            limit: limits.max_bytes(),
        });
    }
    Ok(())
}

pub(crate) fn roi_in(plan: &FinalScalePlan, output: Roi) -> Result<Roi, FinalScalePlanError> {
    validate_roi(output, plan.output_roi())?;
    let x = floor_scaled(output.x(), 1.0 / plan.scale_x())?;
    let y = floor_scaled(output.y(), 1.0 / plan.scale_y())?;
    let right =
        ceil_scaled(output.right(), 1.0 / plan.scale_x())?.min(plan.source_dimensions().width());
    let bottom =
        ceil_scaled(output.bottom(), 1.0 / plan.scale_y())?.min(plan.source_dimensions().height());
    let width = right
        .saturating_sub(x)
        .max(16)
        .min(plan.source_dimensions().width().saturating_sub(x));
    let height = bottom
        .saturating_sub(y)
        .max(16)
        .min(plan.source_dimensions().height().saturating_sub(y));
    Roi::new(x, y, width, height).map_err(|_| FinalScalePlanError::InvalidRoi)
}

pub(crate) fn roi_out(plan: &FinalScalePlan, input: Roi) -> Result<Roi, FinalScalePlanError> {
    validate_roi(input, plan.source_roi())?;
    let x = floor_scaled(input.x(), plan.scale_x())?;
    let y = floor_scaled(input.y(), plan.scale_y())?;
    let right = ceil_scaled(input.right(), plan.scale_x())?.min(plan.output_dimensions().width());
    let bottom =
        ceil_scaled(input.bottom(), plan.scale_y())?.min(plan.output_dimensions().height());
    Roi::new(
        x,
        y,
        right.saturating_sub(x).max(1),
        bottom.saturating_sub(y).max(1),
    )
    .map_err(|_| FinalScalePlanError::InvalidRoi)
}

pub(crate) fn transform_points(
    points: &mut [f32],
    scale_x: f64,
    scale_y: f64,
) -> Result<(), FinalScalePlanError> {
    if !points.len().is_multiple_of(2) {
        return Err(FinalScalePlanError::InvalidRoi);
    }
    for pair in points.as_chunks_mut::<2>().0 {
        if !pair[0].is_finite() || !pair[1].is_finite() {
            return Err(FinalScalePlanError::InvalidRoi);
        }
        let x = f64::from(pair[0]) * scale_x;
        let y = f64::from(pair[1]) * scale_y;
        if !x.is_finite() || !y.is_finite() {
            return Err(FinalScalePlanError::ArithmeticOverflow);
        }
        pair[0] = x as f32;
        pair[1] = y as f32;
    }
    Ok(())
}

fn validate_roi(roi: Roi, bounds: Roi) -> Result<(), FinalScalePlanError> {
    if roi.x() < bounds.x()
        || roi.y() < bounds.y()
        || roi.right() > bounds.right()
        || roi.bottom() > bounds.bottom()
    {
        Err(FinalScalePlanError::InvalidRoi)
    } else {
        Ok(())
    }
}

fn floor_scaled(value: u32, scale: f64) -> Result<u32, FinalScalePlanError> {
    let result = f64::from(value) * scale;
    if !result.is_finite() || result > f64::from(u32::MAX) {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    Ok(result.floor() as u32)
}

fn ceil_scaled(value: u32, scale: f64) -> Result<u32, FinalScalePlanError> {
    let result = f64::from(value) * scale;
    if !result.is_finite() || result > f64::from(u32::MAX) {
        return Err(FinalScalePlanError::ArithmeticOverflow);
    }
    Ok(result.ceil() as u32)
}
