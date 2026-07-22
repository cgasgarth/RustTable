//! Explicit color-space boundaries for Lab-native processing operations.

use std::fmt;

use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    ColorEncoding, ColorRole, ColorTransformPlanner, ColorTransformRequest,
    ColorTransformRequestError, ExtendedRange, Precision, RenderingIntent, TransformExecutionError,
    TransformPlan,
};

use crate::operations::defringe::{
    DefringeConfig, DefringeExecutionError, DefringePixel, DefringePlan,
};
use crate::operations::relight::{RelightConfig, RelightPixel, RelightPlan};
use crate::operations::shadhi::{ShadhiConfig, ShadhiPixel, ShadhiPlan};
use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel};

const COLOR_PLANNER_VERSION: u16 = 1;

#[derive(Debug)]
pub(crate) enum LabBoundaryError {
    Request(ColorTransformRequestError),
    Planner(rusttable_color::PlannerError),
    Transform(TransformExecutionError),
    Defringe(DefringeExecutionError),
    Shadhi(crate::operations::OperationExecutionError),
    Relight(crate::operations::OperationExecutionError),
    NonFinite { pixel: usize, channel: RgbChannel },
}

impl fmt::Display for LabBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(error) => write!(formatter, "Lab boundary request failed: {error}"),
            Self::Planner(error) => write!(formatter, "Lab boundary planning failed: {error}"),
            Self::Transform(error) => write!(formatter, "Lab boundary transform failed: {error}"),
            Self::Defringe(error) => write!(formatter, "Lab operation failed: {error}"),
            Self::Shadhi(error) => write!(formatter, "Lab shadhi operation failed: {error}"),
            Self::Relight(error) => write!(formatter, "Lab relight operation failed: {error}"),
            Self::NonFinite { pixel, channel } => write!(
                formatter,
                "Lab boundary produced a non-finite {channel:?} at pixel {pixel}"
            ),
        }
    }
}

impl std::error::Error for LabBoundaryError {}

pub(crate) fn apply_defringe(
    config: DefringeConfig,
    pixels: &[LinearRgb],
    dimensions: RasterDimensions,
    source_encoding: ColorEncoding,
    opacity: f32,
) -> Result<Vec<LinearRgb>, LabBoundaryError> {
    let to_lab = plan(source_encoding, ColorEncoding::LabD50)?;
    let from_lab = plan(ColorEncoding::LabD50, source_encoding)?;
    let lab_pixels = pixels
        .iter()
        .copied()
        .map(|value| {
            let lab = to_lab
                .apply_rgb(
                    [value.red().get(), value.green().get(), value.blue().get()],
                    || false,
                )
                .map_err(LabBoundaryError::Transform)?;
            Ok(DefringePixel::new(lab[0], lab[1], lab[2], 1.0))
        })
        .collect::<Result<Vec<_>, LabBoundaryError>>()?;
    let plan =
        DefringePlan::new(config, dimensions, 1.0, 1.0).map_err(LabBoundaryError::Defringe)?;
    let (lab_output, _) = plan
        .execute_with_receipt(&lab_pixels, None, opacity, || false)
        .map_err(LabBoundaryError::Defringe)?;
    lab_output
        .into_iter()
        .enumerate()
        .map(|(pixel, value)| {
            let channels = value.channels();
            let rgb = from_lab
                .apply_rgb([channels[0], channels[1], channels[2]], || false)
                .map_err(LabBoundaryError::Transform)?;
            let red = finite(rgb[0], pixel, RgbChannel::Red)?;
            let green = finite(rgb[1], pixel, RgbChannel::Green)?;
            let blue = finite(rgb[2], pixel, RgbChannel::Blue)?;
            Ok(LinearRgb::new(red, green, blue))
        })
        .collect()
}

pub(crate) fn apply_shadhi(
    config: ShadhiConfig,
    pixels: &[LinearRgb],
    dimensions: RasterDimensions,
    source_encoding: ColorEncoding,
    opacity: f32,
) -> Result<Vec<LinearRgb>, LabBoundaryError> {
    let to_lab = plan(source_encoding, ColorEncoding::LabD50)?;
    let from_lab = plan(ColorEncoding::LabD50, source_encoding)?;
    let lab_pixels = pixels
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel, value)| {
            let lab = to_lab
                .apply_rgb(
                    [value.red().get(), value.green().get(), value.blue().get()],
                    || false,
                )
                .map_err(LabBoundaryError::Transform)?;
            if lab.iter().any(|channel| !channel.is_finite()) {
                return Err(LabBoundaryError::NonFinite {
                    pixel,
                    channel: RgbChannel::Red,
                });
            }
            Ok(ShadhiPixel::new(lab[0], lab[1], lab[2], 1.0))
        })
        .collect::<Result<Vec<_>, LabBoundaryError>>()?;
    let plan = ShadhiPlan::new(config, dimensions).map_err(LabBoundaryError::Shadhi)?;
    let lab_output = plan
        .execute_lab(&lab_pixels, None, opacity, || false)
        .map_err(LabBoundaryError::Shadhi)?;
    lab_output
        .into_iter()
        .enumerate()
        .map(|(pixel, value)| {
            let channels = value.channels();
            let rgb = from_lab
                .apply_rgb([channels[0], channels[1], channels[2]], || false)
                .map_err(LabBoundaryError::Transform)?;
            let red = finite(rgb[0], pixel, RgbChannel::Red)?;
            let green = finite(rgb[1], pixel, RgbChannel::Green)?;
            let blue = finite(rgb[2], pixel, RgbChannel::Blue)?;
            Ok(LinearRgb::new(red, green, blue))
        })
        .collect()
}

pub(crate) fn apply_relight(
    config: RelightConfig,
    pixels: &[LinearRgb],
    dimensions: RasterDimensions,
    source_encoding: ColorEncoding,
    opacity: f32,
) -> Result<Vec<LinearRgb>, LabBoundaryError> {
    let to_lab = plan(source_encoding, ColorEncoding::LabD50)?;
    let from_lab = plan(ColorEncoding::LabD50, source_encoding)?;
    let lab_pixels = pixels
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel, value)| {
            let lab = to_lab
                .apply_rgb(
                    [value.red().get(), value.green().get(), value.blue().get()],
                    || false,
                )
                .map_err(LabBoundaryError::Transform)?;
            if lab.iter().any(|channel| !channel.is_finite()) {
                return Err(LabBoundaryError::NonFinite {
                    pixel,
                    channel: RgbChannel::Red,
                });
            }
            Ok(RelightPixel::new(lab[0], lab[1], lab[2], 1.0))
        })
        .collect::<Result<Vec<_>, LabBoundaryError>>()?;
    let plan = RelightPlan::new(config, dimensions);
    let lab_output = plan
        .execute_lab(&lab_pixels, opacity, || false)
        .map_err(LabBoundaryError::Relight)?;
    lab_output
        .into_iter()
        .enumerate()
        .map(|(pixel, value)| {
            let channels = value.channels();
            let rgb = from_lab
                .apply_rgb([channels[0], channels[1], channels[2]], || false)
                .map_err(LabBoundaryError::Transform)?;
            let red = finite(rgb[0], pixel, RgbChannel::Red)?;
            let green = finite(rgb[1], pixel, RgbChannel::Green)?;
            let blue = finite(rgb[2], pixel, RgbChannel::Blue)?;
            Ok(LinearRgb::new(red, green, blue))
        })
        .collect()
}

fn plan(source: ColorEncoding, target: ColorEncoding) -> Result<TransformPlan, LabBoundaryError> {
    let request = ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        COLOR_PLANNER_VERSION,
    )
    .map_err(LabBoundaryError::Request)?;
    BuiltinColorTransformPlanner
        .plan(&request)
        .map_err(LabBoundaryError::Planner)
}

fn finite(value: f32, pixel: usize, channel: RgbChannel) -> Result<FiniteF32, LabBoundaryError> {
    FiniteF32::new(value).map_err(|_| LabBoundaryError::NonFinite { pixel, channel })
}
