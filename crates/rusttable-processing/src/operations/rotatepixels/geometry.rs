use crate::RasterDimensions;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};
use std::f64::consts::PI;
use std::fmt;

use super::codec::{
    ROTATEPIXELS_COMPATIBILITY_ID, ROTATEPIXELS_IMPLEMENTATION_VERSION, ROTATEPIXELS_MAX_DIMENSION,
    ROTATEPIXELS_PARAMETER_VERSION, ROTATEPIXELS_SCHEMA_VERSION, RotatePixelsConfig,
    RotatePixelsInterpolation,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatePixelsMatrix {
    coefficients: [f32; 4],
    inverse: [f32; 4],
}

impl RotatePixelsMatrix {
    #[must_use]
    pub const fn coefficients(self) -> [f32; 4] {
        self.coefficients
    }

    #[must_use]
    pub const fn inverse(self) -> [f32; 4] {
        self.inverse
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotatePixelsPlanError {
    InvalidDimensions,
    DimensionTooLarge,
    CenterOutsideSource,
    InvalidScale,
    NonFiniteAngle,
    NonFiniteMatrix,
    EmptyOutput,
    ArithmeticOverflow,
    InvalidRoi,
}

impl fmt::Display for RotatePixelsPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "rotatepixels source dimensions are invalid",
            Self::DimensionTooLarge => "rotatepixels dimensions exceed the supported limit",
            Self::CenterOutsideSource => "rotatepixels center is outside the source",
            Self::InvalidScale => "rotatepixels scale must be finite and positive",
            Self::NonFiniteAngle => "rotatepixels angle must be finite",
            Self::NonFiniteMatrix => "rotatepixels matrix is non-finite",
            Self::EmptyOutput => "rotatepixels output is empty",
            Self::ArithmeticOverflow => "rotatepixels arithmetic overflowed",
            Self::InvalidRoi => "rotatepixels ROI is invalid",
        })
    }
}

impl std::error::Error for RotatePixelsPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotatePixelsCoordinateError {
    OddPointBuffer,
    NonFinitePoint,
    NonFiniteResult,
}

impl fmt::Display for RotatePixelsCoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OddPointBuffer => "rotatepixels points must contain x/y pairs",
            Self::NonFinitePoint => "rotatepixels point is non-finite",
            Self::NonFiniteResult => "rotatepixels point transform is non-finite",
        })
    }
}

impl std::error::Error for RotatePixelsCoordinateError {}

/// Immutable geometry, sampling, transform, and cache plan.
#[derive(Debug, Clone, PartialEq)]
pub struct RotatePixelsPlan {
    pub(crate) config: RotatePixelsConfig,
    pub(crate) source_dimensions: RasterDimensions,
    pub(crate) output_dimensions: RasterDimensions,
    pub(crate) source_roi: Roi,
    pub(crate) output_roi: Roi,
    pub(crate) matrix: RotatePixelsMatrix,
    pub(crate) interpolation: RotatePixelsInterpolation,
    pub(crate) scale: f64,
    pub(crate) enabled: bool,
    pub(crate) identity: [u8; 32],
}

impl RotatePixelsPlan {
    /// # Errors
    ///
    /// Returns an error when dimensions, parameters, geometry, or output bounds are invalid.
    pub fn new(
        source_dimensions: RasterDimensions,
        config: RotatePixelsConfig,
        interpolation: RotatePixelsInterpolation,
    ) -> Result<Self, RotatePixelsPlanError> {
        Self::new_with_scale(source_dimensions, config, interpolation, 1.0)
    }

    /// # Errors
    ///
    /// Returns an error when dimensions, scale, parameters, geometry, or output bounds are invalid.
    pub fn new_with_scale(
        source_dimensions: RasterDimensions,
        config: RotatePixelsConfig,
        interpolation: RotatePixelsInterpolation,
        scale: f64,
    ) -> Result<Self, RotatePixelsPlanError> {
        validate_dimensions(source_dimensions)?;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(RotatePixelsPlanError::InvalidScale);
        }
        let parameters = config.parameters();
        if !parameters.angle.is_finite() {
            return Err(RotatePixelsPlanError::NonFiniteAngle);
        }
        if parameters.rx >= source_dimensions.width() || parameters.ry >= source_dimensions.height()
        {
            return Err(RotatePixelsPlanError::CenterOutsideSource);
        }
        let enabled = parameters.rx != 0 || parameters.ry != 0;
        let matrix = rotation_matrix(parameters.angle, enabled)?;
        let source_roi = Roi::full(to_image_dimensions(source_dimensions));
        let output_dimensions = if enabled {
            compatibility_dimensions(source_dimensions, parameters.ry, interpolation, scale)?
        } else {
            source_dimensions
        };
        let output_roi = Roi::full(to_image_dimensions(output_dimensions));
        let identity = plan_identity(
            source_dimensions,
            output_dimensions,
            &config,
            interpolation,
            scale,
            matrix,
        )?;
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            source_roi,
            output_roi,
            matrix,
            interpolation,
            scale,
            enabled,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &RotatePixelsConfig {
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
    pub const fn source_roi(&self) -> Roi {
        self.source_roi
    }

    #[must_use]
    pub const fn output_roi(&self) -> Roi {
        self.output_roi
    }

    #[must_use]
    pub const fn matrix(&self) -> RotatePixelsMatrix {
        self.matrix
    }

    #[must_use]
    pub const fn interpolation(&self) -> RotatePixelsInterpolation {
        self.interpolation
    }

    #[must_use]
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// # Errors
    ///
    /// Returns an error for non-finite points or transforms.
    pub fn forward_point(&self, point: [f32; 2]) -> Result<[f32; 2], RotatePixelsCoordinateError> {
        transform_point(point, self.matrix.coefficients, self.center())
    }

    /// # Errors
    ///
    /// Returns an error for non-finite points or transforms.
    pub fn back_point(&self, point: [f32; 2]) -> Result<[f32; 2], RotatePixelsCoordinateError> {
        inverse_transform_point(point, self.matrix.inverse, self.center())
    }

    /// # Errors
    ///
    /// Returns an error for odd point buffers, non-finite points, or non-finite results.
    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), RotatePixelsCoordinateError> {
        transform_points(points, self.matrix.coefficients, self.center())
    }

    /// # Errors
    ///
    /// Returns an error for odd point buffers, non-finite points, or non-finite results.
    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), RotatePixelsCoordinateError> {
        inverse_transform_points(points, self.matrix.inverse, self.center())
    }

    /// # Errors
    ///
    /// Returns an error when the source ROI is outside the source image.
    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, RotatePixelsPlanError> {
        input
            .within(to_image_dimensions(self.source_dimensions))
            .map_err(|_| RotatePixelsPlanError::InvalidRoi)?;
        Roi::new(
            input.x(),
            input.y(),
            self.output_dimensions.width(),
            self.output_dimensions.height(),
        )
        .map_err(|_| RotatePixelsPlanError::ArithmeticOverflow)
    }

    /// # Errors
    ///
    /// Returns an error when the output ROI is outside the source-coordinate bounds.
    pub fn modify_roi_in(&self, output: Roi) -> Result<Roi, RotatePixelsPlanError> {
        output
            .within(to_image_dimensions(self.source_dimensions))
            .map_err(|_| RotatePixelsPlanError::InvalidRoi)?;
        if output.is_empty() || !self.enabled {
            return Ok(self.source_roi);
        }
        let corners = [
            [f64::from(output.x()), f64::from(output.y())],
            [f64::from(output.right()), f64::from(output.y())],
            [f64::from(output.x()), f64::from(output.bottom())],
            [f64::from(output.right()), f64::from(output.bottom())],
        ];
        let center = self.center_f64();
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for point in corners {
            let mapped = multiply(
                self.matrix.inverse,
                [f64_to_f32(point[0]), f64_to_f32(point[1])],
            );
            let mapped_x = f64::from(mapped[0]) + center[0];
            let mapped_y = f64::from(mapped[1]) + center[1];
            min_x = min_x.min(mapped_x);
            min_y = min_y.min(mapped_y);
            max_x = max_x.max(mapped_x);
            max_y = max_y.max(mapped_y);
        }
        let halo = f64::from(self.interpolation.compatibility_width()) * self.scale;
        let source_width = f64::from(self.source_dimensions.width()) * self.scale;
        let source_height = f64::from(self.source_dimensions.height()) * self.scale;
        let x = checked_u32((min_x - halo).max(0.0), Rounding::Floor)?
            .min(self.source_dimensions.width());
        let y = checked_u32((min_y - halo).max(0.0), Rounding::Floor)?
            .min(self.source_dimensions.height());
        let right = checked_u32((max_x + halo).max(0.0), Rounding::Ceil)?
            .min(checked_u32(source_width, Rounding::Ceil)?);
        let bottom = checked_u32((max_y + halo).max(0.0), Rounding::Ceil)?
            .min(checked_u32(source_height, Rounding::Ceil)?);
        if right <= x || bottom <= y {
            return Err(RotatePixelsPlanError::EmptyOutput);
        }
        Roi::new(x, y, right - x, bottom - y).map_err(|_| RotatePixelsPlanError::ArithmeticOverflow)
    }

    pub(crate) fn center(&self) -> [f32; 2] {
        let parameters = self.config.parameters();
        [
            f64_to_f32(f64::from(parameters.rx) * self.scale),
            f64_to_f32(f64::from(parameters.ry) * self.scale),
        ]
    }

    fn center_f64(&self) -> [f64; 2] {
        let parameters = self.config.parameters();
        [
            f64::from(parameters.rx) * self.scale,
            f64::from(parameters.ry) * self.scale,
        ]
    }
}

fn validate_dimensions(dimensions: RasterDimensions) -> Result<(), RotatePixelsPlanError> {
    if dimensions.width() == 0 || dimensions.height() == 0 {
        return Err(RotatePixelsPlanError::InvalidDimensions);
    }
    if dimensions.width() > ROTATEPIXELS_MAX_DIMENSION
        || dimensions.height() > ROTATEPIXELS_MAX_DIMENSION
    {
        return Err(RotatePixelsPlanError::DimensionTooLarge);
    }
    if dimensions.pixel_count() > usize::MAX as u64 {
        return Err(RotatePixelsPlanError::ArithmeticOverflow);
    }
    Ok(())
}

fn rotation_matrix(
    angle_degrees: f32,
    enabled: bool,
) -> Result<RotatePixelsMatrix, RotatePixelsPlanError> {
    if !angle_degrees.is_finite() {
        return Err(RotatePixelsPlanError::NonFiniteAngle);
    }
    if !enabled {
        return Ok(RotatePixelsMatrix {
            coefficients: [1.0, 0.0, 0.0, 1.0],
            inverse: [1.0, 0.0, 0.0, 1.0],
        });
    }
    let radians = f64::from(angle_degrees) * PI / 180.0;
    let cosine = f64_to_f32(radians.cos());
    let sine = f64_to_f32(radians.sin());
    let matrix = RotatePixelsMatrix {
        coefficients: [cosine, sine, -sine, cosine],
        inverse: [cosine, -sine, sine, cosine],
    };
    if matrix.coefficients.iter().all(|value| value.is_finite())
        && matrix.inverse.iter().all(|value| value.is_finite())
    {
        Ok(matrix)
    } else {
        Err(RotatePixelsPlanError::NonFiniteMatrix)
    }
}

fn compatibility_dimensions(
    source: RasterDimensions,
    ry: u32,
    interpolation: RotatePixelsInterpolation,
    scale: f64,
) -> Result<RasterDimensions, RotatePixelsPlanError> {
    let center_y = f64::from(ry) * scale;
    let width_delta = f64::from(source.width()) - f64::from(ry);
    let halo = f64::from(interpolation.compatibility_width()) * scale;
    let output_width = even_floor_nonnegative(2.0_f64.sqrt() * center_y - halo)?;
    let output_height = even_floor_nonnegative(2.0_f64.sqrt() * width_delta.abs() - halo)?;
    if output_width == 0 || output_height == 0 {
        return Err(RotatePixelsPlanError::EmptyOutput);
    }
    RasterDimensions::new(output_width, output_height)
        .map_err(|_| RotatePixelsPlanError::InvalidDimensions)
}

fn even_floor_nonnegative(value: f64) -> Result<u32, RotatePixelsPlanError> {
    if !value.is_finite() || value <= 0.0 {
        return Ok(0);
    }
    let floored = value.floor();
    let even = floored - floored.rem_euclid(2.0);
    checked_u32(even, Rounding::Floor)
}

#[derive(Debug, Clone, Copy)]
enum Rounding {
    Floor,
    Ceil,
}

fn checked_u32(value: f64, rounding: Rounding) -> Result<u32, RotatePixelsPlanError> {
    let rounded = match rounding {
        Rounding::Floor => value.floor(),
        Rounding::Ceil => value.ceil(),
    };
    if !rounded.is_finite() || rounded < 0.0 || rounded > f64::from(u32::MAX) {
        return Err(RotatePixelsPlanError::ArithmeticOverflow);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(rounded as u32)
}

fn to_image_dimensions(dimensions: RasterDimensions) -> rusttable_image::ImageDimensions {
    rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
        .expect("validated raster dimensions are nonzero")
}

fn transform_point(
    point: [f32; 2],
    matrix: [f32; 4],
    center: [f32; 2],
) -> Result<[f32; 2], RotatePixelsCoordinateError> {
    if !point.iter().all(|value| value.is_finite()) {
        return Err(RotatePixelsCoordinateError::NonFinitePoint);
    }
    let translated = [point[0] - center[0], point[1] - center[1]];
    let result = multiply(matrix, translated);
    if result.iter().all(|value| value.is_finite()) {
        Ok(result)
    } else {
        Err(RotatePixelsCoordinateError::NonFiniteResult)
    }
}

fn inverse_transform_point(
    point: [f32; 2],
    inverse: [f32; 4],
    center: [f32; 2],
) -> Result<[f32; 2], RotatePixelsCoordinateError> {
    if !point.iter().all(|value| value.is_finite()) {
        return Err(RotatePixelsCoordinateError::NonFinitePoint);
    }
    let result = multiply(inverse, point);
    let result = [result[0] + center[0], result[1] + center[1]];
    if result.iter().all(|value| value.is_finite()) {
        Ok(result)
    } else {
        Err(RotatePixelsCoordinateError::NonFiniteResult)
    }
}

fn transform_points(
    points: &mut [f32],
    matrix: [f32; 4],
    center: [f32; 2],
) -> Result<(), RotatePixelsCoordinateError> {
    if !points.len().is_multiple_of(2) {
        return Err(RotatePixelsCoordinateError::OddPointBuffer);
    }
    for pair in points.as_chunks::<2>().0 {
        if !pair.iter().all(|value| value.is_finite()) {
            return Err(RotatePixelsCoordinateError::NonFinitePoint);
        }
    }
    for pair in points.as_chunks_mut::<2>().0 {
        let transformed = transform_point([pair[0], pair[1]], matrix, center)?;
        pair.copy_from_slice(&transformed);
    }
    Ok(())
}

fn inverse_transform_points(
    points: &mut [f32],
    inverse: [f32; 4],
    center: [f32; 2],
) -> Result<(), RotatePixelsCoordinateError> {
    if !points.len().is_multiple_of(2) {
        return Err(RotatePixelsCoordinateError::OddPointBuffer);
    }
    for pair in points.as_chunks::<2>().0 {
        if !pair.iter().all(|value| value.is_finite()) {
            return Err(RotatePixelsCoordinateError::NonFinitePoint);
        }
    }
    for pair in points.as_chunks_mut::<2>().0 {
        let transformed = inverse_transform_point([pair[0], pair[1]], inverse, center)?;
        pair.copy_from_slice(&transformed);
    }
    Ok(())
}

fn multiply(matrix: [f32; 4], point: [f32; 2]) -> [f32; 2] {
    [
        matrix[0] * point[0] + matrix[1] * point[1],
        matrix[2] * point[0] + matrix[3] * point[1],
    ]
}

fn plan_identity(
    source: RasterDimensions,
    output: RasterDimensions,
    config: &RotatePixelsConfig,
    interpolation: RotatePixelsInterpolation,
    scale: f64,
    matrix: RotatePixelsMatrix,
) -> Result<[u8; 32], RotatePixelsPlanError> {
    let mut hasher = Sha256::new();
    hasher.update(ROTATEPIXELS_COMPATIBILITY_ID.as_bytes());
    hasher.update(ROTATEPIXELS_SCHEMA_VERSION.to_le_bytes());
    hasher.update(ROTATEPIXELS_PARAMETER_VERSION.to_le_bytes());
    hasher.update(ROTATEPIXELS_IMPLEMENTATION_VERSION.to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update(config.parameters().rx.to_le_bytes());
    hasher.update(config.parameters().ry.to_le_bytes());
    hasher.update(config.parameters().angle.to_bits().to_le_bytes());
    hasher.update(interpolation.tag().to_le_bytes());
    hasher.update(scale.to_bits().to_le_bytes());
    for coefficient in matrix.coefficients.into_iter().chain(matrix.inverse) {
        if !coefficient.is_finite() {
            return Err(RotatePixelsPlanError::NonFiniteMatrix);
        }
        hasher.update(coefficient.to_bits().to_le_bytes());
    }
    if let Some(source) = config.opaque_source() {
        let length =
            u64::try_from(source.len()).map_err(|_| RotatePixelsPlanError::ArithmeticOverflow)?;
        hasher.update(length.to_le_bytes());
        hasher.update(source);
    }
    Ok(hasher.finalize().into())
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn f64_to_f32(value: f64) -> f32 {
    value as f32
}
