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
    transform_left: u32,
    transform_top: u32,
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

    /// Mirrors `modify_roi_out`: side sizes use native `f32` arithmetic and
    /// truncation, then the output is constrained to five pixels through
    /// three times the requested input ROI on each axis.
    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, EnlargeCanvasPlanError> {
        ensure_roi(input, self.source_dimensions, false)?;
        let (width, height) = native_output_size(input.width(), input.height(), self.config)?;
        checked_roi(input.x(), input.y(), Some(width), Some(height))
    }

    pub fn roi_out(&self, input: Roi) -> Result<Roi, EnlargeCanvasPlanError> {
        self.modify_roi_out(input)
    }

    /// Returns only the source intersection for a requested output tile.
    /// `None` explicitly represents a canvas-only tile.  This checked API is
    /// intentionally stricter than Darktable's raw ROI callback, which may
    /// return a one-pixel request for a tile containing only canvas.
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

    /// Ports the native scaled `modify_roi_in` callback.  Unlike
    /// [`Self::modify_roi_in`], this returns the callback's one-pixel request
    /// for a canvas-only tile and keeps the output ROI's coordinate space.
    pub fn modify_roi_in_scaled(
        &self,
        output: Roi,
        input_buffer: RasterDimensions,
        output_buffer: RasterDimensions,
        scale: f32,
    ) -> Result<Roi, EnlargeCanvasPlanError> {
        ensure_buffer_geometry(input_buffer, output_buffer, scale)?;
        ensure_roi(output, output_buffer, true)?;

        let (left, top) = callback_border_sizes(self.config, input_buffer, output_buffer, scale)?;
        let x = rounded_nonnegative(f32_from_i64(i64::from(output.x()) - i64::from(left)))?;
        let y = rounded_nonnegative(f32_from_i64(i64::from(output.y()) - i64::from(top)))?;
        let mut width = i64::from(output.width());
        let mut height = i64::from(output.height());

        width -= i64::from(rounded_nonnegative(f32_from_i64(
            i64::from(left) - i64::from(output.x()),
        ))?);
        height -= i64::from(rounded_nonnegative(f32_from_i64(
            i64::from(top) - i64::from(output.y()),
        ))?);

        let input_width_at_scale = f32_from_u32(input_buffer.width()) * scale;
        let input_height_at_scale = f32_from_u32(input_buffer.height()) * scale;
        width -= i64::from(rounded_nonnegative(
            f32_from_i64(i64::from(x) + width) - input_width_at_scale,
        )?);
        height -= i64::from(rounded_nonnegative(
            f32_from_i64(i64::from(y) + height) - input_height_at_scale,
        )?);

        // Keep the native MIN(p_in*, MAX(1, roi_in->width)) ordering.  The
        // final conversion is deliberately truncation, not rounding.
        let width = truncated_nonnegative(input_width_at_scale.min(f32_from_i64(width.max(1))))?;
        let height = truncated_nonnegative(input_height_at_scale.min(f32_from_i64(height.max(1))))?;
        checked_roi(x, y, Some(width), Some(height))
    }

    /// Computes the native border placement and applies both placement clamps
    /// used by `process` and `distort_mask`.
    pub fn source_placement(
        &self,
        input: Roi,
        output: Roi,
    ) -> Result<(u32, u32), EnlargeCanvasPlanError> {
        self.source_placement_scaled(
            input,
            output,
            self.source_dimensions,
            self.output_dimensions(),
            1.0,
        )
    }

    /// Scaled form of [`Self::source_placement`] for a pixelpipe tile.
    pub fn source_placement_scaled(
        &self,
        input: Roi,
        output: Roi,
        input_buffer: RasterDimensions,
        output_buffer: RasterDimensions,
        scale: f32,
    ) -> Result<(u32, u32), EnlargeCanvasPlanError> {
        ensure_buffer_geometry(input_buffer, output_buffer, scale)?;
        ensure_roi(input, input_buffer, false)?;
        ensure_roi(output, output_buffer, true)?;
        if input.width() > output.width() || input.height() > output.height() {
            return Err(EnlargeCanvasPlanError::InvalidBufferGeometry);
        }
        let (border_x, border_y) =
            aggregate_border_sizes(self.config, input_buffer, output_buffer, scale)?;
        let x = clamp_placement(border_x, output.x(), output.width(), input.width());
        let y = clamp_placement(border_y, output.y(), output.height(), input.height());
        Ok((x, y))
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), EnlargeCanvasGeometryError> {
        transform_points(
            points,
            self.geometry.transform_left,
            self.geometry.transform_top,
            true,
        )
    }

    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), EnlargeCanvasGeometryError> {
        transform_points(
            points,
            self.geometry.transform_left,
            self.geometry.transform_top,
            false,
        )
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
    InvalidScale,
    InvalidBufferGeometry,
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
            Self::InvalidScale => "enlargecanvas ROI scale is invalid",
            Self::InvalidBufferGeometry => "enlargecanvas buffer geometry is invalid",
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
    let (output_width, output_height) =
        native_output_size(source.width(), source.height(), config)?;
    if output_width > ENLARGECANVAS_MAX_DIMENSION || output_height > ENLARGECANVAS_MAX_DIMENSION {
        return Err(EnlargeCanvasPlanError::Geometry(
            EnlargeCanvasGeometryError::OutputTooLarge,
        ));
    }
    let output_dimensions = RasterDimensions::new(output_width, output_height)
        .map_err(|_| EnlargeCanvasPlanError::ArithmeticOverflow)?;
    // `process` and `distort_mask` use `_compute_pos`, including its centered
    // placement when neither side requests a border. `distort_transform` uses
    // the separate callback ratios, which remain zero when left/top is zero.
    // Both derive from the aggregate output/input border rather than the
    // independently truncated side sizes.
    let (left, top) = aggregate_border_sizes(config, source, output_dimensions, 1.0)?;
    let (transform_left, transform_top) =
        callback_border_sizes(config, source, output_dimensions, 1.0)?;
    let right = output_width
        .checked_sub(source.width())
        .and_then(|value| value.checked_sub(left))
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    let bottom = output_height
        .checked_sub(source.height())
        .and_then(|value| value.checked_sub(top))
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    Ok(EnlargeCanvasGeometry {
        left,
        right,
        top,
        bottom,
        transform_left,
        transform_top,
        output_dimensions,
        source_rect: CanvasRect::new(left, top, source.width(), source.height())?,
    })
}

fn native_output_size(
    input_width: u32,
    input_height: u32,
    config: EnlargeCanvasConfig,
) -> Result<(u32, u32), EnlargeCanvasPlanError> {
    let left = native_side(input_width, config.percent_left().get())?;
    let right = native_side(input_width, config.percent_right().get())?;
    let top = native_side(input_height, config.percent_top().get())?;
    let bottom = native_side(input_height, config.percent_bottom().get())?;
    Ok((
        native_output_length(input_width, left, right)?,
        native_output_length(input_height, top, bottom)?,
    ))
}

fn native_output_length(input: u32, left: u32, right: u32) -> Result<u32, EnlargeCanvasPlanError> {
    let requested = input
        .checked_add(left)
        .and_then(|value| value.checked_add(right))
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    let maximum = input
        .checked_mul(3)
        .ok_or(EnlargeCanvasPlanError::ArithmeticOverflow)?;
    // Match GLib's CLAMP evaluation order, including its defined behavior
    // when a tiny input makes the three-times upper bound smaller than five.
    Ok(if requested > maximum {
        maximum
    } else if requested < 5 {
        5
    } else {
        requested
    })
}

#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "the value is checked finite and nonnegative before narrowing"
)]
fn native_side(edge: u32, percent: f32) -> Result<u32, EnlargeCanvasPlanError> {
    // Keep the multiplication, division, and truncation in f32: this is the
    // arithmetic used by `int border_size = width * percent / 100.f`.
    let value = (f32_from_u32(edge) * percent / 100.0).trunc();
    truncated_nonnegative(value)
}

#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "the value is checked finite and nonnegative before narrowing"
)]
fn truncated_nonnegative(value: f32) -> Result<u32, EnlargeCanvasPlanError> {
    if !value.is_finite() || value < 0.0 || value > f32_from_u32(u32::MAX) {
        return Err(EnlargeCanvasPlanError::ArithmeticOverflow);
    }
    Ok(value as u32)
}

fn rounded_nonnegative(value: f32) -> Result<u32, EnlargeCanvasPlanError> {
    let value = value.round().max(0.0);
    truncated_nonnegative(value)
}

fn f32_from_u32(value: u32) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "native ROI arithmetic is explicitly f32-based"
    )]
    {
        value as f32
    }
}

fn f32_from_i64(value: i64) -> f32 {
    #[allow(
        clippy::cast_precision_loss,
        reason = "native ROI arithmetic is explicitly f32-based"
    )]
    {
        value as f32
    }
}

fn ensure_buffer_geometry(
    input: RasterDimensions,
    output: RasterDimensions,
    scale: f32,
) -> Result<(), EnlargeCanvasPlanError> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(EnlargeCanvasPlanError::InvalidScale);
    }
    if output.width() < input.width() || output.height() < input.height() {
        return Err(EnlargeCanvasPlanError::InvalidBufferGeometry);
    }
    Ok(())
}

fn callback_placement_ratios(config: EnlargeCanvasConfig) -> (f32, f32) {
    let horizontal = if config.percent_left().get() > 0.0 {
        config.percent_left().get() / (config.percent_left().get() + config.percent_right().get())
    } else {
        0.0
    };
    let vertical = if config.percent_top().get() > 0.0 {
        config.percent_top().get() / (config.percent_top().get() + config.percent_bottom().get())
    } else {
        0.0
    };
    (horizontal.clamp(0.0, 1.0), vertical.clamp(0.0, 1.0))
}

fn process_placement_ratios(config: EnlargeCanvasConfig) -> (f32, f32) {
    let left = config.percent_left().get();
    let right = config.percent_right().get();
    let top = config.percent_top().get();
    let bottom = config.percent_bottom().get();
    let horizontal = if left > 0.0 || right > 0.0 {
        left / (left + right)
    } else {
        0.5
    };
    let vertical = if top > 0.0 || bottom > 0.0 {
        top / (top + bottom)
    } else {
        0.5
    };
    (horizontal.clamp(0.0, 1.0), vertical.clamp(0.0, 1.0))
}

fn callback_border_sizes(
    config: EnlargeCanvasConfig,
    input: RasterDimensions,
    output: RasterDimensions,
    scale: f32,
) -> Result<(u32, u32), EnlargeCanvasPlanError> {
    ensure_buffer_geometry(input, output, scale)?;
    let (horizontal, vertical) = callback_placement_ratios(config);
    let delta_width = output
        .width()
        .checked_sub(input.width())
        .ok_or(EnlargeCanvasPlanError::InvalidBufferGeometry)?;
    let delta_height = output
        .height()
        .checked_sub(input.height())
        .ok_or(EnlargeCanvasPlanError::InvalidBufferGeometry)?;
    let left = truncated_nonnegative(f32_from_u32(delta_width) * scale * horizontal)?;
    let top = truncated_nonnegative(f32_from_u32(delta_height) * scale * vertical)?;
    Ok((left, top))
}

fn aggregate_border_sizes(
    config: EnlargeCanvasConfig,
    input: RasterDimensions,
    output: RasterDimensions,
    scale: f32,
) -> Result<(u32, u32), EnlargeCanvasPlanError> {
    ensure_buffer_geometry(input, output, scale)?;
    let delta_width = output
        .width()
        .checked_sub(input.width())
        .ok_or(EnlargeCanvasPlanError::InvalidBufferGeometry)?;
    let delta_height = output
        .height()
        .checked_sub(input.height())
        .ok_or(EnlargeCanvasPlanError::InvalidBufferGeometry)?;
    let total_width = truncated_ceil(f32_from_u32(delta_width) * scale)?;
    let total_height = truncated_ceil(f32_from_u32(delta_height) * scale)?;
    let (horizontal, vertical) = process_placement_ratios(config);
    let left = aggregate_border(total_width, horizontal)?;
    let top = aggregate_border(total_height, vertical)?;
    Ok((left, top))
}

#[allow(
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "the value is checked finite and nonnegative before narrowing"
)]
fn truncated_ceil(value: f32) -> Result<u32, EnlargeCanvasPlanError> {
    let value = value.ceil();
    if !value.is_finite() || value < 0.0 || value > f32_from_u32(u32::MAX) {
        return Err(EnlargeCanvasPlanError::ArithmeticOverflow);
    }
    Ok(value as u32)
}

fn aggregate_border(total: u32, ratio: f32) -> Result<u32, EnlargeCanvasPlanError> {
    if ratio <= 0.0 {
        return Ok(0);
    }
    truncated_nonnegative(f32_from_u32(total) * ratio)
}

fn clamp_placement(border: u32, output_origin: u32, output_extent: u32, input_extent: u32) -> u32 {
    let unclipped = border.saturating_sub(output_origin).min(output_extent);
    unclipped.min(output_extent.saturating_sub(input_extent))
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
