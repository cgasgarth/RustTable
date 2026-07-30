#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_midpoint,
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::too_many_arguments
)]

use crate::RasterDimensions;
use rusttable_image::Roi;
use sha2::{Digest, Sha256};
use std::f64::consts::PI;

use super::codec::{
    CLIPPING_COMPATIBILITY_ID, CLIPPING_IMPLEMENTATION_VERSION, CLIPPING_MAX_DIMENSION,
    CLIPPING_PARAMETER_VERSION, ClippingConfig, ClippingInterpolation,
};
use crate::operations::perspective::{Homography, HomographyError, Point, Quad, TransformError};

const MIN_OUTPUT_EDGE: u32 = 1;
const EPSILON: f64 = 1.0e-8;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CropRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl CropRect {
    #[must_use]
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
    #[must_use]
    pub const fn x(self) -> f64 {
        self.x
    }
    #[must_use]
    pub const fn y(self) -> f64 {
        self.y
    }
    #[must_use]
    pub const fn width(self) -> f64 {
        self.width
    }
    #[must_use]
    pub const fn height(self) -> f64 {
        self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeystoneMode {
    None,
    Vertical,
    Horizontal,
    Full,
    OldSystem,
}

impl KeystoneMode {
    fn from_value(value: i32) -> Result<Self, ClippingPlanError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Vertical),
            2 => Ok(Self::Horizontal),
            3 => Ok(Self::Full),
            4 => Ok(Self::OldSystem),
            _ => Err(ClippingPlanError::UnknownKeystoneMode(value)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClippingPlanError {
    InvalidDimensions,
    DimensionTooLarge,
    InvalidCrop,
    InvalidAspect,
    InvalidControlPoint,
    UnknownKeystoneMode(i32),
    InvalidApplyMode,
    Homography,
    EmptyOutput,
    ArithmeticOverflow,
    InvalidRoi,
}

impl std::fmt::Display for ClippingPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidDimensions => f.write_str("clipping source dimensions are invalid"),
            Self::DimensionTooLarge => {
                f.write_str("clipping dimensions exceed the supported limit")
            }
            Self::InvalidCrop => f.write_str("clipping crop rectangle is invalid"),
            Self::InvalidAspect => f.write_str("clipping aspect ratio is invalid"),
            Self::InvalidControlPoint => f.write_str("clipping control point is invalid"),
            Self::UnknownKeystoneMode(value) => {
                write!(f, "clipping keystone mode {value} is unknown")
            }
            Self::InvalidApplyMode => f.write_str("clipping keystone apply mode is invalid"),
            Self::Homography => f.write_str("clipping transform is singular or ill-conditioned"),
            Self::EmptyOutput => f.write_str("clipping output is empty"),
            Self::ArithmeticOverflow => f.write_str("clipping geometry arithmetic overflowed"),
            Self::InvalidRoi => f.write_str("clipping ROI is invalid"),
        }
    }
}
impl std::error::Error for ClippingPlanError {}

impl From<HomographyError> for ClippingPlanError {
    fn from(_: HomographyError) -> Self {
        Self::Homography
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformPointError {
    NonFinite,
    AtInfinity,
}

impl From<TransformError> for TransformPointError {
    fn from(value: TransformError) -> Self {
        match value {
            TransformError::NonFinitePoint | TransformError::NonFiniteResult => Self::NonFinite,
            TransformError::AtInfinity => Self::AtInfinity,
            TransformError::InvalidBounds
            | TransformError::OutsideSource
            | TransformError::Overflow => Self::NonFinite,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClippingPlan {
    config: ClippingConfig,
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    source_roi: Roi,
    output_roi: Roi,
    crop: CropRect,
    interpolation: ClippingInterpolation,
    source_to_output: Homography,
    output_to_source: Homography,
    identity: [u8; 32],
}

impl ClippingPlan {
    pub fn new(
        source_dimensions: RasterDimensions,
        config: ClippingConfig,
        interpolation: ClippingInterpolation,
    ) -> Result<Self, ClippingPlanError> {
        validate_dimensions(source_dimensions)?;
        let p = config.parameters();
        validate_crop(p.cx, p.cy, p.cw, p.ch)?;
        if !(-180.0..=180.0).contains(&p.angle) {
            return Err(ClippingPlanError::InvalidCrop);
        }
        let mode = KeystoneMode::from_value(p.k_type)?;
        if !(0..=1).contains(&p.k_apply) {
            return Err(ClippingPlanError::InvalidApplyMode);
        }
        let source_to_world = source_to_world(source_dimensions, p, mode)?;
        let world_bounds = bounds(source_to_world, source_dimensions)?;
        let world_to_source = source_to_world.inverse()?;
        let crop = if p.crop_auto {
            automatic_crop(
                world_bounds,
                world_to_source,
                source_dimensions,
                p.cx,
                p.cy,
                p.cw,
                p.ch,
                p.ratio_n,
                p.ratio_d,
            )?
        } else {
            CropRect::new(
                world_bounds.0 + p.cx as f64 * (world_bounds.2 - world_bounds.0),
                world_bounds.1 + p.cy as f64 * (world_bounds.3 - world_bounds.1),
                (p.cw - p.cx) as f64 * (world_bounds.2 - world_bounds.0),
                (p.ch - p.cy) as f64 * (world_bounds.3 - world_bounds.1),
            )
        };
        if crop.width <= 0.0
            || crop.height <= 0.0
            || !crop.width.is_finite()
            || !crop.height.is_finite()
        {
            return Err(ClippingPlanError::EmptyOutput);
        }
        let output_dimensions = RasterDimensions::new(
            rounded_dimension(crop.width)?,
            rounded_dimension(crop.height)?,
        )
        .map_err(|_| ClippingPlanError::EmptyOutput)?;
        let source_to_output = source_to_world.then(translation(-crop.x, -crop.y));
        let output_to_source = source_to_output.inverse()?;
        let source_roi = Roi::full(image_dimensions(source_dimensions));
        let output_roi = Roi::full(image_dimensions(output_dimensions));
        let identity = plan_identity(
            &config,
            source_dimensions,
            output_dimensions,
            crop,
            interpolation,
            source_to_output,
        );
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            source_roi,
            output_roi,
            crop,
            interpolation,
            source_to_output,
            output_to_source,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &ClippingConfig {
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
    pub const fn crop(&self) -> CropRect {
        self.crop
    }
    #[must_use]
    pub const fn interpolation(&self) -> ClippingInterpolation {
        self.interpolation
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.output_dimensions == self.source_dimensions
            && self.source_to_output.coefficients() == Homography::identity().coefficients()
    }

    pub fn forward_point(&self, point: Point) -> Result<Point, TransformPointError> {
        self.source_to_output.apply(point).map_err(Into::into)
    }
    pub fn back_point(&self, point: Point) -> Result<Point, TransformPointError> {
        self.output_to_source.apply(point).map_err(Into::into)
    }
    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, ClippingPlanError> {
        input
            .within(self.source_roi_dimensions())
            .map_err(|_| ClippingPlanError::InvalidRoi)?;
        Roi::new(
            input.x(),
            input.y(),
            self.output_dimensions.width(),
            self.output_dimensions.height(),
        )
        .map_err(|_| ClippingPlanError::ArithmeticOverflow)
    }
    pub fn modify_roi_in(&self, output: Roi) -> Result<Roi, ClippingPlanError> {
        output
            .within(self.output_roi_dimensions())
            .map_err(|_| ClippingPlanError::InvalidRoi)?;
        if output.is_empty() {
            return Ok(self.source_roi);
        }
        let points = [
            Point::new(f64::from(output.x()), f64::from(output.y())),
            Point::new(f64::from(output.right()), f64::from(output.y())),
            Point::new(f64::from(output.x()), f64::from(output.bottom())),
            Point::new(f64::from(output.right()), f64::from(output.bottom())),
        ];
        let mapped = points
            .iter()
            .map(|point| self.back_point(*point))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| ClippingPlanError::Homography)?;
        let halo = f64::from(self.interpolation.support());
        let min_x = mapped
            .iter()
            .map(|point| point.x())
            .fold(f64::INFINITY, f64::min)
            - halo;
        let min_y = mapped
            .iter()
            .map(|point| point.y())
            .fold(f64::INFINITY, f64::min)
            - halo;
        let max_x = mapped
            .iter()
            .map(|point| point.x())
            .fold(f64::NEG_INFINITY, f64::max)
            + halo;
        let max_y = mapped
            .iter()
            .map(|point| point.y())
            .fold(f64::NEG_INFINITY, f64::max)
            + halo;
        let x = min_x
            .floor()
            .max(0.0)
            .min(f64::from(self.source_dimensions.width())) as u32;
        let y = min_y
            .floor()
            .max(0.0)
            .min(f64::from(self.source_dimensions.height())) as u32;
        let right = max_x
            .ceil()
            .max(0.0)
            .min(f64::from(self.source_dimensions.width())) as u32;
        let bottom = max_y
            .ceil()
            .max(0.0)
            .min(f64::from(self.source_dimensions.height())) as u32;
        Roi::new(x, y, right.saturating_sub(x), bottom.saturating_sub(y))
            .map_err(|_| ClippingPlanError::InvalidRoi)
    }

    fn source_roi_dimensions(&self) -> rusttable_image::ImageDimensions {
        image_dimensions(self.source_dimensions)
    }
    fn output_roi_dimensions(&self) -> rusttable_image::ImageDimensions {
        image_dimensions(self.output_dimensions)
    }
}

fn validate_dimensions(dimensions: RasterDimensions) -> Result<(), ClippingPlanError> {
    if dimensions.width() == 0 || dimensions.height() == 0 {
        return Err(ClippingPlanError::InvalidDimensions);
    }
    if dimensions.width() > CLIPPING_MAX_DIMENSION || dimensions.height() > CLIPPING_MAX_DIMENSION {
        return Err(ClippingPlanError::DimensionTooLarge);
    }
    if dimensions.pixel_count() > usize::MAX as u64 {
        return Err(ClippingPlanError::ArithmeticOverflow);
    }
    Ok(())
}

fn validate_crop(cx: f32, cy: f32, cw: f32, ch: f32) -> Result<(), ClippingPlanError> {
    if ![cx, cy, cw, ch]
        .iter()
        .all(|value| (0.0..=1.0).contains(value))
        || cx >= cw
        || cy >= ch
    {
        return Err(ClippingPlanError::InvalidCrop);
    }
    Ok(())
}

fn source_to_world(
    dimensions: RasterDimensions,
    p: super::codec::ClippingParametersV5,
    mode: KeystoneMode,
) -> Result<Homography, ClippingPlanError> {
    let width = f64::from(dimensions.width());
    let height = f64::from(dimensions.height());
    let mut transform = Homography::identity();
    if p.k_apply == 1 {
        let quad = Quad::new(
            Point::new(f64::from(p.kxa) * width, f64::from(p.kya) * height),
            Point::new(f64::from(p.kxb) * width, f64::from(p.kyb) * height),
            Point::new(f64::from(p.kxc) * width, f64::from(p.kyc) * height),
            Point::new(f64::from(p.kxd) * width, f64::from(p.kyd) * height),
        );
        transform =
            Homography::from_quad(quad, dimensions.width().max(2), dimensions.height().max(2))?;
    } else if !matches!(mode, KeystoneMode::None) && (p.k_h != 0.0 || p.k_v != 0.0) {
        let center_x = width * 0.5;
        let center_y = height * 0.5;
        let old = Homography::new([
            1.0,
            0.0,
            0.0,
            0.0,
            1.0,
            0.0,
            f64::from(p.k_v) / width,
            f64::from(p.k_h) / height,
            1.0,
        ])?;
        transform = translation(-center_x, -center_y)
            .then(old)
            .then(translation(center_x, center_y));
    }
    let angle = f64::from(p.angle) * PI / 180.0;
    let (sin, cos) = angle.sin_cos();
    let rotation = Homography::new([
        cos,
        sin,
        width * 0.5 - cos * width * 0.5 - sin * height * 0.5,
        -sin,
        cos,
        height * 0.5 + sin * width * 0.5 - cos * height * 0.5,
        0.0,
        0.0,
        1.0,
    ])?;
    Ok(transform.then(rotation))
}

fn automatic_crop(
    bounds: (f64, f64, f64, f64),
    world_to_source: Homography,
    dimensions: RasterDimensions,
    cx: f32,
    cy: f32,
    cw: f32,
    ch: f32,
    ratio_n: i32,
    ratio_d: i32,
) -> Result<CropRect, ClippingPlanError> {
    let requested_aspect = if ratio_n > 0 && ratio_d > 0 {
        f64::from(ratio_n) / f64::from(ratio_d)
    } else {
        f64::from(cw - cx) * f64::from(dimensions.width())
            / (f64::from(ch - cy) * f64::from(dimensions.height()))
    };
    if !requested_aspect.is_finite() || requested_aspect <= 0.0 {
        return Err(ClippingPlanError::InvalidAspect);
    }
    let available_width = bounds.2 - bounds.0;
    let available_height = bounds.3 - bounds.1;
    let (base_width, base_height) = if available_width / available_height > requested_aspect {
        (available_height * requested_aspect, available_height)
    } else {
        (available_width, available_width / requested_aspect)
    };
    let mut low = 0.0;
    let mut high = 1.0;
    for _ in 0..48 {
        let scale = (low + high) * 0.5;
        let candidate = centered_rect(bounds, base_width * scale, base_height * scale);
        if rectangle_inside_source(candidate, world_to_source, dimensions) {
            low = scale;
        } else {
            high = scale;
        }
    }
    let candidate = centered_rect(bounds, base_width * low, base_height * low);
    if candidate.width <= 0.0 || candidate.height <= 0.0 {
        return Err(ClippingPlanError::EmptyOutput);
    }
    Ok(candidate)
}

fn centered_rect(bounds: (f64, f64, f64, f64), width: f64, height: f64) -> CropRect {
    CropRect::new(
        (bounds.0 + bounds.2 - width) * 0.5,
        (bounds.1 + bounds.3 - height) * 0.5,
        width,
        height,
    )
}

fn rectangle_inside_source(
    rect: CropRect,
    world_to_source: Homography,
    dimensions: RasterDimensions,
) -> bool {
    [
        Point::new(rect.x, rect.y),
        Point::new(rect.x + rect.width, rect.y),
        Point::new(rect.x + rect.width, rect.y + rect.height),
        Point::new(rect.x, rect.y + rect.height),
    ]
    .iter()
    .all(|point| {
        world_to_source.apply(*point).is_ok_and(|mapped| {
            mapped.x() >= -EPSILON
                && mapped.y() >= -EPSILON
                && mapped.x() <= f64::from(dimensions.width()) + EPSILON
                && mapped.y() <= f64::from(dimensions.height()) + EPSILON
        })
    })
}

fn bounds(
    transform: Homography,
    dimensions: RasterDimensions,
) -> Result<(f64, f64, f64, f64), ClippingPlanError> {
    let width = f64::from(dimensions.width());
    let height = f64::from(dimensions.height());
    let points = [
        Point::new(0.0, 0.0),
        Point::new(width, 0.0),
        Point::new(width, height),
        Point::new(0.0, height),
    ];
    let mapped = points
        .iter()
        .map(|point| transform.apply(*point))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| ClippingPlanError::Homography)?;
    Ok((
        mapped
            .iter()
            .map(|point| point.x())
            .fold(f64::INFINITY, f64::min),
        mapped
            .iter()
            .map(|point| point.y())
            .fold(f64::INFINITY, f64::min),
        mapped
            .iter()
            .map(|point| point.x())
            .fold(f64::NEG_INFINITY, f64::max),
        mapped
            .iter()
            .map(|point| point.y())
            .fold(f64::NEG_INFINITY, f64::max),
    ))
}

fn rounded_dimension(value: f64) -> Result<u32, ClippingPlanError> {
    let rounded = value.round();
    if !rounded.is_finite()
        || rounded < f64::from(MIN_OUTPUT_EDGE)
        || rounded > f64::from(CLIPPING_MAX_DIMENSION)
    {
        return Err(ClippingPlanError::EmptyOutput);
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Ok(rounded as u32)
}

fn translation(x: f64, y: f64) -> Homography {
    Homography::new([1.0, 0.0, x, 0.0, 1.0, y, 0.0, 0.0, 1.0]).expect("translation is invertible")
}

fn image_dimensions(dimensions: RasterDimensions) -> rusttable_image::ImageDimensions {
    rusttable_image::ImageDimensions::new(dimensions.width(), dimensions.height())
        .expect("validated dimensions")
}

fn plan_identity(
    config: &ClippingConfig,
    source: RasterDimensions,
    output: RasterDimensions,
    crop: CropRect,
    interpolation: ClippingInterpolation,
    transform: Homography,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(CLIPPING_COMPATIBILITY_ID.as_bytes());
    hasher.update(CLIPPING_PARAMETER_VERSION.to_le_bytes());
    hasher.update(CLIPPING_IMPLEMENTATION_VERSION.to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update(interpolation.tag().to_le_bytes());
    for value in [crop.x, crop.y, crop.width, crop.height] {
        hasher.update(value.to_bits().to_le_bytes());
    }
    for coefficient in transform.coefficients() {
        hasher.update(coefficient.to_bits().to_le_bytes());
    }
    if let Some(source) = config.opaque_source() {
        hasher.update((source.len() as u64).to_le_bytes());
        hasher.update(source);
    }
    hasher.finalize().into()
}
