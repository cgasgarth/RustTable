#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::missing_errors_doc
)]

use std::fmt;

use rusttable_image::{ImageDimensions, Roi};
use sha2::{Digest, Sha256};

use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::analysis::{AnalysisError, AnalysisResult};
use super::codec::{ASHIFT_COMPATIBILITY_ID, ASHIFT_MAX_DIMENSION, PerspectiveConfig};
use super::geometry::{Homography, Rect, TransformError};

const MAX_OUTPUT_PIXELS: u64 = 1 << 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Interpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos3,
}

impl Interpolation {
    #[must_use]
    pub const fn support(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos3 => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BoundaryMode {
    Reflect,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerspectiveExecutionError {
    Cancelled,
    InvalidShape { expected: usize, actual: usize },
    ArithmeticOverflow,
    NonFiniteResult { pixel: usize },
    Analysis(AnalysisError),
    Transform(TransformError),
}

impl fmt::Display for PerspectiveExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("perspective execution was cancelled"),
            Self::InvalidShape { expected, actual } => write!(
                formatter,
                "perspective expected {expected} pixels, got {actual}"
            ),
            Self::ArithmeticOverflow => {
                formatter.write_str("perspective execution arithmetic overflowed")
            }
            Self::NonFiniteResult { pixel } => write!(
                formatter,
                "perspective produced a non-finite pixel at {pixel}"
            ),
            Self::Analysis(error) => write!(formatter, "perspective analysis failed: {error}"),
            Self::Transform(error) => write!(formatter, "perspective transform failed: {error}"),
        }
    }
}
impl std::error::Error for PerspectiveExecutionError {}
impl From<TransformError> for PerspectiveExecutionError {
    fn from(error: TransformError) -> Self {
        Self::Transform(error)
    }
}
impl From<AnalysisError> for PerspectiveExecutionError {
    fn from(error: AnalysisError) -> Self {
        Self::Analysis(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerspectiveReceipt {
    identity: [u8; 32],
    sampled_pixels: usize,
}

impl PerspectiveReceipt {
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub const fn sampled_pixels(&self) -> usize {
        self.sampled_pixels
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerspectivePlan {
    config: PerspectiveConfig,
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    forward: Homography,
    inverse: Homography,
    output_bounds: Rect,
    interpolation: Interpolation,
    boundary: BoundaryMode,
    identity: [u8; 32],
}

impl PerspectivePlan {
    pub fn new(
        config: PerspectiveConfig,
        source_dimensions: RasterDimensions,
        interpolation: Interpolation,
    ) -> Result<Self, PerspectiveExecutionError> {
        Self::build(config, source_dimensions, interpolation, None)
    }

    pub fn from_analysis(
        config: PerspectiveConfig,
        source_dimensions: RasterDimensions,
        analysis: &AnalysisResult,
        interpolation: Interpolation,
    ) -> Result<Self, PerspectiveExecutionError> {
        if !analysis.is_ready() {
            return Err(PerspectiveExecutionError::Analysis(
                AnalysisError::InsufficientLines {
                    vertical: analysis
                        .lines()
                        .iter()
                        .filter(|line| matches!(line.kind(), super::analysis::LineKind::Vertical))
                        .count(),
                    horizontal: analysis
                        .lines()
                        .iter()
                        .filter(|line| matches!(line.kind(), super::analysis::LineKind::Horizontal))
                        .count(),
                },
            ));
        }
        Self::build(
            config,
            source_dimensions,
            interpolation,
            analysis.correction(),
        )
    }

    fn build(
        config: PerspectiveConfig,
        source_dimensions: RasterDimensions,
        interpolation: Interpolation,
        automatic: Option<Homography>,
    ) -> Result<Self, PerspectiveExecutionError> {
        if source_dimensions.width() > ASHIFT_MAX_DIMENSION
            || source_dimensions.height() > ASHIFT_MAX_DIMENSION
        {
            return Err(PerspectiveExecutionError::ArithmeticOverflow);
        }
        let base = if let Some(quad) = config.quad() {
            Homography::from_quad(quad, source_dimensions.width(), source_dimensions.height())
                .map_err(|_| TransformError::InvalidBounds)?
        } else if let Some(automatic) = automatic {
            automatic.then(
                Homography::camera(&config, source_dimensions)
                    .map_err(|_| TransformError::InvalidBounds)?,
            )
        } else {
            Homography::camera(&config, source_dimensions)
                .map_err(|_| TransformError::InvalidBounds)?
        };
        let base_bounds = base.bounds(source_dimensions)?;
        let crop = crop_rectangle(
            base,
            base_bounds,
            source_dimensions,
            config.crop_mode(),
            config.aspect().get(),
        )?;
        let base = if crop.min_x().abs() <= f64::EPSILON && crop.min_y().abs() <= f64::EPSILON {
            base
        } else {
            let translation = Homography::new([
                1.0,
                0.0,
                -crop.min_x(),
                0.0,
                1.0,
                -crop.min_y(),
                0.0,
                0.0,
                1.0,
            ])
            .map_err(|_| TransformError::InvalidBounds)?;
            base.then(translation)
        };
        let output_width = dimension_extent(crop.width())?;
        let output_height = dimension_extent(crop.height())?;
        let output_dimensions = RasterDimensions::new(output_width, output_height)
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        let inverse = base.inverse().map_err(|_| TransformError::InvalidBounds)?;
        let identity = identity(
            &config,
            source_dimensions,
            output_dimensions,
            base,
            interpolation,
        );
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            forward: base,
            inverse,
            output_bounds: Rect::new(
                0.0,
                0.0,
                f64::from(output_width.saturating_sub(1)),
                f64::from(output_height.saturating_sub(1)),
            )
            .map_err(|_| TransformError::InvalidBounds)?,
            interpolation,
            boundary: BoundaryMode::Reflect,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &PerspectiveConfig {
        &self.config
    }
    #[must_use]
    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.output_dimensions
    }
    #[must_use]
    pub const fn output_bounds(&self) -> Rect {
        self.output_bounds
    }
    #[must_use]
    pub const fn interpolation(&self) -> Interpolation {
        self.interpolation
    }
    #[must_use]
    pub const fn boundary_mode(&self) -> BoundaryMode {
        self.boundary
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.forward.coefficients() == Homography::identity().coefficients()
    }

    pub fn forward_point(
        &self,
        point: super::geometry::Point,
    ) -> Result<super::geometry::Point, PerspectiveExecutionError> {
        Ok(self.forward.apply(point)?)
    }
    pub fn back_point(
        &self,
        point: super::geometry::Point,
    ) -> Result<super::geometry::Point, PerspectiveExecutionError> {
        Ok(self.inverse.apply(point)?)
    }

    pub fn output_roi(&self, input: Roi) -> Result<Roi, PerspectiveExecutionError> {
        let source = image_dimensions(self.source_dimensions)?;
        input
            .within(source)
            .map_err(|_| TransformError::OutsideSource)?;
        let points = roi_points(input);
        let mapped = points.map(|point| self.forward.apply(point));
        roi_from_points(mapped, self.output_dimensions)
    }

    pub fn input_roi(&self, output: Roi) -> Result<Roi, PerspectiveExecutionError> {
        let target = image_dimensions(self.output_dimensions)?;
        output
            .within(target)
            .map_err(|_| TransformError::OutsideSource)?;
        let points = roi_points(output);
        let mapped = points.map(|point| self.inverse.apply(point));
        let mut roi = roi_from_points(mapped, self.source_dimensions)?;
        let support = self.interpolation.support();
        let x = roi.x().saturating_sub(support);
        let y = roi.y().saturating_sub(support);
        let right = roi
            .right()
            .saturating_add(support)
            .min(self.source_dimensions.width());
        let bottom = roi
            .bottom()
            .saturating_add(support)
            .min(self.source_dimensions.height());
        roi = Roi::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
            .map_err(|_| TransformError::Overflow)?;
        Ok(roi)
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<PerspectiveExecution, PerspectiveExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<PerspectiveExecution, PerspectiveExecutionError> {
        let expected = usize::try_from(self.source_dimensions.pixel_count())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(PerspectiveExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        let output_count = usize::try_from(self.output_dimensions.pixel_count())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        if u64::try_from(output_count).map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?
            > MAX_OUTPUT_PIXELS
        {
            return Err(PerspectiveExecutionError::ArithmeticOverflow);
        }
        let mut pixels = Vec::with_capacity(output_count);
        let width = usize::try_from(self.output_dimensions.width())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        for index in 0..output_count {
            if cancelled() {
                return Err(PerspectiveExecutionError::Cancelled);
            }
            let x = index % width;
            let y = index / width;
            let source = self
                .inverse
                .apply(super::geometry::Point::new(x as f64, y as f64))?;
            let pixel = sample_rgb(
                input,
                self.source_dimensions,
                source.x(),
                source.y(),
                self.interpolation,
                self.boundary,
            )
            .ok_or(PerspectiveExecutionError::NonFiniteResult { pixel: index })?;
            pixels.push(pixel);
        }
        Ok(PerspectiveExecution {
            pixels,
            dimensions: self.output_dimensions,
            receipt: PerspectiveReceipt {
                identity: self.identity,
                sampled_pixels: output_count,
            },
        })
    }

    pub fn execute_plane<F: Fn() -> bool>(
        &self,
        input: &[f32],
        cancelled: F,
    ) -> Result<Vec<f32>, PerspectiveExecutionError> {
        let expected = usize::try_from(self.source_dimensions.pixel_count())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        if input.len() != expected {
            return Err(PerspectiveExecutionError::InvalidShape {
                expected,
                actual: input.len(),
            });
        }
        let output_count = usize::try_from(self.output_dimensions.pixel_count())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        let width = usize::try_from(self.output_dimensions.width())
            .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)?;
        let mut output = Vec::with_capacity(output_count);
        for index in 0..output_count {
            if cancelled() {
                return Err(PerspectiveExecutionError::Cancelled);
            }
            let source = self.inverse.apply(super::geometry::Point::new(
                (index % width) as f64,
                (index / width) as f64,
            ))?;
            output.push(
                sample_plane(
                    input,
                    self.source_dimensions,
                    source.x(),
                    source.y(),
                    self.interpolation,
                    self.boundary,
                )
                .ok_or(PerspectiveExecutionError::NonFiniteResult { pixel: index })?,
            );
        }
        Ok(output)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PerspectiveExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    receipt: PerspectiveReceipt,
}
impl PerspectiveExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn receipt(&self) -> &PerspectiveReceipt {
        &self.receipt
    }
}

fn dimension_extent(extent: f64) -> Result<u32, PerspectiveExecutionError> {
    if !extent.is_finite() || extent < 0.0 {
        return Err(PerspectiveExecutionError::ArithmeticOverflow);
    }
    let value = (extent.ceil() + 1.0).max(1.0) as u64;
    u32::try_from(value).map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)
}

fn crop_rectangle(
    transform: Homography,
    bounds: Rect,
    source: RasterDimensions,
    mode: super::codec::CropMode,
    aspect_adjust: f32,
) -> Result<Rect, PerspectiveExecutionError> {
    if transform.coefficients() == Homography::identity().coefficients() {
        return Ok(bounds);
    }
    if matches!(mode, super::codec::CropMode::Off) {
        return Ok(bounds);
    }
    let inverse = transform
        .inverse()
        .map_err(|_| TransformError::InvalidBounds)?;
    let source_ratio = f64::from(source.width()) / f64::from(source.height());
    let target_ratio = if matches!(mode, super::codec::CropMode::Aspect) {
        source_ratio * f64::from(aspect_adjust)
    } else {
        bounds.width() / bounds.height().max(f64::EPSILON)
    };
    let (full_width, full_height) = if matches!(mode, super::codec::CropMode::Aspect) {
        let width = bounds.width();
        let height = width / target_ratio;
        if height <= bounds.height() {
            (width, height)
        } else {
            (bounds.height() * target_ratio, bounds.height())
        }
    } else {
        (bounds.width(), bounds.height())
    };
    let center = bounds.center();
    let mut lower = 0.0;
    let mut upper = 1.0;
    let mut best = Rect::new(center.x(), center.y(), center.x(), center.y())
        .map_err(|_| TransformError::InvalidBounds)?;
    for _ in 0..48 {
        let factor = (lower + upper) * 0.5;
        let width = full_width * factor;
        let height = full_height * factor;
        let candidate = Rect::new(
            center.x() - width * 0.5,
            center.y() - height * 0.5,
            center.x() + width * 0.5,
            center.y() + height * 0.5,
        )
        .map_err(|_| TransformError::InvalidBounds)?;
        if crop_is_valid(inverse, candidate, source) {
            best = candidate;
            lower = factor;
        } else {
            upper = factor;
        }
    }
    if best.width() <= f64::EPSILON || best.height() <= f64::EPSILON {
        return Err(TransformError::InvalidBounds.into());
    }
    Ok(best)
}

fn crop_is_valid(transform: Homography, crop: Rect, source: RasterDimensions) -> bool {
    let last_x = f64::from(source.width().saturating_sub(1));
    let last_y = f64::from(source.height().saturating_sub(1));
    [
        super::geometry::Point::new(crop.min_x(), crop.min_y()),
        super::geometry::Point::new(crop.max_x(), crop.min_y()),
        super::geometry::Point::new(crop.max_x(), crop.max_y()),
        super::geometry::Point::new(crop.min_x(), crop.max_y()),
    ]
    .into_iter()
    .all(|point| {
        transform.apply(point).is_ok_and(|mapped| {
            mapped.x() >= -1.0e-7
                && mapped.y() >= -1.0e-7
                && mapped.x() <= last_x + 1.0e-7
                && mapped.y() <= last_y + 1.0e-7
        })
    })
}

fn image_dimensions(
    dimensions: RasterDimensions,
) -> Result<ImageDimensions, PerspectiveExecutionError> {
    ImageDimensions::new(dimensions.width(), dimensions.height())
        .map_err(|_| PerspectiveExecutionError::ArithmeticOverflow)
}

fn roi_points(roi: Roi) -> [super::geometry::Point; 4] {
    [
        super::geometry::Point::new(f64::from(roi.x()), f64::from(roi.y())),
        super::geometry::Point::new(f64::from(roi.right().saturating_sub(1)), f64::from(roi.y())),
        super::geometry::Point::new(
            f64::from(roi.right().saturating_sub(1)),
            f64::from(roi.bottom().saturating_sub(1)),
        ),
        super::geometry::Point::new(
            f64::from(roi.x()),
            f64::from(roi.bottom().saturating_sub(1)),
        ),
    ]
}

fn roi_from_points(
    points: [Result<super::geometry::Point, TransformError>; 4],
    dimensions: RasterDimensions,
) -> Result<Roi, PerspectiveExecutionError> {
    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in points {
        let point = point?;
        min_x = min_x.min(point.x());
        min_y = min_y.min(point.y());
        max_x = max_x.max(point.x());
        max_y = max_y.max(point.y());
    }
    let x = min_x.floor().max(0.0) as u32;
    let y = min_y.floor().max(0.0) as u32;
    let right = (max_x.ceil() + 1.0).min(f64::from(dimensions.width())) as u32;
    let bottom = (max_y.ceil() + 1.0).min(f64::from(dimensions.height())) as u32;
    Roi::new(
        x.min(dimensions.width()),
        y.min(dimensions.height()),
        right.saturating_sub(x),
        bottom.saturating_sub(y),
    )
    .map_err(|_| TransformError::Overflow.into())
}

fn reflect(value: f64, size: u32) -> Option<f64> {
    if !value.is_finite() || size == 0 {
        return None;
    }
    if size == 1 {
        return Some(0.0);
    }
    let period = f64::from(size - 1) * 2.0;
    let mut value = value.rem_euclid(period);
    if value > f64::from(size - 1) {
        value = period - value;
    }
    Some(value)
}

fn sample_plane(
    input: &[f32],
    dimensions: RasterDimensions,
    x: f64,
    y: f64,
    interpolation: Interpolation,
    _: BoundaryMode,
) -> Option<f32> {
    sample_channel(dimensions, x, y, interpolation, |x, y| {
        pixel_plane(input, dimensions, x, y)
    })
}

fn sample_rgb(
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    x: f64,
    y: f64,
    interpolation: Interpolation,
    _boundary: BoundaryMode,
) -> Option<LinearRgb> {
    let red = sample_channel(dimensions, x, y, interpolation, |x, y| {
        pixel_rgb(input, dimensions, x, y).map(|pixel| pixel.red().get())
    })?;
    let green = sample_channel(dimensions, x, y, interpolation, |x, y| {
        pixel_rgb(input, dimensions, x, y).map(|pixel| pixel.green().get())
    })?;
    let blue = sample_channel(dimensions, x, y, interpolation, |x, y| {
        pixel_rgb(input, dimensions, x, y).map(|pixel| pixel.blue().get())
    })?;
    Some(LinearRgb::new(
        FiniteF32::new(red).ok()?,
        FiniteF32::new(green).ok()?,
        FiniteF32::new(blue).ok()?,
    ))
}

fn pixel_plane(input: &[f32], dimensions: RasterDimensions, x: u32, y: u32) -> Option<f32> {
    let width = usize::try_from(dimensions.width()).ok()?;
    input
        .get(
            usize::try_from(y)
                .ok()?
                .checked_mul(width)?
                .checked_add(usize::try_from(x).ok()?)?,
        )
        .copied()
}

fn pixel_rgb(
    input: &[LinearRgb],
    dimensions: RasterDimensions,
    x: u32,
    y: u32,
) -> Option<LinearRgb> {
    let width = usize::try_from(dimensions.width()).ok()?;
    input
        .get(
            usize::try_from(y)
                .ok()?
                .checked_mul(width)?
                .checked_add(usize::try_from(x).ok()?)?,
        )
        .copied()
}

fn sample_channel<F: Fn(u32, u32) -> Option<f32>>(
    dimensions: RasterDimensions,
    x: f64,
    y: f64,
    interpolation: Interpolation,
    value: F,
) -> Option<f32> {
    let x = reflect(x, dimensions.width())?;
    let y = reflect(y, dimensions.height())?;
    if matches!(interpolation, Interpolation::Nearest) {
        return value(x.round() as u32, y.round() as u32);
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(dimensions.width() - 1);
    let y1 = (y0 + 1).min(dimensions.height() - 1);
    let fx = x - f64::from(x0);
    let fy = y - f64::from(y0);
    let a = f64::from(value(x0, y0)?);
    let b = f64::from(value(x1, y0)?);
    let c = f64::from(value(x0, y1)?);
    let d = f64::from(value(x1, y1)?);
    let result = (a * (1.0 - fx) + b * fx) * (1.0 - fy) + (c * (1.0 - fx) + d * fx) * fy;
    result.is_finite().then_some(result as f32)
}

fn identity(
    config: &PerspectiveConfig,
    source: RasterDimensions,
    output: RasterDimensions,
    matrix: Homography,
    interpolation: Interpolation,
) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(ASHIFT_COMPATIBILITY_ID.as_bytes());
    digest.update(1_u16.to_le_bytes());
    digest.update(source.width().to_le_bytes());
    digest.update(source.height().to_le_bytes());
    digest.update(output.width().to_le_bytes());
    digest.update(output.height().to_le_bytes());
    for coefficient in matrix.coefficients() {
        digest.update(coefficient.to_bits().to_le_bytes());
    }
    digest.update([match interpolation {
        Interpolation::Nearest => 0,
        Interpolation::Bilinear => 1,
        Interpolation::Bicubic => 2,
        Interpolation::Lanczos3 => 3,
    }]);
    digest.update([config.method() as u8, config.crop_mode() as u8]);
    digest.finalize().into()
}
