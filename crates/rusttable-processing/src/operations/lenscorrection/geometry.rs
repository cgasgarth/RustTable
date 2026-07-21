use super::parameters::{
    CorrectionFlags, LensCorrectionConfig, LensCorrectionMethod, LensCorrectionMode,
};
use super::snapshot::{
    DistortionCalibration, LensProfile, LensfunSnapshot, TcaCalibration, VignettingCalibration,
};
use crate::RasterDimensions;
use rusttable_image::{ImageDimensions, Roi};
use sha2::{Digest, Sha256};
use std::fmt;

const MAX_DIMENSION: u32 = 1 << 30;
const INVERSE_ITERATIONS: usize = 12;
const ROI_HALO: f64 = 2.0;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct RadialCalibration {
    a: f64,
    b: f64,
    c: f64,
    poly3: bool,
}

impl RadialCalibration {
    const IDENTITY: Self = Self {
        a: 0.0,
        b: 0.0,
        c: 0.0,
        poly3: false,
    };

    fn factor(self, radius: f64) -> f64 {
        let radius_squared = radius * radius;
        if self.poly3 {
            1.0 + self.a * radius_squared
        } else {
            1.0 + self.c * radius + self.b * radius_squared + self.a * radius_squared * radius
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LensCorrectionPlan {
    config: LensCorrectionConfig,
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    source_roi: Roi,
    output_roi: Roi,
    camera_crop_factor: f64,
    lens: Option<LensProfile>,
    distortion: RadialCalibration,
    tca: Option<TcaCalibration>,
    vignetting: Option<VignettingCalibration>,
    identity: [u8; 32],
}

impl LensCorrectionPlan {
    /// Resolves a configuration against the immutable pinned Lensfun snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid dimensions, unresolved camera/lens data, or
    /// non-finite geometry.
    pub fn new(
        source_dimensions: RasterDimensions,
        config: LensCorrectionConfig,
    ) -> Result<Self, LensCorrectionPlanError> {
        validate_dimensions(source_dimensions)?;
        let parameters = config.parameters();
        let snapshot = LensfunSnapshot::pinned();
        let camera = snapshot.cameras.iter().copied().find(|camera| {
            same_name(camera.model, &parameters.camera)
                || camera
                    .aliases
                    .iter()
                    .any(|alias| same_name(alias, &parameters.camera))
        });
        let camera = if parameters.camera.is_empty() {
            None
        } else {
            Some(camera.ok_or(LensCorrectionPlanError::CameraNotFound)?)
        };
        let lens = if parameters.lens.is_empty() {
            None
        } else {
            snapshot
                .find_lens(camera, &parameters.lens)
                .ok_or(LensCorrectionPlanError::LensNotFound)?
                .into()
        };
        let camera_crop_factor =
            f64::from(camera.map_or(parameters.crop_factor, |camera| camera.crop_factor));
        if camera_crop_factor <= 0.0 || !camera_crop_factor.is_finite() {
            return Err(LensCorrectionPlanError::InvalidCropFactor);
        }
        let distortion = if parameters
            .modify_flags
            .contains(CorrectionFlags::DISTORTION)
            && parameters.method == LensCorrectionMethod::Lensfun
        {
            lens.and_then(|lens| lens.distortion_at(parameters.focal_length))
                .map_or(RadialCalibration::IDENTITY, radial_from_distortion)
        } else {
            RadialCalibration::IDENTITY
        };
        let tca = if parameters.modify_flags.contains(CorrectionFlags::TCA)
            && parameters.method == LensCorrectionMethod::Lensfun
        {
            if parameters.tca_override {
                Some(TcaCalibration {
                    focal: parameters.focal_length,
                    red_linear: parameters.tca_red,
                    red_cubic: 0.0,
                    blue_linear: parameters.tca_blue,
                    blue_cubic: 0.0,
                })
            } else {
                lens.and_then(|lens| lens.tca_at(parameters.focal_length))
            }
        } else {
            None
        };
        let vignetting = if parameters
            .modify_flags
            .contains(CorrectionFlags::VIGNETTING)
        {
            lens.and_then(|lens| {
                lens.vignetting_at(
                    parameters.focal_length,
                    parameters.aperture,
                    parameters.distance,
                )
            })
        } else {
            None
        };
        let source_roi = Roi::full(to_image_dimensions(source_dimensions)?);
        let output_dimensions = source_dimensions;
        let output_roi = source_roi;
        let identity = plan_identity(
            &config,
            source_dimensions,
            distortion,
            tca,
            vignetting,
            camera_crop_factor,
        );
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions,
            source_roi,
            output_roi,
            camera_crop_factor,
            lens,
            distortion,
            tca,
            vignetting,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &LensCorrectionConfig {
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
    pub const fn lens(&self) -> Option<LensProfile> {
        self.lens
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    /// Maps a source point to the corrected output's green channel.
    ///
    /// # Errors
    ///
    /// Returns an error for non-finite input or output coordinates.
    pub fn forward_point(
        &self,
        point: [f32; 2],
    ) -> Result<[f32; 2], LensCorrectionCoordinateError> {
        self.map_point(point, 1.0)
    }

    /// Maps an output point back to source coordinates for inverse resampling.
    ///
    /// # Errors
    ///
    /// Returns an error if the bounded inverse does not converge.
    pub fn back_point(&self, point: [f32; 2]) -> Result<[f32; 2], LensCorrectionCoordinateError> {
        self.inverse_map_point(point, 1.0)
    }

    /// Maps a point for one color plane, applying calibrated TCA where present.
    pub fn back_channel_point(
        &self,
        point: [f32; 2],
        channel: usize,
    ) -> Result<[f32; 2], LensCorrectionCoordinateError> {
        let channel_scale = self.channel_scale(point, channel)?;
        self.inverse_map_point(point, channel_scale)
    }

    /// Maps a source ROI to the corresponding output-coordinate ROI.
    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, LensCorrectionPlanError> {
        input
            .within(to_image_dimensions(self.source_dimensions)?)
            .map_err(|_| LensCorrectionPlanError::InvalidRoi)?;
        Ok(input)
    }

    /// Computes a checked source enclosure for an output ROI, including the
    /// bilinear sampler's deterministic two-pixel compatibility halo.
    pub fn modify_roi_in(&self, output: Roi) -> Result<Roi, LensCorrectionPlanError> {
        output
            .within(to_image_dimensions(self.output_dimensions)?)
            .map_err(|_| LensCorrectionPlanError::InvalidRoi)?;
        if output.is_empty() {
            return Ok(self.source_roi);
        }
        let points = [
            [f64::from(output.x()), f64::from(output.y())],
            [f64::from(output.right()), f64::from(output.y())],
            [f64::from(output.x()), f64::from(output.bottom())],
            [f64::from(output.right()), f64::from(output.bottom())],
        ];
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for point in points {
            let mapped = self
                .inverse_map_point_f64(
                    Point {
                        x: point[0],
                        y: point[1],
                    },
                    1.0,
                )
                .map_err(|_| LensCorrectionPlanError::TransformFailed)?;
            min_x = min_x.min(mapped.x);
            min_y = min_y.min(mapped.y);
            max_x = max_x.max(mapped.x);
            max_y = max_y.max(mapped.y);
        }
        let width = f64::from(self.source_dimensions.width());
        let height = f64::from(self.source_dimensions.height());
        let x = checked_coordinate((min_x - ROI_HALO).floor().max(0.0))?
            .min(self.source_dimensions.width());
        let y = checked_coordinate((min_y - ROI_HALO).floor().max(0.0))?
            .min(self.source_dimensions.height());
        let right =
            checked_coordinate((max_x + ROI_HALO).ceil().max(0.0))?.min(checked_coordinate(width)?);
        let bottom = checked_coordinate((max_y + ROI_HALO).ceil().max(0.0))?
            .min(checked_coordinate(height)?);
        if right <= x || bottom <= y {
            return Err(LensCorrectionPlanError::EmptyRoi);
        }
        Roi::new(x, y, right - x, bottom - y)
            .map_err(|_| LensCorrectionPlanError::ArithmeticOverflow)
    }

    fn map_point(
        &self,
        point: [f32; 2],
        channel_scale: f64,
    ) -> Result<[f32; 2], LensCorrectionCoordinateError> {
        let point = finite_point(point)?;
        let mapped = self.map_point_f64(point, channel_scale);
        checked_point(mapped)
    }

    fn inverse_map_point(
        &self,
        point: [f32; 2],
        channel_scale: f64,
    ) -> Result<[f32; 2], LensCorrectionCoordinateError> {
        let point = finite_point(point)?;
        let mapped = self
            .inverse_map_point_f64(point, channel_scale)
            .map_err(|_| LensCorrectionCoordinateError::InverseDidNotConverge)?;
        checked_point(mapped)
    }

    fn map_point_f64(&self, point: Point, channel_scale: f64) -> Point {
        let normalized = self.normalize(point);
        let radial = Self::radial_map(normalized, self.radial_factor());
        let scaled = Point {
            x: radial.x * self.config.parameters().scale_f64() * channel_scale,
            y: radial.y * self.config.parameters().scale_f64() * channel_scale,
        };
        self.denormalize(scaled)
    }

    fn inverse_map_point_f64(
        &self,
        point: Point,
        channel_scale: f64,
    ) -> Result<Point, LensCorrectionCoordinateError> {
        let normalized = self.normalize(point);
        let scaled = Point {
            x: normalized.x / (self.config.parameters().scale_f64() * channel_scale),
            y: normalized.y / (self.config.parameters().scale_f64() * channel_scale),
        };
        let radial = invert_radial(scaled, self.radial_factor())?;
        Ok(self.denormalize(radial))
    }

    fn radial_factor(&self) -> RadialCalibration {
        match self.config.parameters().mode {
            LensCorrectionMode::Correct => self.distortion,
            LensCorrectionMode::Distort => invert_calibration(self.distortion),
        }
    }

    fn radial_map(point: Point, calibration: RadialCalibration) -> Point {
        let radius = point.x.hypot(point.y);
        let factor = calibration.factor(radius);
        Point {
            x: point.x * factor,
            y: point.y * factor,
        }
    }

    fn normalize(&self, point: Point) -> Point {
        let width = f64::from(self.source_dimensions.width());
        let height = f64::from(self.source_dimensions.height());
        let radius = width.min(height) * 0.5 * self.camera_crop_factor;
        Point {
            x: (point.x - (width - 1.0) * 0.5) / radius,
            y: (point.y - (height - 1.0) * 0.5) / radius,
        }
    }

    fn denormalize(&self, point: Point) -> Point {
        let width = f64::from(self.source_dimensions.width());
        let height = f64::from(self.source_dimensions.height());
        let radius = width.min(height) * 0.5 * self.camera_crop_factor;
        Point {
            x: point.x * radius + (width - 1.0) * 0.5,
            y: point.y * radius + (height - 1.0) * 0.5,
        }
    }

    fn channel_scale(
        &self,
        point: [f32; 2],
        channel: usize,
    ) -> Result<f64, LensCorrectionCoordinateError> {
        if channel == 1 {
            return Ok(1.0);
        }
        let Some(tca) = self.tca else {
            return Ok(1.0);
        };
        let normalized = self.normalize(finite_point(point)?);
        let radius_squared = normalized
            .x
            .mul_add(normalized.x, normalized.y * normalized.y);
        let (linear, cubic) = if channel == 0 {
            (f64::from(tca.red_linear), f64::from(tca.red_cubic))
        } else if channel == 2 {
            (f64::from(tca.blue_linear), f64::from(tca.blue_cubic))
        } else {
            return Err(LensCorrectionCoordinateError::InvalidChannel);
        };
        let scale = linear + cubic * radius_squared;
        scale
            .is_finite()
            .then_some(scale)
            .ok_or(LensCorrectionCoordinateError::NonFiniteResult)
    }

    pub(crate) fn vignetting_calibration(&self) -> Option<VignettingCalibration> {
        self.vignetting
    }
}

impl LensCorrectionParametersExt for super::parameters::LensCorrectionParametersV1 {
    fn scale_f64(&self) -> f64 {
        f64::from(self.scale)
    }
}

trait LensCorrectionParametersExt {
    fn scale_f64(&self) -> f64;
}

fn radial_from_distortion(calibration: DistortionCalibration) -> RadialCalibration {
    match calibration {
        DistortionCalibration::Poly3 { k1, .. } => RadialCalibration {
            a: f64::from(k1),
            b: 0.0,
            c: 0.0,
            poly3: true,
        },
        DistortionCalibration::PtLens { a, b, c, .. } => RadialCalibration {
            a: f64::from(a),
            b: f64::from(b),
            c: f64::from(c),
            poly3: false,
        },
    }
}

fn invert_calibration(calibration: RadialCalibration) -> RadialCalibration {
    RadialCalibration {
        a: -calibration.a,
        b: -calibration.b,
        c: -calibration.c,
        poly3: calibration.poly3,
    }
}

fn invert_radial(
    point: Point,
    calibration: RadialCalibration,
) -> Result<Point, LensCorrectionCoordinateError> {
    let target_radius = point.x.hypot(point.y);
    if target_radius == 0.0 {
        return Ok(point);
    }
    let mut radius = target_radius;
    for _ in 0..INVERSE_ITERATIONS {
        let factor = calibration.factor(radius);
        let value = radius * factor - target_radius;
        let epsilon = 1.0e-6;
        let derivative = ((radius + epsilon) * calibration.factor(radius + epsilon)
            - (radius - epsilon).max(0.0) * calibration.factor((radius - epsilon).max(0.0)))
            / (epsilon + epsilon.min(radius));
        if !factor.is_finite() || !derivative.is_finite() || derivative.abs() < 1.0e-9 {
            return Err(LensCorrectionCoordinateError::InverseDidNotConverge);
        }
        radius -= value / derivative;
        if !radius.is_finite() || radius < 0.0 {
            return Err(LensCorrectionCoordinateError::InverseDidNotConverge);
        }
    }
    let scale = radius / target_radius;
    Ok(Point {
        x: point.x * scale,
        y: point.y * scale,
    })
}

fn validate_dimensions(dimensions: RasterDimensions) -> Result<(), LensCorrectionPlanError> {
    if dimensions.width() > MAX_DIMENSION || dimensions.height() > MAX_DIMENSION {
        return Err(LensCorrectionPlanError::DimensionTooLarge);
    }
    Ok(())
}

fn to_image_dimensions(
    dimensions: RasterDimensions,
) -> Result<ImageDimensions, LensCorrectionPlanError> {
    ImageDimensions::new(dimensions.width(), dimensions.height())
        .map_err(|_| LensCorrectionPlanError::InvalidDimensions)
}

fn finite_point(point: [f32; 2]) -> Result<Point, LensCorrectionCoordinateError> {
    if point.iter().all(|value| value.is_finite()) {
        Ok(Point {
            x: f64::from(point[0]),
            y: f64::from(point[1]),
        })
    } else {
        Err(LensCorrectionCoordinateError::NonFinitePoint)
    }
}

fn checked_point(point: Point) -> Result<[f32; 2], LensCorrectionCoordinateError> {
    if point.x.is_finite() && point.y.is_finite() {
        let x = point.x as f32;
        let y = point.y as f32;
        if x.is_finite() && y.is_finite() {
            return Ok([x, y]);
        }
    }
    Err(LensCorrectionCoordinateError::NonFiniteResult)
}

fn checked_coordinate(value: f64) -> Result<u32, LensCorrectionPlanError> {
    if !value.is_finite() || value < 0.0 || value > f64::from(u32::MAX) {
        return Err(LensCorrectionPlanError::ArithmeticOverflow);
    }
    Ok(value as u32)
}

fn same_name(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
}

fn plan_identity(
    config: &LensCorrectionConfig,
    dimensions: RasterDimensions,
    distortion: RadialCalibration,
    tca: Option<TcaCalibration>,
    vignetting: Option<VignettingCalibration>,
    crop_factor: f64,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(super::LENS_CORRECTION_COMPATIBILITY_ID.as_bytes());
    hasher.update(super::snapshot::LENSFUN_DATABASE_COMMIT.as_bytes());
    hasher.update(dimensions.width().to_le_bytes());
    hasher.update(dimensions.height().to_le_bytes());
    hasher.update(postcard::to_allocvec(config.parameters()).expect("config is serializable"));
    for value in [distortion.a, distortion.b, distortion.c, crop_factor] {
        hasher.update(value.to_bits().to_le_bytes());
    }
    if let Some(value) = tca {
        hasher.update(value.red_linear.to_bits().to_le_bytes());
        hasher.update(value.blue_linear.to_bits().to_le_bytes());
    }
    if let Some(value) = vignetting {
        hasher.update(value.k1.to_bits().to_le_bytes());
        hasher.update(value.k2.to_bits().to_le_bytes());
        hasher.update(value.k3.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensCorrectionPlanError {
    InvalidDimensions,
    DimensionTooLarge,
    InvalidCropFactor,
    CameraNotFound,
    LensNotFound,
    InvalidRoi,
    EmptyRoi,
    TransformFailed,
    ArithmeticOverflow,
}

impl fmt::Display for LensCorrectionPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidDimensions => "lens correction dimensions are invalid",
            Self::DimensionTooLarge => "lens correction dimensions exceed the supported limit",
            Self::InvalidCropFactor => "lens correction crop factor is invalid",
            Self::CameraNotFound => "lens correction camera is absent from the pinned snapshot",
            Self::LensNotFound => "lens correction lens is absent from the pinned snapshot",
            Self::InvalidRoi => "lens correction ROI is invalid",
            Self::EmptyRoi => "lens correction ROI enclosure is empty",
            Self::TransformFailed => "lens correction ROI transform did not converge",
            Self::ArithmeticOverflow => "lens correction geometry arithmetic overflowed",
        })
    }
}

impl std::error::Error for LensCorrectionPlanError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LensCorrectionCoordinateError {
    NonFinitePoint,
    NonFiniteResult,
    InverseDidNotConverge,
    InvalidChannel,
}

impl fmt::Display for LensCorrectionCoordinateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinitePoint => "lens correction point is non-finite",
            Self::NonFiniteResult => "lens correction transform is non-finite",
            Self::InverseDidNotConverge => "lens correction inverse did not converge",
            Self::InvalidChannel => "lens correction channel is invalid",
        })
    }
}

impl std::error::Error for LensCorrectionCoordinateError {}
