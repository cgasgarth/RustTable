use super::{
    MAX_OUTPUT_DIMENSION, SCALEPIXELS_COMPATIBILITY_ID, SCALEPIXELS_SCHEMA_VERSION,
    ScalePixelsConfig, ScalePixelsGpuDispatch, ScalePixelsPlan, ScalePixelsPlanError,
    ScalePixelsPreferences,
};
use crate::RasterDimensions;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalePixelsGeometry {
    output_dimensions: RasterDimensions,
    output_origin: (u32, u32),
}

impl ScalePixelsGeometry {
    #[must_use]
    pub const fn output_dimensions(self) -> RasterDimensions {
        self.output_dimensions
    }

    #[must_use]
    pub const fn output_origin(self) -> (u32, u32) {
        self.output_origin
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalePixelsGeometryError {
    InvalidPointBuffer,
    NonFinitePoint,
    NonFiniteResult,
}

impl std::fmt::Display for ScalePixelsGeometryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidPointBuffer => "point buffer must contain x/y pairs",
            Self::NonFinitePoint => "point contains a non-finite coordinate",
            Self::NonFiniteResult => "point transform produced a non-finite coordinate",
        })
    }
}

impl std::error::Error for ScalePixelsGeometryError {}

impl ScalePixelsPlan {
    pub fn new(
        config: ScalePixelsConfig,
        source_dimensions: RasterDimensions,
    ) -> Result<Self, ScalePixelsPlanError> {
        Self::new_with_preferences(config, source_dimensions, ScalePixelsPreferences::default())
    }

    pub fn new_with_preferences(
        config: ScalePixelsConfig,
        source_dimensions: RasterDimensions,
        preferences: ScalePixelsPreferences,
    ) -> Result<Self, ScalePixelsPlanError> {
        let geometry = resolve_geometry(config.pixel_aspect_ratio(), source_dimensions)?;
        let output_dimensions = geometry.output_dimensions();
        let x_scale = f64::from(source_dimensions.width()) / f64::from(output_dimensions.width());
        let y_scale = f64::from(source_dimensions.height()) / f64::from(output_dimensions.height());
        let x_scale = finite_scale(x_scale)?;
        let y_scale = finite_scale(y_scale)?;
        let identity = plan_identity(&config, source_dimensions, output_dimensions, preferences);
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            x_scale,
            y_scale,
            preferences,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &ScalePixelsConfig {
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
    pub const fn x_scale(&self) -> f32 {
        self.x_scale
    }

    #[must_use]
    pub const fn y_scale(&self) -> f32 {
        self.y_scale
    }

    #[must_use]
    pub const fn preferences(&self) -> ScalePixelsPreferences {
        self.preferences
    }

    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.config.is_identity()
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn geometry_for_roi(
        &self,
        input: Roi,
    ) -> Result<ScalePixelsGeometry, ScalePixelsPlanError> {
        ensure_roi(input, self.source_dimensions, true)?;
        resolve_roi_geometry(input, self.config.pixel_aspect_ratio())
    }

    pub fn roi_in(&self, output: Roi) -> Result<Roi, ScalePixelsPlanError> {
        ensure_roi(output, self.output_dimensions, false)?;
        scale_roi(output, self.x_scale, self.y_scale, self.source_dimensions)
    }

    pub fn roi_out(&self, input: Roi) -> Result<Roi, ScalePixelsPlanError> {
        ensure_roi(input, self.source_dimensions, true)?;
        scale_roi(
            input,
            1.0 / self.x_scale,
            1.0 / self.y_scale,
            self.output_dimensions,
        )
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), ScalePixelsGeometryError> {
        transform_points(points, 1.0 / self.x_scale, 1.0 / self.y_scale)
    }

    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), ScalePixelsGeometryError> {
        transform_points(points, self.x_scale, self.y_scale)
    }

    pub fn gpu_dispatch(
        &self,
        input_roi: Roi,
        output_roi: Roi,
        input_row_stride: usize,
        output_row_stride: usize,
    ) -> Result<ScalePixelsGpuDispatch, ScalePixelsPlanError> {
        ensure_roi(input_roi, self.source_dimensions, true)?;
        ensure_roi(output_roi, self.output_dimensions, false)?;
        let input_width = usize::try_from(input_roi.width())
            .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
        let output_width = usize::try_from(output_roi.width())
            .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
        if input_row_stride < input_width.saturating_mul(16)
            || output_row_stride < output_width.saturating_mul(16)
        {
            return Err(ScalePixelsPlanError::ArithmeticOverflow);
        }
        let input_bytes = input_row_stride
            .checked_mul(
                usize::try_from(input_roi.height())
                    .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?,
            )
            .ok_or(ScalePixelsPlanError::ArithmeticOverflow)?;
        let output_bytes = output_row_stride
            .checked_mul(
                usize::try_from(output_roi.height())
                    .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?,
            )
            .ok_or(ScalePixelsPlanError::ArithmeticOverflow)?;
        Ok(ScalePixelsGpuDispatch {
            input_roi,
            output_roi,
            input_row_stride,
            output_row_stride,
            image_support: self.preferences.image().support(),
            mask_support: self.preferences.warp().support(),
            workgroups: (
                output_roi.width().div_ceil(8),
                output_roi.height().div_ceil(8),
            ),
            memory_bytes: input_bytes
                .checked_add(output_bytes)
                .ok_or(ScalePixelsPlanError::ArithmeticOverflow)?,
        })
    }

    pub fn memory_estimate_bytes(&self) -> Result<usize, ScalePixelsPlanError> {
        let source = usize::try_from(self.source_dimensions.pixel_count())
            .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?
            .checked_mul(16)
            .ok_or(ScalePixelsPlanError::ArithmeticOverflow)?;
        let output = usize::try_from(self.output_dimensions.pixel_count())
            .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?
            .checked_mul(16)
            .ok_or(ScalePixelsPlanError::ArithmeticOverflow)?;
        source
            .checked_add(output)
            .ok_or(ScalePixelsPlanError::ArithmeticOverflow)
    }
}

fn ensure_roi(
    roi: Roi,
    dimensions: RasterDimensions,
    source: bool,
) -> Result<(), ScalePixelsPlanError> {
    let dimensions = rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
        .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
    roi.within(dimensions).map_err(|_| {
        if source {
            ScalePixelsPlanError::RoiOutsideSource
        } else {
            ScalePixelsPlanError::RoiOutsideOutput
        }
    })?;
    Ok(())
}

fn resolve_geometry(
    ratio: f32,
    dimensions: RasterDimensions,
) -> Result<ScalePixelsGeometry, ScalePixelsPlanError> {
    resolve_roi_geometry(
        Roi::new(0, 0, dimensions.width(), dimensions.height())
            .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?,
        ratio,
    )
}

fn resolve_roi_geometry(
    input: Roi,
    ratio: f32,
) -> Result<ScalePixelsGeometry, ScalePixelsPlanError> {
    let x_factor = if ratio >= 1.0 { f64::from(ratio) } else { 1.0 };
    let y_factor = if ratio < 1.0 {
        1.0 / f64::from(ratio)
    } else {
        1.0
    };
    let origin_x = checked_floor(u64::from(input.x()), x_factor)?;
    let origin_y = checked_floor(u64::from(input.y()), y_factor)?;
    let width = checked_ceil(u64::from(input.width()), x_factor)?.max(1);
    let height = checked_ceil(u64::from(input.height()), y_factor)?.max(1);
    if width > u64::from(MAX_OUTPUT_DIMENSION) || height > u64::from(MAX_OUTPUT_DIMENSION) {
        return Err(ScalePixelsPlanError::OutputTooLarge);
    }
    let output_dimensions = RasterDimensions::new(
        u32::try_from(width).map_err(|_| ScalePixelsPlanError::OutputTooLarge)?,
        u32::try_from(height).map_err(|_| ScalePixelsPlanError::OutputTooLarge)?,
    )
    .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
    Ok(ScalePixelsGeometry {
        output_dimensions,
        output_origin: (
            u32::try_from(origin_x).map_err(|_| ScalePixelsPlanError::OutputTooLarge)?,
            u32::try_from(origin_y).map_err(|_| ScalePixelsPlanError::OutputTooLarge)?,
        ),
    })
}

fn checked_floor(value: u64, factor: f64) -> Result<u64, ScalePixelsPlanError> {
    let result = value as f64 * factor;
    if !result.is_finite() || result < 0.0 || result > u64::MAX as f64 {
        return Err(ScalePixelsPlanError::ArithmeticOverflow);
    }
    Ok(result.floor() as u64)
}

fn checked_ceil(value: u64, factor: f64) -> Result<u64, ScalePixelsPlanError> {
    let result = value as f64 * factor;
    if !result.is_finite() || result < 0.0 || result > u64::MAX as f64 {
        return Err(ScalePixelsPlanError::ArithmeticOverflow);
    }
    Ok(result.ceil() as u64)
}

fn finite_scale(value: f64) -> Result<f32, ScalePixelsPlanError> {
    let value = value as f32;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(ScalePixelsPlanError::ArithmeticOverflow)
    }
}

fn scale_roi(
    roi: Roi,
    x_factor: f32,
    y_factor: f32,
    bounds: RasterDimensions,
) -> Result<Roi, ScalePixelsPlanError> {
    let left = u32::try_from(checked_floor(u64::from(roi.x()), f64::from(x_factor))?)
        .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
    let top = u32::try_from(checked_floor(u64::from(roi.y()), f64::from(y_factor))?)
        .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?;
    let right = u32::try_from(checked_ceil(u64::from(roi.right()), f64::from(x_factor))?)
        .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?
        .min(bounds.width());
    let bottom = u32::try_from(checked_ceil(u64::from(roi.bottom()), f64::from(y_factor))?)
        .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)?
        .min(bounds.height());
    Roi::new(
        left.min(right),
        top.min(bottom),
        right.saturating_sub(left),
        bottom.saturating_sub(top),
    )
    .map_err(|_| ScalePixelsPlanError::ArithmeticOverflow)
}

fn transform_points(
    points: &mut [f32],
    x_factor: f32,
    y_factor: f32,
) -> Result<(), ScalePixelsGeometryError> {
    if points.len() % 2 != 0 {
        return Err(ScalePixelsGeometryError::InvalidPointBuffer);
    }
    for pair in points.chunks_exact_mut(2) {
        if pair.iter().any(|value| !value.is_finite()) {
            return Err(ScalePixelsGeometryError::NonFinitePoint);
        }
        pair[0] *= x_factor;
        pair[1] *= y_factor;
        if pair.iter().any(|value| !value.is_finite()) {
            return Err(ScalePixelsGeometryError::NonFiniteResult);
        }
    }
    Ok(())
}

fn plan_identity(
    config: &ScalePixelsConfig,
    source: RasterDimensions,
    output: RasterDimensions,
    preferences: ScalePixelsPreferences,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SCALEPIXELS_COMPATIBILITY_ID.as_bytes());
    hasher.update(SCALEPIXELS_SCHEMA_VERSION.to_le_bytes());
    hasher.update(config.pixel_aspect_ratio().to_bits().to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update([preferences.image().tag(), preferences.warp().tag()]);
    hasher.update(kernel_hash(preferences.image()));
    hasher.update(kernel_hash(preferences.warp()));
    hasher.finalize().into()
}

fn kernel_hash(kernel: super::ScalePixelsKernel) -> [u8; 32] {
    Sha256::digest(
        [
            b"rusttable.scalepixels.kernel.v1".as_slice(),
            &[kernel.tag()],
        ]
        .concat(),
    )
    .into()
}
