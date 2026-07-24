//! Explicit color-space boundaries for Lab-native processing operations.

use std::fmt;

use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    ColorEncoding, ColorRole, ColorTransformPlanner, ColorTransformRequest,
    ColorTransformRequestError, ExtendedRange, Precision, RenderingIntent, TransformExecutionError,
    TransformPlan,
};

use crate::operations::OperationExecutionError;
use crate::operations::bloom::{BloomConfig, BloomPixel, BloomPlan};
use crate::operations::defringe::{
    DefringeConfig, DefringeExecutionError, DefringePixel, DefringePlan,
};
use crate::operations::relight::{RelightConfig, RelightPixel, RelightPlan};
use crate::operations::shadhi::{
    ShadhiAlgorithm, ShadhiBilateralRequest, ShadhiConfig, ShadhiPixel, ShadhiPlan,
};
use crate::{FiniteF32, LinearRgb, RasterDimensions, RgbChannel, WorkingRgbImage};

const COLOR_PLANNER_VERSION: u16 = 1;

#[derive(Debug)]
pub(crate) enum LabBoundaryError {
    Request(ColorTransformRequestError),
    Planner(rusttable_color::PlannerError),
    Transform(TransformExecutionError),
    Bloom(crate::operations::OperationExecutionError),
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
            Self::Bloom(error) => write!(formatter, "Lab bloom operation failed: {error}"),
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

impl LabBoundaryError {
    pub(crate) const fn is_cancelled(&self) -> bool {
        matches!(
            self,
            Self::Transform(TransformExecutionError::Cancelled)
                | Self::Bloom(OperationExecutionError::Cancelled)
                | Self::Shadhi(OperationExecutionError::Cancelled)
        )
    }
}

pub(crate) fn apply_bloom_with_cancellation<C: Fn() -> bool>(
    config: BloomConfig,
    pixels: &[LinearRgb],
    dimensions: RasterDimensions,
    source_encoding: ColorEncoding,
    mask: Option<&[f32]>,
    opacity: f32,
    cancelled: C,
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
                    &cancelled,
                )
                .map_err(LabBoundaryError::Transform)?;
            if lab.iter().any(|channel| !channel.is_finite()) {
                return Err(LabBoundaryError::NonFinite {
                    pixel,
                    channel: RgbChannel::Red,
                });
            }
            Ok(BloomPixel::new(lab[0], lab[1], lab[2], 1.0))
        })
        .collect::<Result<Vec<_>, LabBoundaryError>>()?;
    let plan = BloomPlan::new(config, dimensions).map_err(LabBoundaryError::Bloom)?;
    let lab_output = plan
        .execute_lab(&lab_pixels, mask, opacity, &cancelled)
        .map_err(LabBoundaryError::Bloom)?;
    lab_output
        .into_iter()
        .enumerate()
        .map(|(pixel, value)| {
            let channels = value.channels();
            let rgb = from_lab
                .apply_rgb([channels[0], channels[1], channels[2]], &cancelled)
                .map_err(LabBoundaryError::Transform)?;
            let red = finite(rgb[0], pixel, RgbChannel::Red)?;
            let green = finite(rgb[1], pixel, RgbChannel::Green)?;
            let blue = finite(rgb[2], pixel, RgbChannel::Blue)?;
            Ok(LinearRgb::new(red, green, blue))
        })
        .collect()
}

/// Processing-side failure from the external bilateral Shadhi boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadhiBilateralBoundaryError {
    Color(String),
    Operation(OperationExecutionError),
}

impl fmt::Display for ShadhiBilateralBoundaryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Color(reason) => write!(formatter, "Shadhi Lab boundary failed: {reason}"),
            Self::Operation(error) => write!(formatter, "Shadhi operation failed: {error}"),
        }
    }
}

impl std::error::Error for ShadhiBilateralBoundaryError {}

/// Failure from evaluating bilateral Shadhi with an injected base-layer
/// backend. Backend failures remain typed so the pixelpipe can fall back
/// without flattening diagnostics.
#[derive(Debug)]
pub enum ShadhiBilateralEvaluationError<E> {
    Backend(E),
    Boundary(ShadhiBilateralBoundaryError),
}

impl<E: fmt::Display> fmt::Display for ShadhiBilateralEvaluationError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Backend(error) => write!(formatter, "bilateral backend failed: {error}"),
            Self::Boundary(error) => error.fmt(formatter),
        }
    }
}

impl<E: std::error::Error + 'static> std::error::Error for ShadhiBilateralEvaluationError<E> {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(error) => Some(error),
            Self::Boundary(error) => Some(error),
        }
    }
}

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

pub(crate) fn apply_shadhi_with_cancellation<C: Fn() -> bool>(
    config: ShadhiConfig,
    pixels: &[LinearRgb],
    dimensions: RasterDimensions,
    source_encoding: ColorEncoding,
    mask: Option<&[f32]>,
    opacity: f32,
    cancelled: C,
) -> Result<Vec<LinearRgb>, LabBoundaryError> {
    let boundary = prepare_shadhi_boundary_with_cancellation(pixels, source_encoding, &cancelled)?;
    let plan = ShadhiPlan::new(config, dimensions).map_err(LabBoundaryError::Shadhi)?;
    let lab_output = plan
        .execute_lab(&boundary.lab_pixels, mask, opacity, &cancelled)
        .map_err(LabBoundaryError::Shadhi)?;
    finish_shadhi_boundary_with_cancellation(&boundary.from_lab, lab_output, cancelled)
}

/// Evaluates bilateral Shadhi while delegating only Darktable's common
/// bilateral-grid base layer to a caller-provided backend.
///
/// RGB↔D50 Lab conversion, filtered-base validation, Shadhi mixing, opacity,
/// and the working-frame boundary remain canonical processing code.
///
/// # Errors
///
/// Returns a typed backend failure unchanged, or a boundary failure for a
/// non-bilateral configuration, color conversion, malformed backend output,
/// or Shadhi execution.
pub fn evaluate_bilateral_shadhi_with<E, F>(
    input: &WorkingRgbImage,
    config: ShadhiConfig,
    opacity: f32,
    backend: F,
) -> Result<WorkingRgbImage, ShadhiBilateralEvaluationError<E>>
where
    F: FnOnce(ShadhiBilateralRequest<'_>) -> Result<Vec<[f32; 4]>, E>,
{
    evaluate_bilateral_shadhi_with_cancellation(input, config, opacity, backend, || false)
}

/// Cancellable form of [`evaluate_bilateral_shadhi_with`].
///
/// The same signal is polled through both color transforms and the canonical
/// Lab mix. A backend that can cancel its own work should capture that signal
/// too; the boundary always checks it immediately before and after delegation.
///
/// # Errors
///
/// Returns [`OperationExecutionError::Cancelled`] through the typed boundary
/// when cancellation is observed, without publishing a partial frame.
pub fn evaluate_bilateral_shadhi_with_cancellation<E, F, C>(
    input: &WorkingRgbImage,
    config: ShadhiConfig,
    opacity: f32,
    backend: F,
    cancelled: C,
) -> Result<WorkingRgbImage, ShadhiBilateralEvaluationError<E>>
where
    F: FnOnce(ShadhiBilateralRequest<'_>) -> Result<Vec<[f32; 4]>, E>,
    C: Fn() -> bool,
{
    if config.shadhi_algo() != ShadhiAlgorithm::Bilateral {
        return Err(ShadhiBilateralEvaluationError::Boundary(
            ShadhiBilateralBoundaryError::Operation(
                OperationExecutionError::UnsupportedCapability(
                    "external bilateral base requires bilateral shadhi",
                ),
            ),
        ));
    }
    ShadhiPlan::validate_opacity(opacity).map_err(|error| {
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(error))
    })?;
    check_bilateral_cancellation(&cancelled)?;
    if opacity == 0.0 {
        return Ok(input.clone());
    }
    let plan = ShadhiPlan::new_external_bilateral(config, input.dimensions()).map_err(|error| {
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(error))
    })?;
    let PreparedShadhiBoundary {
        lab_pixels,
        from_lab,
    } = prepare_shadhi_boundary_with_cancellation(
        input.pixel_slice(),
        input.frame().encoding(),
        &cancelled,
    )
    .map_err(public_boundary_error)?;
    let guide = lab_pixels
        .iter()
        .map(|pixel| pixel.channels())
        .collect::<Vec<_>>();
    let request = plan.bilateral_request(&guide).map_err(|error| {
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(error))
    })?;
    check_bilateral_cancellation(&cancelled)?;
    let filtered = backend(request).map_err(ShadhiBilateralEvaluationError::Backend)?;
    check_bilateral_cancellation(&cancelled)?;
    drop(guide);
    let lab_output = plan
        .execute_lab_with_filtered_base(&lab_pixels, &filtered, None, opacity, &cancelled)
        .map_err(|error| {
            ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(error))
        })?;
    drop(filtered);
    drop(lab_pixels);
    let pixels = finish_shadhi_boundary_with_cancellation(&from_lab, lab_output, &cancelled)
        .map_err(public_boundary_error)?;
    check_bilateral_cancellation(&cancelled)?;
    WorkingRgbImage::new_with_frame(input.dimensions(), pixels, input.frame()).map_err(|error| {
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Color(
            error.to_string(),
        ))
    })
}

struct PreparedShadhiBoundary {
    lab_pixels: Vec<ShadhiPixel>,
    from_lab: TransformPlan,
}

fn prepare_shadhi_boundary_with_cancellation<C: Fn() -> bool>(
    pixels: &[LinearRgb],
    source_encoding: ColorEncoding,
    cancelled: C,
) -> Result<PreparedShadhiBoundary, LabBoundaryError> {
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
                    &cancelled,
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
    Ok(PreparedShadhiBoundary {
        lab_pixels,
        from_lab,
    })
}

fn finish_shadhi_boundary_with_cancellation<C: Fn() -> bool>(
    from_lab: &TransformPlan,
    lab_output: Vec<ShadhiPixel>,
    cancelled: C,
) -> Result<Vec<LinearRgb>, LabBoundaryError> {
    lab_output
        .into_iter()
        .enumerate()
        .map(|(pixel, value)| {
            let channels = value.channels();
            let rgb = from_lab
                .apply_rgb([channels[0], channels[1], channels[2]], &cancelled)
                .map_err(LabBoundaryError::Transform)?;
            let red = finite(rgb[0], pixel, RgbChannel::Red)?;
            let green = finite(rgb[1], pixel, RgbChannel::Green)?;
            let blue = finite(rgb[2], pixel, RgbChannel::Blue)?;
            Ok(LinearRgb::new(red, green, blue))
        })
        .collect()
}

fn public_boundary_error<E>(error: LabBoundaryError) -> ShadhiBilateralEvaluationError<E> {
    let boundary = match error {
        LabBoundaryError::Shadhi(error) => ShadhiBilateralBoundaryError::Operation(error),
        LabBoundaryError::Transform(TransformExecutionError::Cancelled) => {
            ShadhiBilateralBoundaryError::Operation(OperationExecutionError::Cancelled)
        }
        LabBoundaryError::NonFinite { pixel, channel } => {
            ShadhiBilateralBoundaryError::Operation(OperationExecutionError::NonFiniteResult {
                pixel,
                channel,
            })
        }
        other => ShadhiBilateralBoundaryError::Color(other.to_string()),
    };
    ShadhiBilateralEvaluationError::Boundary(boundary)
}

fn check_bilateral_cancellation<E, C: Fn() -> bool>(
    cancelled: &C,
) -> Result<(), ShadhiBilateralEvaluationError<E>> {
    if cancelled() {
        Err(ShadhiBilateralEvaluationError::Boundary(
            ShadhiBilateralBoundaryError::Operation(OperationExecutionError::Cancelled),
        ))
    } else {
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn canonical_shadhi_cancels_mid_bilateral_filter_without_output() {
        let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
        let source_encoding = ColorEncoding::LinearSrgbD65;
        let source = LinearRgb::new(
            FiniteF32::new(0.2).expect("red"),
            FiniteF32::new(0.3).expect("green"),
            FiniteF32::new(0.4).expect("blue"),
        );
        let pixels = vec![source; usize::try_from(dimensions.pixel_count()).expect("pixel count")];

        let transform_polls = Cell::new(0_usize);
        plan(source_encoding, ColorEncoding::LabD50)
            .expect("RGB to Lab plan")
            .apply_rgb(
                [
                    source.red().get(),
                    source.green().get(),
                    source.blue().get(),
                ],
                || {
                    transform_polls.set(transform_polls.get() + 1);
                    false
                },
            )
            .expect("sample transform");
        assert!(transform_polls.get() > 0);

        // Preparation polls once per transform step and pixel. The next
        // three checks enter execute_lab, start splatting, and process row 0;
        // cancelling on the fourth therefore stops at row 1 of the grid.
        let cancel_at = transform_polls
            .get()
            .checked_mul(pixels.len())
            .and_then(|polls| polls.checked_add(4))
            .expect("bounded poll count");
        let polls = Cell::new(0_usize);
        let result = apply_shadhi_with_cancellation(
            ShadhiConfig::defaults(),
            &pixels,
            dimensions,
            source_encoding,
            None,
            1.0,
            || {
                let next = polls.get() + 1;
                polls.set(next);
                next >= cancel_at
            },
        );

        assert!(matches!(
            result,
            Err(LabBoundaryError::Shadhi(OperationExecutionError::Cancelled))
        ));
        assert_eq!(polls.get(), cancel_at);
    }
}
