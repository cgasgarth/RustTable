use std::cmp::Ordering;

pub use rusttable_core::numerics::ToleranceClass;
use serde::{Deserialize, Serialize};

mod artifacts;
mod color;
use artifacts::make_artifacts;
pub use color::ciede2000;
use color::{converter_error, delta_e_2000};
mod types;
pub use types::{
    ArtifactKind, BlinkPlanes, DiffArtifact, DiffArtifactDescriptor, DiffArtifactPayload,
};
mod receipt;
pub use receipt::DiffReceipt;

pub const DIFF_SCHEMA_VERSION: u32 = 3;
pub const MAX_OUTLIERS: usize = 32;
pub const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
const MAX_NEIGHBORHOOD_RADIUS: u32 = 8;
const DEFAULT_UNPREMULTIPLY_EPSILON: f32 = 1.0e-8;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub enum CanonicalProfile {
    Srgb,
    DisplayP3,
    Rec2020,
}

pub type CanonicalProfileId = CanonicalProfile;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
pub struct ComparisonProfile {
    pub canonical: CanonicalProfile,
}

impl ComparisonProfile {
    #[must_use]
    pub const fn new(canonical: CanonicalProfile) -> Self {
        Self { canonical }
    }
}

impl Default for ComparisonProfile {
    fn default() -> Self {
        Self::new(CanonicalProfile::Srgb)
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ImageInput {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub stride: usize,
    pub alpha: AlphaMode,
    pub profile: CanonicalProfile,
    pub transfer: TransferFunction,
    #[serde(default = "default_unpremultiply_epsilon")]
    pub unpremultiply_epsilon: f32,
    pub pixels: Vec<f32>,
}

const fn default_unpremultiply_epsilon() -> f32 {
    DEFAULT_UNPREMULTIPLY_EPSILON
}

impl ImageInput {
    #[must_use]
    pub fn rgba(width: u32, height: u32, pixels: Vec<f32>) -> Self {
        let stride = usize::try_from(width)
            .ok()
            .and_then(|width| width.checked_mul(4))
            .unwrap_or(0);
        Self {
            width,
            height,
            channels: 4,
            stride,
            alpha: AlphaMode::Straight,
            profile: CanonicalProfile::Srgb,
            transfer: TransferFunction::Linear,
            unpremultiply_epsilon: DEFAULT_UNPREMULTIPLY_EPSILON,
            pixels,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct MatrixProfileConverter;

pub trait ProfileConverter {
    fn convert(
        &self,
        source: CanonicalProfile,
        target: CanonicalProfile,
        rgb: [f32; 3],
    ) -> Option<[f32; 3]>;
}

impl ProfileConverter for MatrixProfileConverter {
    fn convert(
        &self,
        source: CanonicalProfile,
        target: CanonicalProfile,
        rgb: [f32; 3],
    ) -> Option<[f32; 3]> {
        Some(color::profile_convert(source, target, rgb))
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct ImageBuffer {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub alpha: AlphaMode,
    pub profile: CanonicalProfile,
    pub pixels: Vec<f32>,
}

impl ImageBuffer {
    #[must_use]
    pub fn rgba(width: u32, height: u32, pixels: Vec<f32>) -> Self {
        Self::canonical_rgba(width, height, CanonicalProfile::Srgb, pixels)
    }

    #[must_use]
    pub fn canonical_rgba(
        width: u32,
        height: u32,
        profile: CanonicalProfile,
        pixels: Vec<f32>,
    ) -> Self {
        Self {
            width,
            height,
            channels: 4,
            alpha: AlphaMode::Straight,
            profile,
            pixels,
        }
    }

    fn pixel_count(&self) -> Result<usize, DiffError> {
        checked_pixel_count(self.width, self.height)
    }

    fn expected_len(&self) -> Result<usize, DiffError> {
        self.pixel_count()?
            .checked_mul(usize::from(self.channels))
            .ok_or_else(|| DiffError::InvalidImage("image dimensions overflow".to_owned()))
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

/// Converts one explicitly described input into a tightly packed canonical buffer.
///
/// RGB transfer decoding and premultiplied unassociation happen here, never in
/// `compare`. Alpha remains a linear scalar and zero-alpha RGB is normalized to
/// zero so hidden premultiplied RGB cannot affect a comparison.
///
/// # Errors
///
/// Returns a typed error for invalid dimensions, layout, channel count, or an
/// undeclared profile conversion.
#[allow(clippy::too_many_lines)]
pub fn normalize(
    input: &ImageInput,
    target: CanonicalProfile,
    converter: Option<&dyn ProfileConverter>,
) -> Result<ImageBuffer, DiffError> {
    if input.width == 0 || input.height == 0 {
        return Err(DiffError::InvalidImage(
            "image dimensions must be non-zero".to_owned(),
        ));
    }
    if input.channels != 4 {
        return Err(DiffError::ChannelMismatch {
            source: input.channels,
            reference: 4,
        });
    }
    if !input.unpremultiply_epsilon.is_finite()
        || input.unpremultiply_epsilon <= 0.0
        || input.unpremultiply_epsilon > 1.0
    {
        return Err(DiffError::InvalidImage(
            "unpremultiply epsilon must be finite, positive, and at most one".to_owned(),
        ));
    }
    if let TransferFunction::Gamma(gamma) = input.transfer
        && (!gamma.is_finite() || gamma <= 0.0 || gamma > 32.0)
    {
        return Err(DiffError::InvalidTransfer(
            "gamma must be finite, positive, and at most 32".to_owned(),
        ));
    }
    let row_len = usize::try_from(input.width)
        .ok()
        .and_then(|width| width.checked_mul(usize::from(input.channels)))
        .ok_or_else(|| DiffError::InvalidImage("image dimensions overflow".to_owned()))?;
    if input.stride < row_len {
        return Err(DiffError::InvalidImage(
            "stride is shorter than one packed row".to_owned(),
        ));
    }
    let input_len = input
        .stride
        .checked_mul(usize::try_from(input.height).unwrap_or(usize::MAX))
        .ok_or_else(|| DiffError::InvalidImage("image dimensions overflow".to_owned()))?;
    if input.pixels.len() != input_len {
        return Err(DiffError::InvalidImage(
            "pixel buffer length does not match stride and dimensions".to_owned(),
        ));
    }
    if input.profile != target && converter.is_none() {
        return Err(DiffError::ProfileMismatch {
            source: input.profile,
            reference: target,
        });
    }
    let pixel_count = checked_pixel_count(input.width, input.height)?;
    let mut pixels = Vec::with_capacity(pixel_count * 4);
    for row in 0..usize::try_from(input.height).unwrap_or(0) {
        let row_start = row * input.stride;
        for column in 0..usize::try_from(input.width).unwrap_or(0) {
            let offset = row_start + column * 4;
            let alpha = match input.alpha {
                AlphaMode::Opaque => 1.0,
                _ => input.pixels[offset + 3],
            };
            if !alpha.is_finite() || !(0.0..=1.0).contains(&alpha) {
                return Err(DiffError::NonFiniteInput(
                    "alpha must be finite and within [0, 1]".to_owned(),
                ));
            }
            let mut rgb = [
                input.pixels[offset],
                input.pixels[offset + 1],
                input.pixels[offset + 2],
            ];
            if rgb.iter().any(|value| !value.is_finite()) {
                return Err(DiffError::NonFiniteInput(
                    "RGB samples must be finite".to_owned(),
                ));
            }
            if input.alpha == AlphaMode::Premultiplied {
                if alpha <= input.unpremultiply_epsilon {
                    rgb = [0.0; 3];
                } else {
                    for channel in &mut rgb {
                        *channel /= alpha;
                    }
                }
            }
            for channel in &mut rgb {
                *channel = decode_transfer(*channel, input.transfer);
            }
            if input.profile != target {
                rgb = converter
                    .and_then(|converter| converter.convert(input.profile, target, rgb))
                    .ok_or_else(converter_error)?;
            }
            if rgb.iter().any(|value| !value.is_finite()) {
                return Err(DiffError::NonFiniteInput(
                    "normalized RGB samples must be finite".to_owned(),
                ));
            }
            pixels.extend_from_slice(&rgb);
            pixels.push(alpha);
        }
    }
    Ok(ImageBuffer::canonical_rgba(
        input.width,
        input.height,
        target,
        pixels,
    ))
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
    pub psnr_peak: Option<f32>,
    pub allow_matching_infinities: bool,
    pub include_heatmap: bool,
    pub include_blink: bool,
    pub artifact_budget_bytes: usize,
}

impl DiffPolicy {
    #[must_use]
    pub fn for_class(class: ToleranceClass) -> Self {
        let (absolute, relative, rmse, delta_e, radius, max_outliers) = match class {
            ToleranceClass::Exact => (0.0, 0.0, 0.0, 0.0, 0, 0),
            ToleranceClass::Transfer => (2.0e-5, 2.0e-4, 2.0e-5, 0.02, 0, 0),
            ToleranceClass::Pointwise => (1.0e-3, 1.0e-2, 1.0e-3, 1.0, 0, 0),
            ToleranceClass::Neighborhood => (2.0e-3, 2.0e-2, 2.0e-3, 2.0, 1, 0),
            ToleranceClass::LegacyGpu => (8.0e-3, 8.0e-2, 8.0e-3, 4.0, 1, MAX_OUTLIERS),
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
            max_outliers,
            neighborhood_radius: radius,
            psnr_peak: Some(1.0),
            allow_matching_infinities: false,
            include_heatmap: false,
            include_blink: false,
            artifact_budget_bytes: MAX_ARTIFACT_BYTES,
        }
    }

    /// Validates thresholds and all report/artifact bounds before traversal.
    ///
    /// # Errors
    ///
    /// Returns an error when the schema, thresholds, PSNR peak, or bounds are invalid.
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
        if self
            .psnr_peak
            .is_some_and(|peak| !peak.is_finite() || peak <= 0.0)
        {
            return Err(DiffError::InvalidPolicy(
                "PSNR peak must be finite and positive".to_owned(),
            ));
        }
        if self.max_outliers > MAX_OUTLIERS || self.neighborhood_radius > MAX_NEIGHBORHOOD_RADIUS {
            return Err(DiffError::InvalidPolicy(
                "report bound exceeds policy limit".to_owned(),
            ));
        }
        if self.artifact_budget_bytes == 0 || self.artifact_budget_bytes > MAX_ARTIFACT_BYTES {
            return Err(DiffError::InvalidPolicy(
                "artifact budget exceeds the policy limit".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DiffMetrics {
    pub maximum_absolute_error: f32,
    pub maximum_rgb_absolute_error: f32,
    pub maximum_alpha_absolute_error: f32,
    pub weighted_maximum_absolute_error: f32,
    pub maximum_relative_error: f32,
    pub rgb_rmse: f32,
    pub alpha_rmse: f32,
    pub rmse: f32,
    pub psnr: Option<f32>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffError {
    InvalidPolicy(String),
    InvalidImage(String),
    InvalidTransfer(String),
    NonFiniteInput(String),
    DimensionMismatch {
        source: (u32, u32),
        reference: (u32, u32),
    },
    ChannelMismatch {
        source: u8,
        reference: u8,
    },
    AlphaMismatch,
    ProfileMismatch {
        source: CanonicalProfile,
        reference: CanonicalProfile,
    },
    NonCanonicalImage,
    Artifact(String),
    InvalidReceipt(String),
    Serialization(String),
}

impl std::fmt::Display for DiffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolicy(message) => write!(formatter, "invalid diff policy: {message}"),
            Self::InvalidImage(message) => write!(formatter, "invalid image: {message}"),
            Self::InvalidTransfer(message) => write!(formatter, "invalid transfer: {message}"),
            Self::NonFiniteInput(message) => write!(formatter, "non-finite input: {message}"),
            Self::DimensionMismatch { source, reference } => {
                write!(formatter, "dimension mismatch: {source:?} vs {reference:?}")
            }
            Self::ChannelMismatch { source, reference } => {
                write!(formatter, "channel mismatch: {source} vs {reference}")
            }
            Self::AlphaMismatch => write!(formatter, "alpha mode mismatch"),
            Self::ProfileMismatch { source, reference } => {
                write!(
                    formatter,
                    "canonical profile mismatch: {source:?} vs {reference:?}"
                )
            }
            Self::NonCanonicalImage => {
                write!(formatter, "comparison requires straight canonical RGBA")
            }
            Self::Artifact(message) => write!(formatter, "invalid diff artifact: {message}"),
            Self::InvalidReceipt(message) => write!(formatter, "invalid diff receipt: {message}"),
            Self::Serialization(message) => {
                write!(formatter, "receipt serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for DiffError {}

/// Compares two tightly packed, straight-alpha, linear canonical RGBA images.
/// Input normalization and profile conversion are intentionally separate.
///
/// # Errors
///
/// Returns an error for malformed images, mismatched profiles, or invalid policies.
pub fn compare(
    source: &ImageBuffer,
    reference: &ImageBuffer,
    policy: &DiffPolicy,
) -> Result<DiffReceipt, DiffError> {
    policy.validate()?;
    validate_images(source, reference)?;
    let pixel_count = source.pixel_count()?;
    let mut metrics = MetricsAccumulator::new(policy.alpha_weight);
    let mut retained = RetainedOutliers::default();
    let mut severities = if policy.include_heatmap {
        vec![0.0; pixel_count]
    } else {
        Vec::new()
    };
    for index in 0..pixel_count {
        let observation = if policy.neighborhood_radius > 0 {
            symmetric_neighborhood_match(index, source, reference, policy)
        } else {
            let source_pixel = source.pixel(index);
            let reference_pixel = reference.pixel(index);
            observe_pair(source_pixel, reference_pixel, source.profile, policy)
        };
        metrics.observe(&observation);
        if let Some(severity) = severities.get_mut(index) {
            *severity = observation.severity;
        }
        let (x, y) = coordinates(index, source.width);
        if observation.severity > 1.0 || observation.invalid {
            retained.push(
                observation.severity,
                DiffOutlier {
                    x,
                    y,
                    source: source.pixel(index),
                    reference: reference.pixel(index),
                    channel_error: observation.errors,
                    delta_e: observation.delta_e,
                    tile_identity: format!("pixel:{x}:{y}"),
                },
            );
            metrics.outlier_count += 1;
        }
    }
    metrics.finish(pixel_count, policy.psnr_peak);
    let metrics = metrics.into_metrics();
    let artifact_payloads = make_artifacts(source, reference, policy, &severities)?;
    let artifacts = artifact_payloads
        .iter()
        .map(DiffArtifactDescriptor::from_payload)
        .collect();
    let passed = metrics.maximum_absolute_error <= policy.max_absolute_error
        && metrics.maximum_relative_error <= policy.max_relative_error
        && metrics.rmse <= policy.max_rmse
        && metrics.maximum_delta_e.is_finite()
        && metrics.maximum_delta_e <= policy.max_delta_e
        && metrics.outlier_count <= policy.max_outliers
        && metrics.nonfinite_mismatch_count == 0;
    let receipt = DiffReceipt {
        schema_version: DIFF_SCHEMA_VERSION,
        policy: policy.clone(),
        metrics,
        outliers: retained.finish(),
        artifacts,
        artifact_payloads,
        passed,
    };
    receipt.validate()?;
    Ok(receipt)
}

fn validate_images(source: &ImageBuffer, reference: &ImageBuffer) -> Result<(), DiffError> {
    if (source.width, source.height) != (reference.width, reference.height) {
        return Err(DiffError::DimensionMismatch {
            source: (source.width, source.height),
            reference: (reference.width, reference.height),
        });
    }
    if source.width == 0 || source.height == 0 {
        return Err(DiffError::InvalidImage(
            "image dimensions must be non-zero".to_owned(),
        ));
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
    if source.alpha != AlphaMode::Straight || reference.alpha != AlphaMode::Straight {
        return Err(DiffError::NonCanonicalImage);
    }
    if source.profile != reference.profile {
        return Err(DiffError::ProfileMismatch {
            source: source.profile,
            reference: reference.profile,
        });
    }
    let expected = source.expected_len()?;
    if source.pixels.len() != expected || reference.pixels.len() != expected {
        return Err(DiffError::InvalidImage(
            "pixel buffer length does not match dimensions".to_owned(),
        ));
    }
    Ok(())
}

struct RetainedOutliers {
    values: Vec<ScoredOutlier>,
}

impl Default for RetainedOutliers {
    fn default() -> Self {
        Self {
            values: Vec::with_capacity(MAX_OUTLIERS),
        }
    }
}

struct ScoredOutlier {
    severity: f32,
    value: DiffOutlier,
}

impl RetainedOutliers {
    fn push(&mut self, severity: f32, value: DiffOutlier) {
        let candidate = ScoredOutlier { severity, value };
        if self.values.len() < MAX_OUTLIERS {
            self.values.push(candidate);
            return;
        }
        let Some(worst) = self
            .values
            .iter()
            .enumerate()
            .min_by(|(_, left), (_, right)| compare_outlier(left, right))
            .map(|(index, _)| index)
        else {
            return;
        };
        if compare_outlier(&candidate, &self.values[worst]) == Ordering::Greater {
            self.values[worst] = candidate;
        }
    }

    fn finish(mut self) -> Vec<DiffOutlier> {
        self.values
            .sort_by(|left, right| compare_outlier(right, left));
        self.values.into_iter().map(|entry| entry.value).collect()
    }
}

fn compare_outlier(left: &ScoredOutlier, right: &ScoredOutlier) -> Ordering {
    left.severity
        .total_cmp(&right.severity)
        .then_with(|| right.value.y.cmp(&left.value.y))
        .then_with(|| right.value.x.cmp(&left.value.x))
}

#[derive(Default)]
struct MetricsAccumulator {
    maximum_absolute_error: f32,
    maximum_rgb_absolute_error: f32,
    maximum_alpha_absolute_error: f32,
    maximum_relative_error: f32,
    rgb_squared_error: f64,
    alpha_squared_error: f64,
    squared_error: f64,
    pixel_count: usize,
    maximum_delta_e: f32,
    changed_pixel_count: usize,
    outlier_count: usize,
    nonfinite_mismatch_count: usize,
    alpha_weight: f32,
    psnr: Option<f32>,
}

impl MetricsAccumulator {
    fn new(alpha_weight: f32) -> Self {
        Self {
            alpha_weight,
            ..Self::default()
        }
    }

    fn observe(&mut self, observation: &Observation) {
        let maximum = observation.errors.iter().copied().fold(0.0, f32::max);
        let rgb_maximum = observation.raw_errors[..3]
            .iter()
            .copied()
            .fold(0.0, f32::max);
        self.maximum_absolute_error = self.maximum_absolute_error.max(maximum);
        self.maximum_rgb_absolute_error = self.maximum_rgb_absolute_error.max(rgb_maximum);
        self.maximum_alpha_absolute_error = self
            .maximum_alpha_absolute_error
            .max(observation.raw_errors[3]);
        self.maximum_delta_e = self.maximum_delta_e.max(observation.delta_e);
        self.maximum_relative_error = self.maximum_relative_error.max(observation.relative_error);
        if observation.invalid || maximum > 0.0 {
            self.changed_pixel_count += 1;
        }
        if observation.invalid {
            self.nonfinite_mismatch_count += 1;
        }
        for error in &observation.raw_errors[..3] {
            self.rgb_squared_error += f64::from(*error) * f64::from(*error);
        }
        self.alpha_squared_error +=
            f64::from(observation.raw_errors[3]) * f64::from(observation.raw_errors[3]);
        for error in &observation.errors {
            self.squared_error += f64::from(*error) * f64::from(*error);
        }
    }

    #[allow(clippy::cast_precision_loss)]
    #[allow(clippy::cast_possible_truncation)]
    fn finish(&mut self, pixels: usize, peak: Option<f32>) {
        self.pixel_count = pixels;
        let channel_weight = 3.0 + self.alpha_weight * self.alpha_weight;
        self.squared_error /= pixels.max(1) as f64 * f64::from(channel_weight);
        self.psnr = peak.and_then(|peak| {
            let rmse = self.squared_error.sqrt() as f32;
            (rmse > 0.0)
                .then(|| 20.0 * (peak / rmse).log10())
                .filter(|psnr| psnr.is_finite())
        });
    }

    #[allow(clippy::cast_possible_truncation)]
    #[allow(clippy::cast_precision_loss)]
    fn into_metrics(self) -> DiffMetrics {
        let pixels = self.pixel_count.max(1) as f64;
        let rgb_rmse = (self.rgb_squared_error / (pixels * 3.0)).sqrt() as f32;
        let alpha_rmse = (self.alpha_squared_error / pixels).sqrt() as f32;
        DiffMetrics {
            maximum_absolute_error: self.maximum_absolute_error,
            maximum_rgb_absolute_error: self.maximum_rgb_absolute_error,
            maximum_alpha_absolute_error: self.maximum_alpha_absolute_error,
            weighted_maximum_absolute_error: self.maximum_absolute_error,
            maximum_relative_error: self.maximum_relative_error,
            rgb_rmse,
            alpha_rmse,
            rmse: self.squared_error.sqrt() as f32,
            psnr: self.psnr,
            maximum_delta_e: self.maximum_delta_e,
            changed_pixel_count: self.changed_pixel_count,
            outlier_count: self.outlier_count,
            nonfinite_mismatch_count: self.nonfinite_mismatch_count,
        }
    }
}

#[derive(Clone, Copy)]
struct Observation {
    errors: [f32; 4],
    raw_errors: [f32; 4],
    relative_error: f32,
    delta_e: f32,
    invalid: bool,
    severity: f32,
}

fn symmetric_neighborhood_match(
    index: usize,
    source: &ImageBuffer,
    reference: &ImageBuffer,
    policy: &DiffPolicy,
) -> Observation {
    let (x, y) = coordinates(index, source.width);
    let source_to_reference =
        best_neighborhood_match(source.pixel(index), x, y, reference, source.profile, policy);
    let reference_to_source =
        best_neighborhood_match(reference.pixel(index), x, y, source, source.profile, policy);
    Observation {
        errors: std::array::from_fn(|channel| {
            source_to_reference.errors[channel].max(reference_to_source.errors[channel])
        }),
        raw_errors: std::array::from_fn(|channel| {
            source_to_reference.raw_errors[channel].max(reference_to_source.raw_errors[channel])
        }),
        relative_error: source_to_reference
            .relative_error
            .max(reference_to_source.relative_error),
        delta_e: source_to_reference.delta_e.max(reference_to_source.delta_e),
        invalid: source_to_reference.invalid || reference_to_source.invalid,
        severity: source_to_reference
            .severity
            .max(reference_to_source.severity),
    }
}

fn best_neighborhood_match(
    pixel: [f32; 4],
    x: u32,
    y: u32,
    target: &ImageBuffer,
    profile: CanonicalProfile,
    policy: &DiffPolicy,
) -> Observation {
    let radius = usize::try_from(policy.neighborhood_radius).unwrap_or(0);
    let width = usize::try_from(target.width).unwrap_or(0);
    let height = usize::try_from(target.height).unwrap_or(0);
    let x = usize::try_from(x).unwrap_or(0);
    let y = usize::try_from(y).unwrap_or(0);
    let mut best = None;
    for neighbor_y in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
        for neighbor_x in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
            let candidate = observe_pair(
                pixel,
                target.pixel(neighbor_y * width + neighbor_x),
                profile,
                policy,
            );
            if best
                .as_ref()
                .is_none_or(|best: &Observation| candidate.severity < best.severity)
            {
                best = Some(candidate);
            }
        }
    }
    best.unwrap_or_else(|| observe_pair(pixel, target.pixel(0), profile, policy))
}

fn observe_pair(
    source: [f32; 4],
    reference: [f32; 4],
    profile: CanonicalProfile,
    policy: &DiffPolicy,
) -> Observation {
    let mut raw_errors = [0.0; 4];
    let mut errors = [0.0; 4];
    let mut invalid = false;
    for channel in 0..4 {
        let (error, valid) = scalar_error(source[channel], reference[channel], policy);
        raw_errors[channel] = error;
        errors[channel] = if channel == 3 {
            error * policy.alpha_weight
        } else {
            error
        };
        invalid |= !valid;
    }
    let relative_error = (0..4)
        .map(|channel| {
            let (error, valid) = scalar_error(source[channel], reference[channel], policy);
            if valid {
                error
                    * if channel == 3 {
                        policy.alpha_weight
                    } else {
                        1.0
                    }
                    / source[channel]
                        .abs()
                        .max(reference[channel].abs())
                        .max(policy.epsilon)
            } else {
                f32::MAX
            }
        })
        .fold(0.0, f32::max);
    let delta_e = if !invalid && source[..3].iter().any(|value| !value.is_finite()) {
        0.0
    } else {
        delta_e_2000(source, reference, profile)
    };
    let severity = if invalid {
        f32::INFINITY
    } else {
        [
            threshold_ratio(
                errors.iter().copied().fold(0.0, f32::max),
                policy.max_absolute_error,
            ),
            threshold_ratio(relative_error, policy.max_relative_error),
            threshold_ratio(delta_e, policy.max_delta_e),
        ]
        .into_iter()
        .fold(0.0, f32::max)
    };
    Observation {
        errors,
        raw_errors,
        relative_error,
        delta_e,
        invalid,
        severity,
    }
}

fn scalar_error(left: f32, right: f32, policy: &DiffPolicy) -> (f32, bool) {
    if left.is_finite() && right.is_finite() {
        ((left - right).abs(), true)
    } else if policy.allow_matching_infinities
        && left.is_infinite()
        && right.is_infinite()
        && left.is_sign_positive() == right.is_sign_positive()
    {
        (0.0, true)
    } else {
        (f32::MAX, false)
    }
}

fn threshold_ratio(value: f32, threshold: f32) -> f32 {
    if value == 0.0 {
        0.0
    } else if threshold == 0.0 {
        f32::INFINITY
    } else {
        value / threshold
    }
}

fn checked_pixel_count(width: u32, height: u32) -> Result<usize, DiffError> {
    usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .ok_or_else(|| DiffError::InvalidImage("image dimensions overflow".to_owned()))
}

fn coordinates(index: usize, width: u32) -> (u32, u32) {
    let width = usize::try_from(width).unwrap_or(1);
    (
        u32::try_from(index % width).unwrap_or(0),
        u32::try_from(index / width).unwrap_or(0),
    )
}

fn decode_transfer(value: f32, transfer: TransferFunction) -> f32 {
    match transfer {
        TransferFunction::Linear => value,
        TransferFunction::Srgb => srgb_to_linear(value),
        TransferFunction::Gamma(gamma) if gamma.is_finite() && gamma > 0.0 => {
            value.signum() * value.abs().powf(gamma)
        }
        TransferFunction::Gamma(_) => f32::NAN,
    }
}

fn srgb_to_linear(value: f32) -> f32 {
    if value.abs() <= 0.04045 {
        value / 12.92
    } else {
        value.signum() * (((value.abs() + 0.055) / 1.055).powf(2.4))
    }
}
