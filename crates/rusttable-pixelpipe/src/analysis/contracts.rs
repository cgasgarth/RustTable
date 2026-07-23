use std::{fmt, num::NonZeroU16};

use rusttable_color::ColorEncoding;
use rusttable_core::numerics::{
    ConversionPolicy, FloatDomainPolicy, FmaPolicy, NonFinitePolicy, NumericalContract,
    ReductionPolicy, SubnormalPolicy, TranscendentalPolicy,
};
use rusttable_image::{ImageDimensions, Roi};
use sha2::{Digest, Sha256};

use crate::{
    CpuPriority, HistogramMaskPolicy, HistogramNonFinitePolicy, HistogramRange, NodeBoundary,
    PublicationTargetKind, ResourceClaim, SchedulerPublicationTarget, TaskError,
};

/// Version of the canonical analysis request and provenance contract.
pub const ANALYSIS_SCHEMA_VERSION: u16 = 1;
/// Hard per-axis bound for a reusable analysis plane.
pub const MAX_ANALYSIS_DIMENSION: u32 = 4_096;
/// Hard resident-buffer bound for one analysis result.
pub const MAX_ANALYSIS_BYTES: u64 = 64 * 1_024 * 1_024;
/// Hard bound on indexed partials accepted by one merge. This caps merge bookkeeping even when a
/// hostile caller supplies a synthetic tile plan.
pub const MAX_ANALYSIS_TILES: usize = 65_536;
/// #286 numerical policy selected by the scalar reference generator. Per-pixel color transforms
/// remain `f32`; all cross-pixel reduction is exact checked `u64` addition.
pub const ANALYSIS_NUMERICAL_CONTRACT: NumericalContract = NumericalContract {
    float_domain: FloatDomainPolicy::F32,
    non_finite: NonFinitePolicy::Reject,
    subnormal: SubnormalPolicy::Preserve,
    fma: FmaPolicy::SeparateRoundings,
    reduction: ReductionPolicy::None,
    transcendental: TranscendentalPolicy::RustIntrinsic,
    conversion: ConversionPolicy::clamped_nearest_even(),
};

/// Analysis-facing name for the established cache/pipeline node boundary contract.
pub type AnalysisBoundary = NodeBoundary;
/// Analysis-facing name for the established color-management endpoint contract.
pub type AnalysisSourceColorSpace = ColorEncoding;

/// Analysis buffer family. RGB waveform and parade intentionally share numerical planes; their
/// different composition belongs to a future renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisKind {
    LuminanceWaveform,
    RgbWaveform,
    RgbParade,
    Vectorscope,
}

/// Direction of the image-position axis in a waveform. RGB parade uses the same numerical
/// orientation and differs only in future plane composition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WaveformOrientation {
    Horizontal,
    Vertical,
}

impl AnalysisKind {
    #[must_use]
    pub const fn plane_count(self) -> u8 {
        match self {
            Self::LuminanceWaveform | Self::Vectorscope => 1,
            Self::RgbWaveform | Self::RgbParade => 3,
        }
    }

    #[must_use]
    pub const fn channels(self) -> &'static [AnalysisChannel] {
        match self {
            Self::LuminanceWaveform => &[AnalysisChannel::Luminance],
            Self::RgbWaveform | Self::RgbParade => &[
                AnalysisChannel::Red,
                AnalysisChannel::Green,
                AnalysisChannel::Blue,
            ],
            Self::Vectorscope => &[AnalysisChannel::Chroma],
        }
    }
}

/// Stable semantic label for one result plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisChannel {
    Luminance,
    Red,
    Green,
    Blue,
    Chroma,
}

/// Coefficients and labels used by waveform/vectorscope graticules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisGraticule {
    Rec601,
    Rec709,
    Rec2020,
}

impl AnalysisGraticule {
    /// Returns `(Kr, Kb)` for deterministic luma/chroma projection.
    #[must_use]
    pub const fn luma_coefficients(self) -> (f32, f32) {
        match self {
            Self::Rec601 => (0.299, 0.114),
            Self::Rec709 => (0.2126, 0.0722),
            Self::Rec2020 => (0.2627, 0.0593),
        }
    }
}

/// Global-coordinate sampling. A tile never resets stride phase, so tile boundaries cannot alter
/// the selected pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisSampling {
    EveryPixel,
    Grid {
        x_step: NonZeroU16,
        y_step: NonZeroU16,
        x_phase: u16,
        y_phase: u16,
    },
}

impl AnalysisSampling {
    /// Creates checked grid sampling with phases in `[0, step)`.
    ///
    /// # Errors
    ///
    /// Rejects zero steps and phases outside their corresponding step.
    pub fn grid(
        x_step: u16,
        y_step: u16,
        x_phase: u16,
        y_phase: u16,
    ) -> Result<Self, AnalysisSamplingError> {
        let x_step = NonZeroU16::new(x_step).ok_or(AnalysisSamplingError::ZeroStep)?;
        let y_step = NonZeroU16::new(y_step).ok_or(AnalysisSamplingError::ZeroStep)?;
        if x_phase >= x_step.get() || y_phase >= y_step.get() {
            return Err(AnalysisSamplingError::PhaseOutsideStep);
        }
        Ok(Self::Grid {
            x_step,
            y_step,
            x_phase,
            y_phase,
        })
    }

    #[must_use]
    pub const fn includes(self, x: u32, y: u32) -> bool {
        match self {
            Self::EveryPixel => true,
            Self::Grid {
                x_step,
                y_step,
                x_phase,
                y_phase,
            } => {
                x % x_step.get() as u32 == x_phase as u32
                    && y % y_step.get() as u32 == y_phase as u32
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisSamplingError {
    ZeroStep,
    PhaseOutsideStep,
}

/// The denominator a renderer should use when mapping integer counts to intensity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisNormalization {
    None,
    Peak,
    SampleIntensity,
}

/// Fixed contribution precision. `FixedPoint { fractional_bits: 8 }` contributes 256 units for
/// an opaque sample; weighted alpha is rounded once with nearest-ties-even.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisIntensity {
    Count,
    FixedPoint { fractional_bits: u8 },
}

impl AnalysisIntensity {
    #[must_use]
    pub(crate) const fn quantum(self) -> u64 {
        match self {
            Self::Count => 1,
            Self::FixedPoint { fractional_bits } => 1_u64 << fractional_bits,
        }
    }
}

/// Alpha participation is independent of fixed accumulation precision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisAlphaPolicy {
    Ignore,
    ExcludeTransparent,
    Weight,
}

/// Checked dimensions for each analysis plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalysisOutputDimensions(ImageDimensions);

impl AnalysisOutputDimensions {
    /// Creates nonzero dimensions within the per-axis analysis limit.
    ///
    /// # Errors
    ///
    /// Rejects zero or over-limit axes.
    pub fn new(width: u32, height: u32) -> Result<Self, AnalysisRequestError> {
        let dimensions = ImageDimensions::new(width, height)
            .map_err(|_| AnalysisRequestError::InvalidDimensions)?;
        if width > MAX_ANALYSIS_DIMENSION || height > MAX_ANALYSIS_DIMENSION {
            return Err(AnalysisRequestError::DimensionsExceeded);
        }
        Ok(Self(dimensions))
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.0.width()
    }
    #[must_use]
    pub const fn height(self) -> u32 {
        self.0.height()
    }
    #[must_use]
    pub const fn image_dimensions(self) -> ImageDimensions {
        self.0
    }
    pub(crate) fn pixel_count(self) -> Result<u64, AnalysisRequestError> {
        self.0
            .pixel_count()
            .map_err(|_| AnalysisRequestError::SizeOverflow)
    }
}

impl TryFrom<ImageDimensions> for AnalysisOutputDimensions {
    type Error = AnalysisRequestError;

    fn try_from(value: ImageDimensions) -> Result<Self, Self::Error> {
        Self::new(value.width(), value.height())
    }
}

/// Stable request digest suitable for equality, provenance, and cache composition.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalysisRequestIdentity([u8; 32]);

impl AnalysisRequestIdentity {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

impl fmt::Debug for AnalysisRequestIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_digest(self.0, formatter)
    }
}

/// Complete cache component: request, exact source raster bits, and optional exact mask bits.
/// Pass [`Self::as_bytes`] to [`crate::CacheKeyBuilder::analysis_identity`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalysisCacheIdentity([u8; 32]);

impl AnalysisCacheIdentity {
    #[must_use]
    pub fn new(
        request: AnalysisRequestIdentity,
        source_raster: [u8; 32],
        mask: Option<[u8; 32]>,
        transform: [u8; 32],
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"rusttable.analysis.cache.v1");
        hasher.update(request.as_bytes());
        hasher.update(source_raster);
        match mask {
            Some(identity) => {
                hasher.update([1]);
                hasher.update(identity);
            }
            None => hasher.update([0]),
        }
        hasher.update(transform);
        Self(hasher.finalize().into())
    }

    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    #[must_use]
    pub const fn publication_target(self) -> SchedulerPublicationTarget {
        SchedulerPublicationTarget::new(PublicationTargetKind::Cache, self.0)
    }
}

impl fmt::Debug for AnalysisCacheIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write_digest(self.0, formatter)
    }
}

/// Immutable, output-affecting request. The histogram range retains #275's inclusive-low,
/// exclusive-high bin convention with out-of-range finite values clamped to edge bins.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalysisRequest {
    kind: AnalysisKind,
    boundary: NodeBoundary,
    roi: Roi,
    output: AnalysisOutputDimensions,
    source_color_space: ColorEncoding,
    analysis_color_space: ColorEncoding,
    sampling: AnalysisSampling,
    range: HistogramRange,
    mask_policy: HistogramMaskPolicy,
    nonfinite_policy: HistogramNonFinitePolicy,
    normalization: AnalysisNormalization,
    graticule: AnalysisGraticule,
    intensity: AnalysisIntensity,
    alpha_policy: AnalysisAlphaPolicy,
    waveform_orientation: WaveformOrientation,
    identity: AnalysisRequestIdentity,
}

impl AnalysisRequest {
    /// Creates a complete output-affecting request and computes its stable identity.
    ///
    /// # Errors
    ///
    /// Rejects unspecified color endpoints, excessive fixed precision, and output storage above
    /// the hard analysis memory limit.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: AnalysisKind,
        boundary: NodeBoundary,
        roi: Roi,
        output: AnalysisOutputDimensions,
        source_color_space: ColorEncoding,
        analysis_color_space: ColorEncoding,
        sampling: AnalysisSampling,
        range: HistogramRange,
        mask_policy: HistogramMaskPolicy,
        nonfinite_policy: HistogramNonFinitePolicy,
        normalization: AnalysisNormalization,
        graticule: AnalysisGraticule,
        intensity: AnalysisIntensity,
        alpha_policy: AnalysisAlphaPolicy,
        waveform_orientation: WaveformOrientation,
    ) -> Result<Self, AnalysisRequestError> {
        if !source_color_space.is_explicit() || !analysis_color_space.is_explicit() {
            return Err(AnalysisRequestError::UnspecifiedColorSpace);
        }
        if boundary.first() > boundary.last() {
            return Err(AnalysisRequestError::InvalidBoundary);
        }
        if matches!(intensity, AnalysisIntensity::FixedPoint { fractional_bits } if fractional_bits > 16)
        {
            return Err(AnalysisRequestError::IntensityPrecisionExceeded);
        }
        let bytes = output_bytes(kind, output)?;
        if bytes > MAX_ANALYSIS_BYTES {
            return Err(AnalysisRequestError::MemoryExceeded {
                requested: bytes,
                limit: MAX_ANALYSIS_BYTES,
            });
        }
        let mut request = Self {
            kind,
            boundary,
            roi,
            output,
            source_color_space,
            analysis_color_space,
            sampling,
            range,
            mask_policy,
            nonfinite_policy,
            normalization,
            graticule,
            intensity,
            alpha_policy,
            waveform_orientation,
            identity: AnalysisRequestIdentity([0; 32]),
        };
        request.identity =
            AnalysisRequestIdentity(Sha256::digest(request.canonical_bytes()).into());
        Ok(request)
    }

    #[must_use]
    pub const fn kind(&self) -> AnalysisKind {
        self.kind
    }
    #[must_use]
    pub const fn boundary(&self) -> &NodeBoundary {
        &self.boundary
    }
    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }
    #[must_use]
    pub const fn output(&self) -> AnalysisOutputDimensions {
        self.output
    }
    #[must_use]
    pub const fn source_color_space(&self) -> ColorEncoding {
        self.source_color_space
    }
    #[must_use]
    pub const fn analysis_color_space(&self) -> ColorEncoding {
        self.analysis_color_space
    }
    #[must_use]
    pub const fn sampling(&self) -> AnalysisSampling {
        self.sampling
    }
    #[must_use]
    pub const fn range(&self) -> HistogramRange {
        self.range
    }
    #[must_use]
    pub const fn mask_policy(&self) -> HistogramMaskPolicy {
        self.mask_policy
    }
    #[must_use]
    pub const fn nonfinite_policy(&self) -> HistogramNonFinitePolicy {
        self.nonfinite_policy
    }
    #[must_use]
    pub const fn normalization(&self) -> AnalysisNormalization {
        self.normalization
    }
    #[must_use]
    pub const fn graticule(&self) -> AnalysisGraticule {
        self.graticule
    }
    #[must_use]
    pub const fn intensity(&self) -> AnalysisIntensity {
        self.intensity
    }
    #[must_use]
    pub const fn alpha_policy(&self) -> AnalysisAlphaPolicy {
        self.alpha_policy
    }
    #[must_use]
    pub const fn waveform_orientation(&self) -> WaveformOrientation {
        self.waveform_orientation
    }
    #[must_use]
    pub const fn identity(&self) -> AnalysisRequestIdentity {
        self.identity
    }

    #[must_use]
    pub fn resident_bytes(&self) -> u64 {
        u64::from(self.output.width())
            * u64::from(self.output.height())
            * u64::from(self.kind.plane_count())
            * 8
    }

    /// Resource estimate for #273 admission. Memory covers the result plus one partial for every
    /// simultaneously active worker. Analysis is background work and does not claim an active
    /// pipeline slot or retain host-pool leases.
    ///
    /// # Errors
    ///
    /// Returns the scheduler's typed error when `worker_tokens` is zero.
    pub fn resource_claim(&self, worker_tokens: u16) -> Result<ResourceClaim, TaskError> {
        let memory_bytes = self
            .resident_bytes()
            .checked_mul(u64::from(worker_tokens) + 1)
            .ok_or(TaskError::ArithmeticOverflow)?;
        ResourceClaim::new(
            memory_bytes,
            worker_tokens,
            worker_tokens,
            false,
            Vec::new(),
        )
    }

    #[must_use]
    pub const fn scheduler_priority(&self) -> CpuPriority {
        CpuPriority::BackgroundAnalysis
    }

    /// Returns a re-identified request with a different alpha policy.
    #[must_use]
    pub fn with_alpha_policy(mut self, alpha_policy: AnalysisAlphaPolicy) -> Self {
        self.alpha_policy = alpha_policy;
        self.refresh_identity();
        self
    }

    /// Returns a re-identified request with a different non-finite policy.
    #[must_use]
    pub fn with_nonfinite_policy(mut self, policy: HistogramNonFinitePolicy) -> Self {
        self.nonfinite_policy = policy;
        self.refresh_identity();
        self
    }

    /// Returns a re-identified request with a different histogram-compatible mask policy.
    #[must_use]
    pub fn with_mask_policy(mut self, policy: HistogramMaskPolicy) -> Self {
        self.mask_policy = policy;
        self.refresh_identity();
        self
    }

    /// Returns a re-identified request with different global-coordinate sampling.
    #[must_use]
    pub fn with_sampling(mut self, sampling: AnalysisSampling) -> Self {
        self.sampling = sampling;
        self.refresh_identity();
        self
    }

    /// Returns a re-identified request with different source and analysis color endpoints.
    ///
    /// # Errors
    ///
    /// Rejects either unspecified endpoint.
    pub fn with_color_spaces(
        mut self,
        source: ColorEncoding,
        analysis: ColorEncoding,
    ) -> Result<Self, AnalysisRequestError> {
        if !source.is_explicit() || !analysis.is_explicit() {
            return Err(AnalysisRequestError::UnspecifiedColorSpace);
        }
        self.source_color_space = source;
        self.analysis_color_space = analysis;
        self.refresh_identity();
        Ok(self)
    }

    /// Returns a re-identified request with a different waveform orientation.
    #[must_use]
    pub fn with_waveform_orientation(mut self, orientation: WaveformOrientation) -> Self {
        self.waveform_orientation = orientation;
        self.refresh_identity();
        self
    }

    fn refresh_identity(&mut self) {
        self.identity = AnalysisRequestIdentity(Sha256::digest(self.canonical_bytes()).into());
    }

    fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(&ANALYSIS_SCHEMA_VERSION.to_le_bytes());
        bytes.push(kind_tag(self.kind));
        write_boundary(&self.boundary, &mut bytes);
        write_roi(self.roi, &mut bytes);
        write_dimensions(self.output, &mut bytes);
        write_color(self.source_color_space, &mut bytes);
        write_color(self.analysis_color_space, &mut bytes);
        write_sampling(self.sampling, &mut bytes);
        bytes.extend_from_slice(&self.range.minimum().to_bits().to_le_bytes());
        bytes.extend_from_slice(&self.range.maximum().to_bits().to_le_bytes());
        bytes.extend_from_slice(&[
            mask_tag(self.mask_policy),
            nonfinite_tag(self.nonfinite_policy),
            normalization_tag(self.normalization),
            graticule_tag(self.graticule),
        ]);
        write_intensity(self.intensity, &mut bytes);
        bytes.push(alpha_tag(self.alpha_policy));
        bytes.push(orientation_tag(self.waveform_orientation));
        bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisRequestError {
    UnspecifiedColorSpace,
    InvalidBoundary,
    InvalidDimensions,
    DimensionsExceeded,
    MemoryExceeded { requested: u64, limit: u64 },
    SizeOverflow,
    IntensityPrecisionExceeded,
}

/// A stable logical tile index plus a checked source-space rectangle. Exact-cover validation is
/// the caller/tile planner's responsibility, matching the existing histogram tile boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnalysisTile {
    index: usize,
    roi: Roi,
}

impl AnalysisTile {
    #[must_use]
    pub const fn new(index: usize, roi: Roi) -> Self {
        Self { index, roi }
    }
    #[must_use]
    pub const fn index(self) -> usize {
        self.index
    }
    #[must_use]
    pub const fn roi(self) -> Roi {
        self.roi
    }
}

fn output_bytes(
    kind: AnalysisKind,
    output: AnalysisOutputDimensions,
) -> Result<u64, AnalysisRequestError> {
    output
        .pixel_count()
        .map_err(|_| AnalysisRequestError::SizeOverflow)?
        .checked_mul(u64::from(kind.plane_count()))
        .and_then(|cells| cells.checked_mul(u64::try_from(size_of::<u64>()).ok()?))
        .ok_or(AnalysisRequestError::SizeOverflow)
}

fn write_boundary(value: &NodeBoundary, bytes: &mut Vec<u8>) {
    match value.boundary() {
        Some(identity) => {
            bytes.push(1);
            bytes.extend_from_slice(&identity);
        }
        None => bytes.push(0),
    }
    bytes.extend_from_slice(&value.first().to_le_bytes());
    bytes.extend_from_slice(&value.last().to_le_bytes());
    let implementation = value.implementation();
    write_len_bytes(implementation.name().as_bytes(), bytes);
    bytes.extend_from_slice(&implementation.version().to_le_bytes());
    write_len_bytes(implementation.build().as_bytes(), bytes);
}

fn write_roi(value: Roi, bytes: &mut Vec<u8>) {
    for component in [value.x(), value.y(), value.width(), value.height()] {
        bytes.extend_from_slice(&component.to_le_bytes());
    }
}

fn write_dimensions(value: AnalysisOutputDimensions, bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&value.width().to_le_bytes());
    bytes.extend_from_slice(&value.height().to_le_bytes());
}

fn write_color(value: ColorEncoding, bytes: &mut Vec<u8>) {
    let encoded = postcard::to_allocvec(&value).expect("closed color encoding serializes");
    write_len_bytes(&encoded, bytes);
}

fn write_len_bytes(value: &[u8], bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(
        &u64::try_from(value.len())
            .expect("identity length fits")
            .to_le_bytes(),
    );
    bytes.extend_from_slice(value);
}

fn write_sampling(value: AnalysisSampling, bytes: &mut Vec<u8>) {
    match value {
        AnalysisSampling::EveryPixel => bytes.push(0),
        AnalysisSampling::Grid {
            x_step,
            y_step,
            x_phase,
            y_phase,
        } => {
            bytes.push(1);
            for component in [x_step.get(), y_step.get(), x_phase, y_phase] {
                bytes.extend_from_slice(&component.to_le_bytes());
            }
        }
    }
}

fn write_intensity(value: AnalysisIntensity, bytes: &mut Vec<u8>) {
    match value {
        AnalysisIntensity::Count => bytes.push(0),
        AnalysisIntensity::FixedPoint { fractional_bits } => {
            bytes.push(1);
            bytes.push(fractional_bits);
        }
    }
}

const fn kind_tag(value: AnalysisKind) -> u8 {
    match value {
        AnalysisKind::LuminanceWaveform => 0,
        AnalysisKind::RgbWaveform => 1,
        AnalysisKind::RgbParade => 2,
        AnalysisKind::Vectorscope => 3,
    }
}
const fn mask_tag(value: HistogramMaskPolicy) -> u8 {
    match value {
        HistogramMaskPolicy::Ignore => 0,
        HistogramMaskPolicy::IncludeNonZero => 1,
        HistogramMaskPolicy::ExcludeNonZero => 2,
        HistogramMaskPolicy::Require => 3,
    }
}
const fn nonfinite_tag(value: HistogramNonFinitePolicy) -> u8 {
    match value {
        HistogramNonFinitePolicy::Skip => 0,
        HistogramNonFinitePolicy::Reject => 1,
    }
}
const fn normalization_tag(value: AnalysisNormalization) -> u8 {
    match value {
        AnalysisNormalization::None => 0,
        AnalysisNormalization::Peak => 1,
        AnalysisNormalization::SampleIntensity => 2,
    }
}
const fn graticule_tag(value: AnalysisGraticule) -> u8 {
    match value {
        AnalysisGraticule::Rec601 => 0,
        AnalysisGraticule::Rec709 => 1,
        AnalysisGraticule::Rec2020 => 2,
    }
}
const fn alpha_tag(value: AnalysisAlphaPolicy) -> u8 {
    match value {
        AnalysisAlphaPolicy::Ignore => 0,
        AnalysisAlphaPolicy::ExcludeTransparent => 1,
        AnalysisAlphaPolicy::Weight => 2,
    }
}
const fn orientation_tag(value: WaveformOrientation) -> u8 {
    match value {
        WaveformOrientation::Horizontal => 0,
        WaveformOrientation::Vertical => 1,
    }
}

fn write_digest(value: [u8; 32], formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in value {
        write!(formatter, "{byte:02x}")?;
    }
    Ok(())
}

impl fmt::Display for AnalysisRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnspecifiedColorSpace => {
                formatter.write_str("analysis color spaces must be explicit")
            }
            Self::InvalidBoundary => {
                formatter.write_str("analysis node boundary range is reversed")
            }
            Self::InvalidDimensions => formatter.write_str("analysis dimensions must be nonzero"),
            Self::DimensionsExceeded => write!(
                formatter,
                "analysis dimensions exceed {MAX_ANALYSIS_DIMENSION}"
            ),
            Self::MemoryExceeded { requested, limit } => write!(
                formatter,
                "analysis requires {requested} bytes, limit is {limit}"
            ),
            Self::SizeOverflow => formatter.write_str("analysis output size overflowed"),
            Self::IntensityPrecisionExceeded => {
                formatter.write_str("analysis fixed-point precision exceeds 16 fractional bits")
            }
        }
    }
}

impl std::error::Error for AnalysisRequestError {}
