//! Shared linear-light preview resampling over the processing `FinalScale` plan.

use std::fmt;

use rusttable_image::ImageDimensions;
use rusttable_processing::operations::finalscale::{
    FinalScaleConfig, FinalScaleExecutionError, FinalScalePlan, FinalScalePlanError, RenderQuality,
    RenderSizeRequest,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage};

use crate::{RenderAlphaPolicy, RenderBorderPolicy, RenderPlan, RenderResampling};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResamplingError {
    Plan(FinalScalePlanError),
    Execution(FinalScaleExecutionError),
    AlphaLength {
        expected: u64,
        actual: usize,
    },
    Image(rusttable_processing::ImageBuildError),
    NonFiniteResult {
        pixel: usize,
        channel: ResamplingChannel,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResamplingChannel {
    Red,
    Green,
    Blue,
}

impl fmt::Display for ResamplingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Plan(source) => write!(formatter, "preview resampling plan failed: {source}"),
            Self::Execution(source) => {
                write!(formatter, "preview resampling execution failed: {source}")
            }
            Self::AlphaLength { expected, actual } => write!(
                formatter,
                "preview resampling alpha has {actual} values; expected {expected}"
            ),
            Self::Image(source) => write!(formatter, "preview resampling image failed: {source:?}"),
            Self::NonFiniteResult { pixel, channel } => write!(
                formatter,
                "preview resampling produced a non-finite {channel:?} at pixel {pixel}"
            ),
        }
    }
}

impl std::error::Error for ResamplingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(source) => Some(source),
            Self::Execution(source) => Some(source),
            Self::AlphaLength { .. } | Self::Image(_) | Self::NonFiniteResult { .. } => None,
        }
    }
}

/// Resamples a completed linear-light frame and its straight alpha plane.
///
/// RGB is premultiplied before filtering and restored to straight form after
/// filtering. The shared `FinalScale` plan owns normalized taps, support,
/// reflected borders, ROI mapping, and deterministic tiled execution.
pub(crate) fn resample_working(
    input: &WorkingRgbImage,
    alpha: &[f32],
    plan: RenderPlan,
) -> Result<(WorkingRgbImage, Vec<f32>), ResamplingError> {
    let expected = input.dimensions().pixel_count();
    if u64::try_from(alpha.len()) != Ok(expected) {
        return Err(ResamplingError::AlphaLength {
            expected,
            actual: alpha.len(),
        });
    }
    if plan.sampling() == crate::RenderSampling::Identity {
        return Ok((input.clone(), alpha.to_vec()));
    }

    let policy = plan
        .resampling()
        .expect("filtered render plans carry an explicit resampling policy");
    let resampler = build_plan(input.dimensions(), plan.output_dimensions(), policy)?;
    let width = usize::try_from(input.dimensions().width())
        .map_err(|_| ResamplingError::Plan(FinalScalePlanError::ArithmeticOverflow))?;
    let mut premultiplied = Vec::with_capacity(input.pixel_slice().len() * 3);
    let mut source_alpha = Vec::with_capacity(alpha.len());
    for (pixel, &alpha) in input.pixel_slice().iter().zip(alpha) {
        let alpha = alpha.clamp(0.0, 1.0);
        premultiplied.extend([
            pixel.red().get() * alpha,
            pixel.green().get() * alpha,
            pixel.blue().get() * alpha,
        ]);
        source_alpha.push(alpha);
    }
    let filtered_rgb = resampler
        .execute_interleaved(&premultiplied, 3, width * 3)
        .map_err(ResamplingError::Execution)?;
    let filtered_alpha = resampler
        .execute_interleaved(&source_alpha, 1, width)
        .map_err(ResamplingError::Execution)?;
    let mut pixels = Vec::with_capacity(filtered_alpha.len());
    for (index, &alpha) in filtered_alpha.iter().enumerate() {
        let alpha = alpha.clamp(0.0, 1.0);
        let denominator = if alpha > f32::EPSILON { alpha } else { 1.0 };
        let base = index * 3;
        pixels.push(LinearRgb::new(
            finite_channel(
                filtered_rgb[base] / denominator,
                index,
                ResamplingChannel::Red,
            )?,
            finite_channel(
                filtered_rgb[base + 1] / denominator,
                index,
                ResamplingChannel::Green,
            )?,
            finite_channel(
                filtered_rgb[base + 2] / denominator,
                index,
                ResamplingChannel::Blue,
            )?,
        ));
    }
    let dimensions = RasterDimensions::new(
        plan.output_dimensions().width(),
        plan.output_dimensions().height(),
    )
    .expect("render plans contain nonzero dimensions");
    let output = WorkingRgbImage::new_with_frame(dimensions, pixels, input.frame())
        .map_err(ResamplingError::Image)?;
    Ok((output, filtered_alpha))
}

fn build_plan(
    source: RasterDimensions,
    output: ImageDimensions,
    policy: RenderResampling,
) -> Result<FinalScalePlan, ResamplingError> {
    debug_assert_eq!(policy.border(), RenderBorderPolicy::Reflect);
    debug_assert_eq!(policy.alpha(), RenderAlphaPolicy::Premultiplied);
    let request = RenderSizeRequest::exact(output.width(), output.height());
    FinalScalePlan::from_config(
        source,
        FinalScaleConfig::new(request).with_quality(RenderQuality::preview(policy.filter())),
    )
    .map_err(ResamplingError::Plan)
}

fn finite_channel(
    value: f32,
    pixel: usize,
    channel: ResamplingChannel,
) -> Result<FiniteF32, ResamplingError> {
    FiniteF32::new(value).map_err(|_| ResamplingError::NonFiniteResult { pixel, channel })
}
