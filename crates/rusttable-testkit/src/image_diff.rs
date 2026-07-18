use serde::{Deserialize, Serialize};

pub const DIFF_SCHEMA_VERSION: u32 = 1;
const MAX_OUTLIERS: usize = 32;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ToleranceClass {
    Exact,
    Transfer,
    Pointwise,
    Neighborhood,
    LegacyGpu,
}

impl ToleranceClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "Exact",
            Self::Transfer => "Transfer",
            Self::Pointwise => "Pointwise",
            Self::Neighborhood => "Neighborhood",
            Self::LegacyGpu => "LegacyGpu",
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
pub enum TransferFunction {
    Linear,
    Srgb,
    Gamma(f32),
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum AlphaMode {
    Straight,
    Premultiplied,
    Opaque,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct ComparisonProfile {
    pub transfer: TransferFunctionName,
    pub d50_lab: bool,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum TransferFunctionName {
    Linear,
    Srgb,
}

impl Default for ComparisonProfile {
    fn default() -> Self {
        Self {
            transfer: TransferFunctionName::Srgb,
            d50_lab: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffPolicy {
    pub schema_version: u32,
    pub class: ToleranceClass,
    pub epsilon: f32,
    pub alpha_weight: f32,
    pub max_absolute_error: f32,
    pub max_relative_error: f32,
    pub max_rmse: f32,
    pub max_delta_e: f32,
    pub max_outliers: usize,
    pub neighborhood_radius: u32,
    pub include_heatmap: bool,
    pub include_blink: bool,
}

impl DiffPolicy {
    #[must_use]
    pub fn for_class(class: ToleranceClass) -> Self {
        let (absolute, relative, rmse, delta_e, outliers, radius) = match class {
            ToleranceClass::Exact => (0.0, 0.0, 0.0, 0.0, 0, 0),
            ToleranceClass::Transfer => (2.0e-5, 2.0e-4, 2.0e-5, 0.02, 0, 0),
            ToleranceClass::Pointwise => (1.0e-3, 1.0e-2, 1.0e-3, 1.0, 0, 0),
            ToleranceClass::Neighborhood => (2.0e-3, 2.0e-2, 2.0e-3, 2.0, 0, 1),
            ToleranceClass::LegacyGpu => (8.0e-3, 8.0e-2, 8.0e-3, 4.0, 64, 1),
        };
        Self {
            schema_version: DIFF_SCHEMA_VERSION,
            class,
            epsilon: 1.0e-6,
            alpha_weight: 1.0,
            max_absolute_error: absolute,
            max_relative_error: relative,
            max_rmse: rmse,
            max_delta_e: delta_e,
            max_outliers: outliers,
            neighborhood_radius: radius,
            include_heatmap: false,
            include_blink: false,
        }
    }

    /// Validates a policy before it is used for a comparison.
    ///
    /// # Errors
    ///
    /// Returns an error when a policy contains a non-finite value or exceeds
    /// the bounded report/artifact limits.
    pub fn validate(&self) -> Result<(), DiffError> {
        if self.schema_version != DIFF_SCHEMA_VERSION {
            return Err(DiffError::InvalidPolicy(
                "unsupported schema version".to_owned(),
            ));
        }
        let values = [
            self.epsilon,
            self.alpha_weight,
            self.max_absolute_error,
            self.max_relative_error,
            self.max_rmse,
            self.max_delta_e,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err(DiffError::InvalidPolicy(
                "thresholds must be finite and nonnegative".to_owned(),
            ));
        }
        if self.max_outliers > MAX_OUTLIERS || self.neighborhood_radius > 8 {
            return Err(DiffError::InvalidPolicy(
                "report bound exceeds policy limit".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ImageBuffer {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub alpha: AlphaMode,
    pub profile: ComparisonProfile,
    pub pixels: Vec<f32>,
}

impl ImageBuffer {
    #[must_use]
    pub fn rgba(width: u32, height: u32, pixels: Vec<f32>) -> Self {
        Self {
            width,
            height,
            channels: 4,
            alpha: AlphaMode::Straight,
            profile: ComparisonProfile::default(),
            pixels,
        }
    }

    fn expected_len(&self) -> Result<usize, DiffError> {
        usize::try_from(self.width)
            .ok()
            .and_then(|width| {
                usize::try_from(self.height)
                    .ok()
                    .map(|height| width * height)
            })
            .and_then(|pixels| pixels.checked_mul(usize::from(self.channels)))
            .ok_or(DiffError::InvalidImage(
                "image dimensions overflow".to_owned(),
            ))
    }

    fn pixel(&self, index: usize) -> [f32; 4] {
        let offset = index * 4;
        [
            self.pixels[offset],
            self.pixels[offset + 1],
            self.pixels[offset + 2],
            self.pixels[offset + 3],
        ]
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffMetrics {
    pub maximum_absolute_error: f32,
    pub maximum_relative_error: f32,
    pub rmse: f32,
    pub psnr: f32,
    pub maximum_delta_e: f32,
    pub changed_pixel_count: usize,
    pub outlier_count: usize,
    pub nonfinite_mismatch_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffOutlier {
    pub x: u32,
    pub y: u32,
    pub source: [f32; 4],
    pub reference: [f32; 4],
    pub channel_error: [f32; 4],
    pub delta_e: f32,
    pub tile_identity: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffArtifact {
    pub kind: ArtifactKind,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum ArtifactKind {
    HeatmapRgba8,
    BlinkRgba32,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffReceipt {
    pub schema_version: u32,
    pub policy: DiffPolicy,
    pub metrics: DiffMetrics,
    pub outliers: Vec<DiffOutlier>,
    pub artifacts: Vec<DiffArtifact>,
    pub passed: bool,
}

impl DiffReceipt {
    /// Serializes a receipt with stable field ordering and no image dump.
    ///
    /// # Errors
    ///
    /// Returns an error when JSON serialization cannot represent a receipt.
    pub fn stable_json(&self) -> Result<String, DiffError> {
        serde_json::to_string(self).map_err(|error| DiffError::Serialization(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    InvalidPolicy(String),
    InvalidImage(String),
    DimensionMismatch {
        source: (u32, u32),
        reference: (u32, u32),
    },
    ChannelMismatch {
        source: u8,
        reference: u8,
    },
    AlphaMismatch,
    Serialization(String),
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolicy(message) => write!(formatter, "invalid diff policy: {message}"),
            Self::InvalidImage(message) => write!(formatter, "invalid image: {message}"),
            Self::DimensionMismatch { source, reference } => {
                write!(formatter, "dimension mismatch: {source:?} vs {reference:?}")
            }
            Self::ChannelMismatch { source, reference } => {
                write!(formatter, "channel mismatch: {source} vs {reference}")
            }
            Self::AlphaMismatch => write!(formatter, "alpha mode mismatch"),
            Self::Serialization(message) => {
                write!(formatter, "receipt serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for DiffError {}

/// Compares two decoded RGBA images in the canonical comparison profile.
///
/// # Errors
///
/// Returns an error for malformed buffers or incompatible dimensions, channels,
/// and alpha modes. A metric mismatch is represented by `DiffReceipt::passed`.
pub fn compare(
    source: &ImageBuffer,
    reference: &ImageBuffer,
    policy: &DiffPolicy,
) -> Result<DiffReceipt, DiffError> {
    policy.validate()?;
    if (source.width, source.height) != (reference.width, reference.height) {
        return Err(DiffError::DimensionMismatch {
            source: (source.width, source.height),
            reference: (reference.width, reference.height),
        });
    }
    if source.channels != reference.channels || source.channels != 4 {
        return Err(DiffError::ChannelMismatch {
            source: source.channels,
            reference: reference.channels,
        });
    }
    if source.alpha != reference.alpha {
        return Err(DiffError::AlphaMismatch);
    }
    let expected = source.expected_len()?;
    if source.pixels.len() != expected || reference.pixels.len() != expected {
        return Err(DiffError::InvalidImage(
            "pixel buffer length does not match dimensions".to_owned(),
        ));
    }
    let mut metrics = MetricsAccumulator::default();
    let mut outliers = Vec::new();
    for index in 0..(expected / 4) {
        let source_pixel = source.pixel(index);
        let reference_pixel = reference.pixel(index);
        let errors = channel_errors(source_pixel, reference_pixel, source.alpha, policy);
        let nonfinite = source_pixel
            .iter()
            .zip(reference_pixel)
            .any(|(left, right)| left.is_finite() != right.is_finite());
        let delta_e = delta_e_2000(source_pixel, reference_pixel, source.profile);
        metrics.observe(
            &errors,
            source_pixel,
            reference_pixel,
            delta_e,
            nonfinite,
            policy.epsilon,
        );
        let (x, y) = (
            u32::try_from(index % usize::try_from(source.width).unwrap_or(1)).unwrap_or(0),
            u32::try_from(index / usize::try_from(source.width).unwrap_or(1)).unwrap_or(0),
        );
        let outlier = is_outlier(index, source, &errors, delta_e, nonfinite, policy);
        if outlier {
            metrics.outlier_count += 1;
            outliers.push(DiffOutlier {
                x,
                y,
                source: source_pixel,
                reference: reference_pixel,
                channel_error: errors,
                delta_e,
                tile_identity: format!("tile:{x}:{y}"),
            });
        }
    }
    metrics.finish(expected / 4);
    let metrics: DiffMetrics = metrics.into();
    outliers.sort_by(|left, right| {
        right
            .channel_error
            .iter()
            .copied()
            .fold(0.0, f32::max)
            .total_cmp(&left.channel_error.iter().copied().fold(0.0, f32::max))
            .then_with(|| left.y.cmp(&right.y))
            .then_with(|| left.x.cmp(&right.x))
    });
    outliers.truncate(MAX_OUTLIERS);
    let artifacts = make_artifacts(source, reference, policy, &outliers);
    let passed = metrics.maximum_absolute_error <= policy.max_absolute_error
        && metrics.maximum_relative_error <= policy.max_relative_error
        && metrics.rmse <= policy.max_rmse
        && metrics.maximum_delta_e <= policy.max_delta_e
        && metrics.outlier_count <= policy.max_outliers
        && metrics.nonfinite_mismatch_count == 0;
    Ok(DiffReceipt {
        schema_version: DIFF_SCHEMA_VERSION,
        policy: policy.clone(),
        metrics,
        outliers,
        artifacts,
        passed,
    })
}

#[derive(Default)]
struct MetricsAccumulator {
    maximum_absolute_error: f32,
    maximum_relative_error: f32,
    squared_error: f64,
    maximum_delta_e: f32,
    changed_pixel_count: usize,
    outlier_count: usize,
    nonfinite_mismatch_count: usize,
}

impl MetricsAccumulator {
    fn observe(
        &mut self,
        errors: &[f32; 4],
        source: [f32; 4],
        reference: [f32; 4],
        delta_e: f32,
        nonfinite: bool,
        epsilon: f32,
    ) {
        let maximum = errors.iter().copied().fold(0.0, f32::max);
        self.maximum_absolute_error = self.maximum_absolute_error.max(maximum);
        self.maximum_delta_e = self.maximum_delta_e.max(delta_e);
        let relative = errors
            .iter()
            .zip(source.into_iter().zip(reference))
            .map(|(error, (left, right))| {
                if left.is_finite() && right.is_finite() {
                    *error / right.abs().max(left.abs()).max(epsilon)
                } else if left.is_finite() == right.is_finite() {
                    0.0
                } else {
                    f32::MAX
                }
            })
            .fold(0.0, f32::max);
        self.maximum_relative_error = self.maximum_relative_error.max(relative);
        if maximum > 0.0 {
            self.changed_pixel_count += 1;
        }
        if nonfinite {
            self.nonfinite_mismatch_count += 1;
        }
        for error in errors {
            self.squared_error += f64::from(*error) * f64::from(*error);
        }
    }

    #[allow(clippy::cast_precision_loss)]
    fn finish(&mut self, pixels: usize) {
        self.squared_error /= (pixels.max(1) * 4) as f64;
    }
}

impl From<MetricsAccumulator> for DiffMetrics {
    #[allow(clippy::cast_possible_truncation)]
    fn from(value: MetricsAccumulator) -> Self {
        let rmse = value.squared_error.sqrt() as f32;
        let psnr = if rmse == 0.0 {
            f32::INFINITY
        } else {
            20.0 * (1.0 / rmse).log10()
        };
        Self {
            maximum_absolute_error: value.maximum_absolute_error,
            maximum_relative_error: value.maximum_relative_error,
            rmse,
            psnr,
            maximum_delta_e: value.maximum_delta_e,
            changed_pixel_count: value.changed_pixel_count,
            outlier_count: value.outlier_count,
            nonfinite_mismatch_count: value.nonfinite_mismatch_count,
        }
    }
}

#[allow(clippy::float_cmp)]
fn channel_errors(
    source: [f32; 4],
    reference: [f32; 4],
    alpha: AlphaMode,
    policy: &DiffPolicy,
) -> [f32; 4] {
    let mut errors = [0.0; 4];
    for channel in 0..4 {
        let left = linear_sample(source[channel], source[3], alpha);
        let right = linear_sample(reference[channel], reference[3], alpha);
        errors[channel] = if left.is_finite() && right.is_finite() {
            (left - right).abs()
        } else if left.is_finite() == right.is_finite() && left == right {
            0.0
        } else {
            f32::MAX
        };
    }
    if (policy.alpha_weight - 1.0).abs() > f32::EPSILON {
        errors[3] *= policy.alpha_weight;
    }
    errors
}

fn linear_sample(value: f32, alpha: f32, mode: AlphaMode) -> f32 {
    let value = match mode {
        AlphaMode::Premultiplied if alpha > 0.0 => value / alpha,
        _ => value,
    };
    srgb_to_linear(value)
}

fn srgb_to_linear(value: f32) -> f32 {
    if value.abs() <= 0.04045 {
        value / 12.92
    } else {
        value.signum() * (((value.abs() + 0.055) / 1.055).powf(2.4))
    }
}

fn is_outlier(
    index: usize,
    image: &ImageBuffer,
    errors: &[f32; 4],
    delta_e: f32,
    nonfinite: bool,
    policy: &DiffPolicy,
) -> bool {
    if nonfinite {
        return true;
    }
    let maximum = errors.iter().copied().fold(0.0, f32::max);
    let exceeded = maximum > policy.max_absolute_error || delta_e > policy.max_delta_e;
    if !exceeded {
        return false;
    }
    if policy.class != ToleranceClass::Neighborhood || policy.neighborhood_radius == 0 {
        return true;
    }
    let width = usize::try_from(image.width).unwrap_or(1);
    let x = index % width;
    let y = index / width;
    let radius = usize::try_from(policy.neighborhood_radius).unwrap_or(0);
    for neighbor_y in y.saturating_sub(radius)
        ..=(y + radius).min(usize::try_from(image.height).unwrap_or(1).saturating_sub(1))
    {
        for neighbor_x in x.saturating_sub(radius)..=(x + radius).min(width.saturating_sub(1)) {
            if neighbor_x == x && neighbor_y == y {
                continue;
            }
            let neighbor = image.pixel(neighbor_y * width + neighbor_x);
            if neighbor.iter().all(|value| value.is_finite()) {
                return false;
            }
        }
    }
    true
}

fn delta_e_2000(left: [f32; 4], right: [f32; 4], profile: ComparisonProfile) -> f32 {
    let first = lab(left, profile);
    let second = lab(right, profile);
    let c1 = (first[1] * first[1] + first[2] * first[2]).sqrt();
    let c2 = (second[1] * second[1] + second[2] * second[2]).sqrt();
    let c_bar = f32::midpoint(c1, c2);
    let c_bar_7 = c_bar.powi(7);
    let twenty_five_7 = 25.0_f32.powi(7);
    let g = 0.5 * (1.0 - (c_bar_7 / (c_bar_7 + twenty_five_7)).sqrt());
    let a1 = (1.0 + g) * first[1];
    let a2 = (1.0 + g) * second[1];
    let c1 = (a1 * a1 + first[2] * first[2]).sqrt();
    let c2 = (a2 * a2 + second[2] * second[2]).sqrt();
    let h1 = hue_angle(a1, first[2]);
    let h2 = hue_angle(a2, second[2]);
    let delta_l = second[0] - first[0];
    let delta_c = c2 - c1;
    let delta_h = if c1 * c2 == 0.0 {
        0.0
    } else if (h2 - h1).abs() <= 180.0 {
        h2 - h1
    } else if h2 <= h1 {
        h2 - h1 + 360.0
    } else {
        h2 - h1 - 360.0
    };
    let delta_big_h = 2.0 * (c1 * c2).sqrt() * (delta_h.to_radians() / 2.0).sin();
    let l_bar = f32::midpoint(first[0], second[0]);
    let c_bar = f32::midpoint(c1, c2);
    let h_bar = if c1 * c2 == 0.0 {
        h1 + h2
    } else if (h1 - h2).abs() <= 180.0 {
        f32::midpoint(h1, h2)
    } else if h1 + h2 < 360.0 {
        (h1 + h2 + 360.0) / 2.0
    } else {
        (h1 + h2 - 360.0) / 2.0
    };
    let t = 1.0 - 0.17 * ((h_bar - 30.0).to_radians()).cos()
        + 0.24 * ((2.0 * h_bar).to_radians()).cos()
        + 0.32 * ((3.0 * h_bar + 6.0).to_radians()).cos()
        - 0.20 * ((4.0 * h_bar - 63.0).to_radians()).cos();
    let delta_theta = 30.0 * (-((h_bar - 275.0) / 25.0).powi(2)).exp();
    let rc = 2.0 * (c_bar.powi(7) / (c_bar.powi(7) + twenty_five_7)).sqrt();
    let sl = 1.0 + 0.015 * (l_bar - 50.0).powi(2) / (20.0 + (l_bar - 50.0).powi(2)).sqrt();
    let sc = 1.0 + 0.045 * c_bar;
    let sh = 1.0 + 0.015 * c_bar * t;
    let rt = -(2.0 * delta_theta.to_radians()).sin() * rc;
    ((delta_l / sl).powi(2)
        + (delta_c / sc).powi(2)
        + (delta_big_h / sh).powi(2)
        + rt * (delta_c / sc) * (delta_big_h / sh))
        .max(0.0)
        .sqrt()
}

fn hue_angle(a: f32, b: f32) -> f32 {
    let angle = b.atan2(a).to_degrees();
    if angle < 0.0 { angle + 360.0 } else { angle }
}

#[allow(clippy::excessive_precision)]
fn lab(pixel: [f32; 4], profile: ComparisonProfile) -> [f32; 3] {
    let red = transfer(pixel[0], profile.transfer);
    let green = transfer(pixel[1], profile.transfer);
    let blue = transfer(pixel[2], profile.transfer);
    let xyz_x = 0.412_456_4 * red + 0.357_576_1 * green + 0.180_437_5 * blue;
    let xyz_y = 0.212_672_9 * red + 0.715_152_2 * green + 0.072_175_0 * blue;
    let xyz_z = 0.019_333_9 * red + 0.119_192 * green + 0.950_304_1 * blue;
    let (x, y, z) = if profile.d50_lab {
        (
            1.047_929_8 * xyz_x + 0.022_946_8 * xyz_y - 0.050_192_2 * xyz_z,
            0.029_627_8 * xyz_x + 0.990_434_5 * xyz_y - 0.017_073_8 * xyz_z,
            -0.009_243_1 * xyz_x + 0.015_055_2 * xyz_y + 0.751_874_3 * xyz_z,
        )
    } else {
        (xyz_x, xyz_y, xyz_z)
    };
    let f = |value: f32| {
        if value > 0.008_856 {
            value.powf(1.0 / 3.0)
        } else {
            7.787 * value + 16.0 / 116.0
        }
    };
    let (xn, yn, zn) = (0.96422, 1.0, 0.82521);
    let (fx, fy, fz) = (f(x / xn), f(y / yn), f(z / zn));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn transfer(value: f32, transfer: TransferFunctionName) -> f32 {
    match transfer {
        TransferFunctionName::Linear => value,
        TransferFunctionName::Srgb => srgb_to_linear(value),
    }
}

fn make_artifacts(
    source: &ImageBuffer,
    reference: &ImageBuffer,
    policy: &DiffPolicy,
    outliers: &[DiffOutlier],
) -> Vec<DiffArtifact> {
    let mut artifacts = Vec::new();
    if policy.include_heatmap {
        let mut bytes = Vec::with_capacity(outliers.len() * 8);
        for outlier in outliers {
            bytes.extend_from_slice(&outlier.x.to_le_bytes());
            bytes.extend_from_slice(&outlier.y.to_le_bytes());
        }
        artifacts.push(DiffArtifact {
            kind: ArtifactKind::HeatmapRgba8,
            width: source.width,
            height: source.height,
            bytes,
        });
    }
    if policy.include_blink {
        let mut bytes = Vec::with_capacity((source.pixels.len() + reference.pixels.len()) * 4);
        for value in source.pixels.iter().chain(&reference.pixels) {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        artifacts.push(DiffArtifact {
            kind: ArtifactKind::BlinkRgba32,
            width: source.width,
            height: source.height,
            bytes,
        });
    }
    artifacts
}
