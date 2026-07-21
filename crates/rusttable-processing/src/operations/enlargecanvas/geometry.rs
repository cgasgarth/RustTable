use crate::RasterDimensions;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};
use std::fmt;

use super::{
    CanvasFill, ENLARGECANVAS_COMPATIBILITY_ID, ENLARGECANVAS_MAX_DIMENSION, EnlargeCanvasConfig,
};

/// A checked half-open rectangle used for the source and output placements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CanvasRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl CanvasRect {
    pub const fn new(
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, EnlargeCanvasGeometryError> {
        if x.checked_add(width).is_none() || y.checked_add(height).is_none() {
            return Err(EnlargeCanvasGeometryError::ArithmeticOverflow);
        }
        Ok(Self {
            x,
            y,
            width,
            height,
        })
    }

    #[must_use]
    pub const fn x(self) -> u32 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> u32 {
        self.y
    }
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
    #[must_use]
    pub const fn right(self) -> u32 {
        self.x + self.width
    }
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.y + self.height
    }

    #[must_use]
    pub fn as_roi(self) -> Roi {
        Roi::new(self.x, self.y, self.width, self.height).expect("checked canvas rectangle")
    }
}

/// Resolved integer geometry for one canvas plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnlargeCanvasGeometry {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
    output_dimensions: RasterDimensions,
    source_rect: CanvasRect,
}

impl EnlargeCanvasGeometry {
    #[must_use]
    pub const fn left(self) -> u32 {
        self.left
    }
    #[must_use]
    pub const fn right(self) -> u32 {
        self.right
    }
    #[must_use]
    pub const fn top(self) -> u32 {
        self.top
    }
    #[must_use]
    pub const fn bottom(self) -> u32 {
        self.bottom
    }
    #[must_use]
    pub const fn output_dimensions(self) -> RasterDimensions {
        self.output_dimensions
    }
    #[must_use]
    pub const fn source_rect(self) -> CanvasRect {
        self.source_rect
    }
    #[must_use]
    pub fn output_roi(self) -> Roi {
        Roi::full(to_image_dimensions(self.output_dimensions))
    }
}

/// Immutable checked plan shared by scalar RGB, mask, and image-contract paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnlargeCanvasPlan {
    config: EnlargeCanvasConfig,
    fill: CanvasFill,
    source_dimensions: RasterDimensions,
    geometry: EnlargeCanvasGeometry,
    identity: [u8; 32],
}

impl EnlargeCanvasPlan {
    pub fn new(
        config: EnlargeCanvasConfig,
        source_dimensions: RasterDimensions,
    ) -> Result<Self, EnlargeCanvasPlanError> {
        Self::new_with_fill(config, source_dimensions, config.fill())
    }

    pub fn new_with_fill(
        config: EnlargeCanvasConfig,
        source_dimensions: RasterDimensions,
        fill: CanvasFill,
    ) -> Result<Self, EnlargeCanvasPlanError> {
        let geometry = resolve_geometry(config, source_dimensions)?;
        let identity = plan_identity(config, fill, source_dimensions, geometry);
        Ok(Self {
            config,
            fill,
            source_dimensions,
            geometry,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> EnlargeCanvasConfig {
        self.config
    }
    #[must_use]
    pub const fn fill(&self) -> CanvasFill {
        self.fill
    }
    #[must_use]
    pub const fn source_dimensions(&self) -> RasterDimensions {
        self.source_dimensions
    }
    #[must_use]
    pub const fn output_dimensions(&self) -> RasterDimensions {
        self.geometry.output_dimensions()
    }
    #[must_use]
    pub const fn geometry(&self) -> EnlargeCanvasGeometry {
        self.geometry
    }
    #[must_use]
    pub const fn source_offset(&self) -> (u32, u32) {
        (self.geometry.left(), self.geometry.top())
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.geometry.left() == 0
            && self.geometry.right() == 0
            && self.geometry.top() == 0
            && self.geometry.bottom() == 0
    }

    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, EnlargeCanvasPlanError> {
        ensure_roi(input, self.source_dimensions, false)?;
        checked_roi(
            input.x(),
            input.y(),
            input
                .width()
                .checked_add(self.geometry.left())
                .and_then(|v| v.checked_add(self.geometry.right())),
            input
                .height()
                .checked_add(self.geometry.top())
                .and_then(|v| v.checked_add(self.geometry.bottom())),
        )
    }

    pub fn roi_out(&self, input: Roi) -> Result<Roi, EnlargeCanvasPlanError> {
        self.modify_roi_out(input)
    }

    /// Returns only the source intersection for a requested output tile.
    /// `None` explicitly represents a canvas-only tile.
    pub fn modify_roi_in(&self, output: Roi) -> Result<Option<Roi>, EnlargeCanvasPlanError> {
        ensure_roi(output, self.output_dimensions(), true)?;
        let source = self.geometry.source_rect();
        let x0 = i64::from(output.x()) - i64::from(source.x());
        let y0 = i64::from(output.y()) - i64::from(source.y());
        let x1 = x0 + i64::from(output.width());
        let y1 = y0 + i64::from(output.height());
        let left = x0.max(0).min(i64::from(self.source_dimensions.width()));
        let top = y0.max(0).min(i64::from(self.source_dimensions.height()));
        let right = x1.max(0).min(i64::from(self.source_dimensions.width()));
        let bottom = y1.max(0).min(i64::from(self.source_dimensions.height()));
        if left >= right || top >= bottom {
            return Ok(None);
        }
        Ok(Some(checked_roi(
            u32::try_from(left).map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?,
            u32::try_from(top).map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?,
            Some(
                u32::try_from(right - left)
                    .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?,
            ),
            Some(
                u32::try_from(bottom - top)
                    .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?,
            ),
        )?))
    }

    pub fn roi_in(&self, output: Roi) -> Result<Option<Roi>, EnlargeCanvasPlanError> {
        self.modify_roi_in(output)
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), EnlargeCanvasGeometryError> {
        transform_points(points, self.geometry.left(), self.geometry.top(), true)
    }

    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), EnlargeCanvasGeometryError> {
        transform_points(points, self.geometry.left(), self.geometry.top(), false)
    }

    pub fn memory_estimate_bytes(&self) -> Result<usize, EnlargeCanvasPlanError> {
        let pixels = usize::try_from(self.output_dimensions().pixel_count())
            .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?;
        pixels
            .checked_mul(16)
            .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnlargeCanvasGeometryError {
    InvalidPointBuffer,
    NonFinitePoint,
    NonFiniteResult,
    ArithmeticOverflow,
    OutputTooLarge,
}

impl fmt::Display for EnlargeCanvasGeometryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPointBuffer => "enlargecanvas point buffer must contain x/y pairs",
            Self::NonFinitePoint => "enlargecanvas point is non-finite",
            Self::NonFiniteResult => "enlargecanvas transform is non-finite",
            Self::ArithmeticOverflow => "enlargecanvas geometry arithmetic overflowed",
            Self::OutputTooLarge => "enlargecanvas output dimensions are excessive",
        })
    }
}

impl std::error::Error for EnlargeCanvasGeometryError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnlargeCanvasPlanError {
    Geometry(EnlargeCanvasGeometryError),
    RoiOutsideSource,
    RoiOutsideOutput,
    ArithmeticOverflow,
}

impl fmt::Display for EnlargeCanvasPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Geometry(error) => {
                return write!(formatter, "enlargecanvas geometry error: {error}");
            }
            Self::RoiOutsideSource => "ROI is outside enlargecanvas source dimensions",
            Self::RoiOutsideOutput => "ROI is outside enlargecanvas output dimensions",
            Self::ArithmeticOverflow => "enlargecanvas plan arithmetic overflowed",
        })
    }
}

impl std::error::Error for EnlargeCanvasPlanError {}

impl From<EnlargeCanvasGeometryError> for EnlargeCanvasPlanError {
    fn from(error: EnlargeCanvasGeometryError) -> Self {
        Self::Geometry(error)
    }
}

fn resolve_geometry(
    config: EnlargeCanvasConfig,
    source: RasterDimensions,
) -> Result<EnlargeCanvasGeometry, EnlargeCanvasPlanError> {
    let left = scaled_side(source.width(), config.percent_left().get())?;
    let right = scaled_side(source.width(), config.percent_right().get())?;
    let top = scaled_side(source.height(), config.percent_top().get())?;
    let bottom = scaled_side(source.height(), config.percent_bottom().get())?;
    let output_width = source
        .width()
        .checked_add(left)
        .and_then(|v| v.checked_add(right))
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    let output_height = source
        .height()
        .checked_add(top)
        .and_then(|v| v.checked_add(bottom))
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    if output_width > ENLARGECANVAS_MAX_DIMENSION || output_height > ENLARGECANVAS_MAX_DIMENSION {
        return Err(EnlargeCanvasPlanError::Geometry(
            EnlargeCanvasGeometryError::OutputTooLarge,
        ));
    }
    let output_dimensions = RasterDimensions::new(output_width, output_height)
        .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?;
    Ok(EnlargeCanvasGeometry {
        left,
        right,
        top,
        bottom,
        output_dimensions,
        source_rect: CanvasRect::new(left, top, source.width(), source.height())?,
    })
}

#[allow(
    clippy::cast_sign_loss,
    reason = "the value is checked finite and nonnegative before narrowing"
)]
fn scaled_side(edge: u32, percent: f32) -> Result<u32, EnlargeCanvasPlanError> {
    let value = (f64::from(edge) * f64::from(percent) / 100.0).floor();
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(EnlargeCanvasPlanError::ArithmeticOverflow);
    }
    Ok(value as u32)
}

fn ensure_roi(
    roi: Roi,
    dimensions: RasterDimensions,
    output: bool,
) -> Result<(), EnlargeCanvasPlanError> {
    let dimensions = rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
        .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?;
    roi.within(dimensions).map_err(|_| {
        if output {
            EnlargeCanvasPlanError::RoiOutsideOutput
        } else {
            EnlargeCanvasPlanError::RoiOutsideSource
        }
    })?;
    Ok(())
}

fn checked_roi(
    x: u32,
    y: u32,
    width: Option<u32>,
    height: Option<u32>,
) -> Result<Roi, EnlargeCanvasPlanError> {
    Roi::new(
        x,
        y,
        width.ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?,
        height.ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?,
    )
    .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)
}

fn transform_points(
    points: &mut [f32],
    x: u32,
    y: u32,
    add: bool,
) -> Result<(), EnlargeCanvasGeometryError> {
    if !points.len().is_multiple_of(2) {
        return Err(EnlargeCanvasGeometryError::InvalidPointBuffer);
    }
    let (pairs, remainder) = points.as_chunks_mut::<2>();
    if !remainder.is_empty() {
        return Err(EnlargeCanvasGeometryError::InvalidPointBuffer);
    }
    for pair in pairs {
        if !pair[0].is_finite() || !pair[1].is_finite() {
            return Err(EnlargeCanvasGeometryError::NonFinitePoint);
        }
        let offset_x = x as f32;
        let offset_y = y as f32;
        pair[0] = if add {
            pair[0] + offset_x
        } else {
            pair[0] - offset_x
        };
        pair[1] = if add {
            pair[1] + offset_y
        } else {
            pair[1] - offset_y
        };
        if !pair[0].is_finite() || !pair[1].is_finite() {
            return Err(EnlargeCanvasGeometryError::NonFiniteResult);
        }
    }
    Ok(())
}

fn to_image_dimensions(dimensions: RasterDimensions) -> rusttable_image::ImageDimensions {
    rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
        .expect("validated raster dimensions")
}

fn plan_identity(
    config: EnlargeCanvasConfig,
    fill: CanvasFill,
    source: RasterDimensions,
    geometry: EnlargeCanvasGeometry,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(ENLARGECANVAS_COMPATIBILITY_ID.as_bytes());
    for value in [
        config.percent_left().get(),
        config.percent_right().get(),
        config.percent_top().get(),
        config.percent_bottom().get(),
        fill.red().get(),
        fill.green().get(),
        fill.blue().get(),
        fill.alpha().get(),
    ] {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.update((config.color() as u32).to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    for value in [
        geometry.left(),
        geometry.right(),
        geometry.top(),
        geometry.bottom(),
    ] {
        hasher.update(value.to_le_bytes());
    }
    hasher.finalize().into()
}
