#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Mapping arithmetic order is preserved for IEEE-754 parity."
)]

//! Bounded Color Mapping CPU leaf ported from `src/iop/colormapping.c`.
//!
//! The leaf keeps the native Lab histogram transfer, source/target cluster
//! acquisition, deterministic scalar k-means, cluster mapping, Shepard weights,
//! bilateral lightness LUT smoothing, interpolation order, and RGBA CPU
//! publication boundary.  The coupled CPU bilateral grid is consumed from the
//! already-portable `src/common/bilateral.c` mapping.  Process-global,
//! per-worker `darktable.points` ownership is retained behind the operation-local
//! source-shaped state transition. Registry, typed history import, and scalar
//! pixelpipe routing are integrated here; GPU/OpenCL, GTK, profile-preview
//! colorspace transforms, masks, and outer blending remain explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::if_same_then_else,
    clippy::large_stack_arrays,
    clippy::many_single_char_names,
    clippy::manual_clamp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::type_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    dead_code,
    reason = "the source-shaped bounded leaf retains native f32 equations and deferred symbols"
)]

use std::fmt;
use std::mem::size_of;

use rusttable_processing::RasterDimensions;
use rusttable_processing::common::bilateral::{BilateralError, BilateralGeometry, BilateralGrid};

pub mod source_map;

/// Native module compatibility identity.
pub const COLOR_MAPPING_COMPATIBILITY_ID: &str = "colormapping";
/// Stable Rust identity reserved for later registry integration.
pub const COLOR_MAPPING_RUST_ID: &str = "rusttable.colormapping";
/// Native `DT_MODULE_INTROSPECTION(1, ...)` schema version.
pub const COLOR_MAPPING_SCHEMA_VERSION: u16 = 1;
/// Native histogram resolution from `HISTN`.
pub const HISTN: usize = 1 << 11;
/// Native maximum number of clusters from `MAXN`.
pub const MAXN: usize = 5;
/// Native four-channel Lab pixel layout.
pub const COLOR_MAPPING_CHANNELS: usize = 4;
/// Exact current native parameter payload size on little-endian targets.
pub const COLOR_MAPPING_PARAMETER_BYTES: usize = 16_600;
/// Native parameter default for the cluster count.
pub const DEFAULT_CLUSTERS: i32 = 3;
/// Native parameter default for dominance.
pub const DEFAULT_DOMINANCE: f32 = 100.0;
/// Native parameter default for histogram equalization.
pub const DEFAULT_EQUALIZATION: f32 = 50.0;
/// Native default colorspace declaration from `default_colorspace`.
pub const DEFAULT_COLORSPACE: &str = "Lab";
/// Native module group names from `default_group`.
pub const DEFAULT_GROUPS: [&str; 2] = ["effect", "effects"];
/// Native GPU program index (`extended.cl`, `programs.conf`).
pub const GPU_PROGRAM: u32 = 8;
/// Native GPU kernel names retained as a deferred capability boundary.
pub const GPU_KERNELS: [&str; 2] = ["colormapping_histogram", "colormapping_mapping"];
/// GPU execution is not part of this CPU-only leaf.
pub const GPU_EXECUTABLE: bool = false;
/// There are no native history migrations for schema version one.
pub const MIGRATION_EDGES: &[(u16, u16)] = &[];
/// Native maximum iteration count for cluster acquisition.
const KMEANS_ITERATIONS: usize = 40;
/// Native histogram-to-lightness scale.
const LIGHTNESS_MAXIMUM: f32 = 100.0;
/// Native bilateral range sigma.
const BILATERAL_RANGE_SIGMA: f32 = 8.0;
/// Native bilateral spatial sigma numerator.
const BILATERAL_SPATIAL_SIGMA: f32 = 50.0;
/// Bounded operation memory admission limit.
pub const DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;
/// Cancellation is polled often enough not to leave a long pixel loop opaque.
const CANCEL_POLL_INTERVAL: usize = 1024;

// The source is an enum-backed i32 field, not a Rust enum: all combinations of
// these bits are serialized by the native history ABI.
pub const FLAG_NEUTRAL: i32 = 0;
pub const FLAG_HAS_SOURCE: i32 = 1 << 0;
pub const FLAG_HAS_TARGET: i32 = 1 << 1;
pub const FLAG_HAS_SOURCE_TARGET: i32 = FLAG_HAS_SOURCE | FLAG_HAS_TARGET;
pub const FLAG_ACQUIRE: i32 = 1 << 2;
pub const FLAG_GET_SOURCE: i32 = 1 << 3;
pub const FLAG_GET_TARGET: i32 = 1 << 4;

/// Native current parameter payload in declaration order.
#[derive(Debug, Clone)]
pub struct ColorMappingParametersV1 {
    pub flag: i32,
    pub n: i32,
    pub dominance: f32,
    pub equalization: f32,
    pub source_ihist: [f32; HISTN],
    pub source_mean: [[f32; 2]; MAXN],
    pub source_var: [[f32; 2]; MAXN],
    pub source_weight: [f32; MAXN],
    pub target_hist: [i32; HISTN],
    pub target_mean: [[f32; 2]; MAXN],
    pub target_var: [[f32; 2]; MAXN],
    pub target_weight: [f32; MAXN],
}

impl PartialEq for ColorMappingParametersV1 {
    fn eq(&self, other: &Self) -> bool {
        self.to_bytes() == other.to_bytes()
    }
}

impl Eq for ColorMappingParametersV1 {}

impl std::hash::Hash for ColorMappingParametersV1 {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.to_bytes(), state);
    }
}

impl Default for ColorMappingParametersV1 {
    fn default() -> Self {
        Self::defaults()
    }
}

impl ColorMappingParametersV1 {
    /// Native defaults emitted by the introspection annotations and zeroed
    /// analysis arrays.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            flag: FLAG_NEUTRAL,
            n: DEFAULT_CLUSTERS,
            dominance: DEFAULT_DOMINANCE,
            equalization: DEFAULT_EQUALIZATION,
            source_ihist: [0.0; HISTN],
            source_mean: [[0.0; 2]; MAXN],
            source_var: [[0.0; 2]; MAXN],
            source_weight: [0.0; MAXN],
            target_hist: [0; HISTN],
            target_mean: [[0.0; 2]; MAXN],
            target_var: [[0.0; 2]; MAXN],
            target_weight: [0.0; MAXN],
        }
    }

    /// Serializes the native little-endian field order without padding.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(COLOR_MAPPING_PARAMETER_BYTES);
        push_i32(&mut bytes, self.flag);
        push_i32(&mut bytes, self.n);
        push_f32(&mut bytes, self.dominance);
        push_f32(&mut bytes, self.equalization);
        for value in self.source_ihist {
            push_f32(&mut bytes, value);
        }
        for pair in self.source_mean {
            for value in pair {
                push_f32(&mut bytes, value);
            }
        }
        for pair in self.source_var {
            for value in pair {
                push_f32(&mut bytes, value);
            }
        }
        for value in self.source_weight {
            push_f32(&mut bytes, value);
        }
        for value in self.target_hist {
            push_i32(&mut bytes, value);
        }
        for pair in self.target_mean {
            for value in pair {
                push_f32(&mut bytes, value);
            }
        }
        for pair in self.target_var {
            for value in pair {
                push_f32(&mut bytes, value);
            }
        }
        for value in self.target_weight {
            push_f32(&mut bytes, value);
        }
        debug_assert_eq!(bytes.len(), COLOR_MAPPING_PARAMETER_BYTES);
        bytes
    }

    /// Decodes exactly one native v1 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorMappingCodecError> {
        if bytes.len() != COLOR_MAPPING_PARAMETER_BYTES {
            return Err(ColorMappingCodecError::InvalidLength {
                expected: COLOR_MAPPING_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut decoder = Decoder::new(bytes);
        let flag = decoder.i32();
        let n = decoder.i32();
        let dominance = decoder.f32();
        let equalization = decoder.f32();
        let source_ihist = std::array::from_fn(|_| decoder.f32());
        let source_mean = std::array::from_fn(|_| [decoder.f32(), decoder.f32()]);
        let source_var = std::array::from_fn(|_| [decoder.f32(), decoder.f32()]);
        let source_weight = std::array::from_fn(|_| decoder.f32());
        let target_hist = std::array::from_fn(|_| decoder.i32());
        let target_mean = std::array::from_fn(|_| [decoder.f32(), decoder.f32()]);
        let target_var = std::array::from_fn(|_| [decoder.f32(), decoder.f32()]);
        let target_weight = std::array::from_fn(|_| decoder.f32());
        debug_assert_eq!(decoder.offset, COLOR_MAPPING_PARAMETER_BYTES);
        Ok(Self {
            flag,
            n,
            dominance,
            equalization,
            source_ihist,
            source_mean,
            source_var,
            source_weight,
            target_hist,
            target_mean,
            target_var,
            target_weight,
        })
    }

    #[must_use]
    pub const fn has_source(&self) -> bool {
        self.flag & FLAG_HAS_SOURCE != 0
    }

    #[must_use]
    pub const fn has_target(&self) -> bool {
        self.flag & FLAG_HAS_TARGET != 0
    }

    /// Applies native source acquisition output and sets `HAS_SOURCE`.
    #[must_use]
    pub const fn with_source_analysis(mut self, analysis: &ColorMappingSourceAnalysis) -> Self {
        self.source_ihist = analysis.inverse_histogram;
        self.source_mean = analysis.mean;
        self.source_var = analysis.variance;
        self.source_weight = analysis.weight;
        self.flag &= !(FLAG_ACQUIRE | FLAG_GET_SOURCE | FLAG_GET_TARGET);
        self.flag |= FLAG_HAS_SOURCE;
        self
    }

    /// Applies native target acquisition output and sets `HAS_TARGET`.
    #[must_use]
    pub const fn with_target_analysis(mut self, analysis: &ColorMappingTargetAnalysis) -> Self {
        self.target_hist = analysis.histogram;
        self.target_mean = analysis.mean;
        self.target_var = analysis.variance;
        self.target_weight = analysis.weight;
        self.flag &= !(FLAG_ACQUIRE | FLAG_GET_SOURCE | FLAG_GET_TARGET);
        self.flag |= FLAG_HAS_TARGET;
        self
    }

    /// Resets both acquired analyses as the native cluster slider does.
    #[must_use]
    pub const fn reset_analysis(mut self) -> Self {
        self.source_ihist = [0.0; HISTN];
        self.source_mean = [[0.0; 2]; MAXN];
        self.source_var = [[0.0; 2]; MAXN];
        self.source_weight = [0.0; MAXN];
        self.target_hist = [0; HISTN];
        self.target_mean = [[0.0; 2]; MAXN];
        self.target_var = [[0.0; 2]; MAXN];
        self.target_weight = [0.0; MAXN];
        self.flag = FLAG_NEUTRAL;
        self
    }
}

/// Known native history payload and byte-preserved future values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorMappingHistory {
    V1(Box<ColorMappingParametersV1>),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorMappingHistory {
    /// Decodes version one and retains unknown versions without guessing.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorMappingCodecError> {
        match version {
            COLOR_MAPPING_SCHEMA_VERSION => Ok(Self::V1(Box::new(
                ColorMappingParametersV1::from_bytes(bytes)?,
            ))),
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => COLOR_MAPPING_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// Materializes the only known schema; future values stay blocked.
    pub fn current(&self) -> Result<ColorMappingParametersV1, ColorMappingCodecError> {
        match self {
            Self::V1(parameters) => Ok((**parameters).clone()),
            Self::Opaque { version, .. } => {
                Err(ColorMappingCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

/// Parameter codec failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorMappingCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "Color Mapping payload has {actual} bytes; expected {expected}"
            ),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "Color Mapping version {version} is opaque and unsupported"
            ),
        }
    }
}

impl std::error::Error for ColorMappingCodecError {}

/// Checked finite parameters used by a CPU plan.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorMappingConfig {
    parameters: ColorMappingParametersV1,
}

impl ColorMappingConfig {
    pub fn new(parameters: ColorMappingParametersV1) -> Result<Self, ColorMappingParameterError> {
        validate_parameters(&parameters)?;
        Ok(Self { parameters })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(ColorMappingParametersV1::defaults())
            .expect("native Color Mapping defaults are finite and valid")
    }

    #[must_use]
    pub const fn parameters(&self) -> &ColorMappingParametersV1 {
        &self.parameters
    }
}

impl TryFrom<ColorMappingParametersV1> for ColorMappingConfig {
    type Error = ColorMappingParameterError;

    fn try_from(parameters: ColorMappingParametersV1) -> Result<Self, Self::Error> {
        Self::new(parameters)
    }
}

/// Parameter validation failures at the bounded execution boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorMappingParameterError {
    InvalidClusterCount(i32),
    NonFinite(&'static str),
    InvalidHistogramBin { index: usize, value: i32 },
}

impl fmt::Display for ColorMappingParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidClusterCount(value) => {
                write!(
                    formatter,
                    "Color Mapping cluster count {value} is outside 1..={MAXN}"
                )
            }
            Self::NonFinite(name) => write!(formatter, "Color Mapping {name} is non-finite"),
            Self::InvalidHistogramBin { index, value } => write!(
                formatter,
                "Color Mapping target histogram bin {index} has invalid value {value}"
            ),
        }
    }
}

impl std::error::Error for ColorMappingParameterError {}

fn validate_parameters(
    parameters: &ColorMappingParametersV1,
) -> Result<(), ColorMappingParameterError> {
    if !(1..=i32::try_from(MAXN).expect("MAXN fits i32")).contains(&parameters.n) {
        return Err(ColorMappingParameterError::InvalidClusterCount(
            parameters.n,
        ));
    }
    finite(parameters.dominance, "dominance")?;
    finite(parameters.equalization, "equalization")?;
    for value in parameters.source_ihist {
        finite(value, "source_ihist")?;
    }
    for pair in parameters.source_mean {
        for value in pair {
            finite(value, "source_mean")?;
        }
    }
    for pair in parameters.source_var {
        for value in pair {
            finite(value, "source_var")?;
        }
    }
    for value in parameters.source_weight {
        finite(value, "source_weight")?;
    }
    for (index, value) in parameters.target_hist.into_iter().enumerate() {
        if !(0..i32::try_from(HISTN).expect("HISTN fits i32")).contains(&value) {
            return Err(ColorMappingParameterError::InvalidHistogramBin { index, value });
        }
    }
    for pair in parameters.target_mean {
        for value in pair {
            finite(value, "target_mean")?;
        }
    }
    for pair in parameters.target_var {
        for value in pair {
            finite(value, "target_var")?;
        }
    }
    for value in parameters.target_weight {
        finite(value, "target_weight")?;
    }
    Ok(())
}

const fn finite(value: f32, name: &'static str) -> Result<(), ColorMappingParameterError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(ColorMappingParameterError::NonFinite(name))
    }
}

/// Four-float Lab pixel in native channel order: L, a, b, alpha.
pub type ColorMappingPixel = [f32; COLOR_MAPPING_CHANNELS];

/// One injected xorshift128+ state matching native `dt_points_get_for`.
///
/// Darktable's `darktable.points` owns one of these states per worker thread for
/// the process lifetime.  This value models one stream only; it deliberately
/// does not claim that shared lifecycle or choose a worker.  Production callers
/// must inject state from the eventual shared points owner through the
/// `from_pixels_with_points*` entry points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PointsState {
    state0: u64,
    state1: u64,
}

impl Default for PointsState {
    fn default() -> Self {
        Self::new()
    }
}

impl PointsState {
    /// Native initial state for worker thread zero.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state0: 1,
            state1: 2,
        }
    }

    /// Returns the next native `dt_points_get` value and advances the stream.
    pub fn next_f32(&mut self) -> f32 {
        let mut s1 = self.state0;
        let s0 = self.state1;
        self.state0 = s0;
        s1 ^= s1 << 23;
        s1 ^= s1 >> 17;
        s1 ^= s0;
        s1 ^= s0 >> 26;
        self.state1 = s1;
        let bits = 0x3f80_0000_u32 | ((self.state0.wrapping_add(self.state1) >> 41) as u32);
        f32::from_bits(bits) - 1.0
    }
}

/// Operation-local convenience owner for successive bounded acquisitions.
///
/// This owner intentionally resets to worker-zero seeds when constructed.  It
/// is suitable for deterministic leaf use and tests, not as a substitute for
/// native process-global points ownership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorMappingAcquisition {
    points: PointsState,
}

impl Default for ColorMappingAcquisition {
    fn default() -> Self {
        Self::new()
    }
}

impl ColorMappingAcquisition {
    /// Starts an operation-local stream with native worker-zero seeds.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            points: PointsState::new(),
        }
    }

    /// Exposes the operation-local stream for source-derived assertions.
    #[must_use]
    pub const fn points(&self) -> &PointsState {
        &self.points
    }

    /// Acquires source statistics while retaining the stream for the next
    /// acquisition.
    pub fn source_analysis(
        &mut self,
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
    ) -> Result<ColorMappingSourceAnalysis, ColorMappingAnalysisError> {
        ColorMappingSourceAnalysis::from_pixels_with_points(
            dimensions,
            input,
            clusters,
            &mut self.points,
        )
    }

    /// Cancellable source acquisition with persistent native point state.
    pub fn source_analysis_with_cancel<F: FnMut() -> bool>(
        &mut self,
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        cancelled: F,
    ) -> Result<ColorMappingSourceAnalysis, ColorMappingAnalysisError> {
        ColorMappingSourceAnalysis::from_pixels_with_points_and_cancel(
            dimensions,
            input,
            clusters,
            &mut self.points,
            cancelled,
        )
    }

    /// Acquires target statistics using the state left by the source
    /// acquisition.
    pub fn target_analysis(
        &mut self,
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
    ) -> Result<ColorMappingTargetAnalysis, ColorMappingAnalysisError> {
        ColorMappingTargetAnalysis::from_pixels_with_points(
            dimensions,
            input,
            clusters,
            &mut self.points,
        )
    }

    /// Cancellable target acquisition with persistent native point state.
    pub fn target_analysis_with_cancel<F: FnMut() -> bool>(
        &mut self,
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        cancelled: F,
    ) -> Result<ColorMappingTargetAnalysis, ColorMappingAnalysisError> {
        ColorMappingTargetAnalysis::from_pixels_with_points_and_cancel(
            dimensions,
            input,
            clusters,
            &mut self.points,
            cancelled,
        )
    }
}

/// Channel labels used by finite input/output diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingChannel {
    Lightness,
    A,
    B,
    Alpha,
}

/// Source acquisition result: inverse L histogram and sorted clusters.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorMappingSourceAnalysis {
    pub inverse_histogram: [f32; HISTN],
    pub mean: [[f32; 2]; MAXN],
    pub variance: [[f32; 2]; MAXN],
    pub weight: [f32; MAXN],
}

impl ColorMappingSourceAnalysis {
    /// An isolated convenience analysis with fresh worker-zero seeds.
    /// Use an injected [`PointsState`] for production-lifetime sequencing, or
    /// [`ColorMappingAcquisition`] for deterministic operation-local captures.
    pub fn from_pixels(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
    ) -> Result<Self, ColorMappingAnalysisError> {
        let mut points = PointsState::new();
        Self::from_pixels_with_points(dimensions, input, clusters, &mut points)
    }

    /// Analyzes pixels using caller-owned persistent native point state.
    pub fn from_pixels_with_points(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        points: &mut PointsState,
    ) -> Result<Self, ColorMappingAnalysisError> {
        Self::from_pixels_with_points_and_cancel(dimensions, input, clusters, points, || false)
    }

    /// An isolated cancellable convenience analysis.
    pub fn from_pixels_with_cancel<F: FnMut() -> bool>(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        cancelled: F,
    ) -> Result<Self, ColorMappingAnalysisError> {
        let mut points = PointsState::new();
        Self::from_pixels_with_points_and_cancel(
            dimensions,
            input,
            clusters,
            &mut points,
            cancelled,
        )
    }

    /// Cancellable source analysis using caller-owned persistent point state.
    pub fn from_pixels_with_points_and_cancel<F: FnMut() -> bool>(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        points: &mut PointsState,
        mut cancelled: F,
    ) -> Result<Self, ColorMappingAnalysisError> {
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        let (width, height) = checked_dimensions(dimensions, input.len())?;
        validate_analysis_input_with_cancel(input, &mut cancelled)?;
        let histogram = capture_histogram_with_cancel(input, width, height, &mut cancelled)?;
        let inverse_histogram = invert_histogram_with_cancel(&histogram, &mut cancelled)?;
        let (mean, variance, weight) =
            kmeans_with_cancel(input, width, height, clusters, points, &mut cancelled)?;
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        Ok(Self {
            inverse_histogram,
            mean,
            variance,
            weight,
        })
    }
}

/// Target acquisition result: normalized L histogram and sorted clusters.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorMappingTargetAnalysis {
    pub histogram: [i32; HISTN],
    pub mean: [[f32; 2]; MAXN],
    pub variance: [[f32; 2]; MAXN],
    pub weight: [f32; MAXN],
}

impl ColorMappingTargetAnalysis {
    /// An isolated convenience analysis with fresh worker-zero seeds.
    /// Use an injected [`PointsState`] for production-lifetime sequencing, or
    /// [`ColorMappingAcquisition`] for deterministic operation-local captures.
    pub fn from_pixels(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
    ) -> Result<Self, ColorMappingAnalysisError> {
        let mut points = PointsState::new();
        Self::from_pixels_with_points(dimensions, input, clusters, &mut points)
    }

    /// Analyzes pixels using caller-owned persistent native point state.
    pub fn from_pixels_with_points(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        points: &mut PointsState,
    ) -> Result<Self, ColorMappingAnalysisError> {
        Self::from_pixels_with_points_and_cancel(dimensions, input, clusters, points, || false)
    }

    /// An isolated cancellable convenience analysis.
    pub fn from_pixels_with_cancel<F: FnMut() -> bool>(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        cancelled: F,
    ) -> Result<Self, ColorMappingAnalysisError> {
        let mut points = PointsState::new();
        Self::from_pixels_with_points_and_cancel(
            dimensions,
            input,
            clusters,
            &mut points,
            cancelled,
        )
    }

    /// Cancellable target analysis using caller-owned persistent point state.
    pub fn from_pixels_with_points_and_cancel<F: FnMut() -> bool>(
        dimensions: RasterDimensions,
        input: &[ColorMappingPixel],
        clusters: usize,
        points: &mut PointsState,
        mut cancelled: F,
    ) -> Result<Self, ColorMappingAnalysisError> {
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        let (width, height) = checked_dimensions(dimensions, input.len())?;
        validate_analysis_input_with_cancel(input, &mut cancelled)?;
        let histogram = capture_histogram_with_cancel(input, width, height, &mut cancelled)?;
        let (mean, variance, weight) =
            kmeans_with_cancel(input, width, height, clusters, points, &mut cancelled)?;
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        Ok(Self {
            histogram,
            mean,
            variance,
            weight,
        })
    }
}

/// Captures the source-shaped normalized lightness histogram.
pub fn capture_histogram(
    dimensions: RasterDimensions,
    input: &[ColorMappingPixel],
) -> Result<[i32; HISTN], ColorMappingAnalysisError> {
    let (width, height) = checked_dimensions(dimensions, input.len())?;
    validate_analysis_input_with_cancel(input, &mut || false)?;
    capture_histogram_with_cancel(input, width, height, &mut || false)
}

/// Inverts a native accumulated histogram with its deliberately nonzero first
/// search index and monotonic lower-bound scan.
#[must_use]
pub fn invert_histogram(histogram: &[i32; HISTN]) -> [f32; HISTN] {
    invert_histogram_with_cancel(histogram, &mut || false)
        .expect("non-cancellable histogram inversion cannot be cancelled")
}

fn invert_histogram_with_cancel<F: FnMut() -> bool>(
    histogram: &[i32; HISTN],
    cancelled: &mut F,
) -> Result<[f32; HISTN], ColorMappingAnalysisError> {
    let mut inverse = [0.0; HISTN];
    let mut last = 31;
    for (index, value) in inverse.iter_mut().take(last + 1).enumerate() {
        *value = LIGHTNESS_MAXIMUM * index as f32 / HISTN as f32;
    }
    for (index, value) in inverse.iter_mut().enumerate().skip(last + 1) {
        if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        for (k, &histogram_value) in histogram.iter().enumerate().skip(last) {
            if histogram_value >= i32::try_from(index).expect("HISTN fits i32") {
                last = k;
                *value = LIGHTNESS_MAXIMUM * k as f32 / HISTN as f32;
                break;
            }
        }
    }
    if cancelled() {
        Err(ColorMappingAnalysisError::Cancelled)
    } else {
        Ok(inverse)
    }
}

/// Errors from source/target acquisition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorMappingAnalysisError {
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    SizeOverflow,
    InvalidClusterCount(usize),
    PixelCountTooLarge(usize),
    NonFiniteInput {
        pixel: usize,
        channel: ColorMappingChannel,
    },
    NonFiniteAnalysis {
        cluster: usize,
        channel: usize,
    },
    Cancelled,
}

impl fmt::Display for ColorMappingAnalysisError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => write!(
                formatter,
                "Color Mapping analysis expected {expected} pixels, got {actual}"
            ),
            Self::SizeOverflow => {
                formatter.write_str("Color Mapping analysis dimensions overflowed")
            }
            Self::InvalidClusterCount(value) => {
                write!(
                    formatter,
                    "Color Mapping analysis cluster count {value} is invalid"
                )
            }
            Self::PixelCountTooLarge(value) => write!(
                formatter,
                "Color Mapping analysis pixel count {value} exceeds native i32 histogram capacity"
            ),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "Color Mapping input pixel {pixel} has non-finite {channel:?}"
                )
            }
            Self::NonFiniteAnalysis { cluster, channel } => write!(
                formatter,
                "Color Mapping cluster {cluster} channel {channel} is non-finite"
            ),
            Self::Cancelled => formatter.write_str("Color Mapping analysis was cancelled"),
        }
    }
}

impl std::error::Error for ColorMappingAnalysisError {}

fn checked_dimensions(
    dimensions: RasterDimensions,
    actual: usize,
) -> Result<(usize, usize), ColorMappingAnalysisError> {
    let expected = usize::try_from(dimensions.pixel_count())
        .map_err(|_| ColorMappingAnalysisError::SizeOverflow)?;
    if expected != actual {
        return Err(ColorMappingAnalysisError::DimensionsMismatch { expected, actual });
    }
    if expected > i32::MAX as usize {
        return Err(ColorMappingAnalysisError::PixelCountTooLarge(expected));
    }
    let width =
        usize::try_from(dimensions.width()).map_err(|_| ColorMappingAnalysisError::SizeOverflow)?;
    let height = usize::try_from(dimensions.height())
        .map_err(|_| ColorMappingAnalysisError::SizeOverflow)?;
    Ok((width, height))
}

fn validate_analysis_input_with_cancel<F: FnMut() -> bool>(
    input: &[ColorMappingPixel],
    cancelled: &mut F,
) -> Result<(), ColorMappingAnalysisError> {
    for (chunk_index, chunk) in input.chunks(CANCEL_POLL_INTERVAL).enumerate() {
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        let chunk_start = chunk_index * CANCEL_POLL_INTERVAL;
        for (offset, values) in chunk.iter().enumerate() {
            // Lab acquisition reads only L/a/b. Native processing deliberately
            // leaves alpha out of analysis, so it may carry NaN or infinity.
            for (channel, value) in values.iter().copied().enumerate().take(3) {
                if !value.is_finite() {
                    return Err(ColorMappingAnalysisError::NonFiniteInput {
                        pixel: chunk_start + offset,
                        channel: channel_from_index(channel),
                    });
                }
            }
        }
    }
    if cancelled() {
        Err(ColorMappingAnalysisError::Cancelled)
    } else {
        Ok(())
    }
}

fn capture_histogram_with_cancel<F: FnMut() -> bool>(
    input: &[ColorMappingPixel],
    width: usize,
    height: usize,
    cancelled: &mut F,
) -> Result<[i32; HISTN], ColorMappingAnalysisError> {
    let mut histogram = [0_i32; HISTN];
    for chunk in input.chunks(CANCEL_POLL_INTERVAL) {
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        for pixel in chunk {
            let lightness = pixel[0];
            let bin = (HISTN as f32 * lightness / LIGHTNESS_MAXIMUM).clamp(0.0, HISTN as f32 - 1.0)
                as usize;
            histogram[bin] = histogram[bin]
                .checked_add(1)
                .ok_or(ColorMappingAnalysisError::PixelCountTooLarge(input.len()))?;
        }
    }
    if cancelled() {
        return Err(ColorMappingAnalysisError::Cancelled);
    }
    for index in 1..HISTN {
        if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        histogram[index] = histogram[index]
            .checked_add(histogram[index - 1])
            .ok_or(ColorMappingAnalysisError::PixelCountTooLarge(input.len()))?;
    }
    let _ = (width, height);
    let normalizer = HISTN as f32 / histogram[HISTN - 1] as f32;
    for chunk in histogram.chunks_mut(CANCEL_POLL_INTERVAL) {
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        for value in chunk {
            *value = (*value as f32 * normalizer).clamp(0.0, HISTN as f32 - 1.0) as i32;
        }
    }
    if cancelled() {
        Err(ColorMappingAnalysisError::Cancelled)
    } else {
        Ok(histogram)
    }
}

fn kmeans_with_cancel<F: FnMut() -> bool>(
    input: &[ColorMappingPixel],
    width: usize,
    height: usize,
    clusters: usize,
    points: &mut PointsState,
    cancelled: &mut F,
) -> Result<([[f32; 2]; MAXN], [[f32; 2]; MAXN], [f32; MAXN]), ColorMappingAnalysisError> {
    if !(1..=MAXN).contains(&clusters) {
        return Err(ColorMappingAnalysisError::InvalidClusterCount(clusters));
    }
    let mut mean_out = [[0.0; 2]; MAXN];
    let mut var_out = [[0.0; 2]; MAXN];
    let mut weight_out = [0.0; MAXN];
    let mut mean = [[0.0; 2]; MAXN];
    let mut variance = [[0.0; 2]; MAXN];
    let mut count = [0_i32; MAXN];

    let mut a_min = f32::MAX;
    let mut b_min = f32::MAX;
    let mut a_max = -f32::MAX;
    let mut b_max = -f32::MAX;
    for (index, pixel) in input.iter().enumerate() {
        if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
        a_min = native_min(a_min, pixel[1]);
        a_max = native_max(a_max, pixel[1]);
        b_min = native_min(b_min, pixel[2]);
        b_max = native_max(b_max, pixel[2]);
    }
    for cluster in 0..clusters {
        mean_out[cluster][0] = 0.9 * (a_min + (a_max - a_min) * points.next_f32());
        mean_out[cluster][1] = 0.9 * (b_min + (b_max - b_min) * points.next_f32());
        var_out[cluster] = [0.0; 2];
        weight_out[cluster] = 0.0;
    }

    for _iteration in 0..KMEANS_ITERATIONS {
        count.fill(0);
        mean.fill([0.0; 2]);
        variance.fill([0.0; 2]);
        for (index, pixel) in input.iter().enumerate() {
            if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
                return Err(ColorMappingAnalysisError::Cancelled);
            }
            let cluster = get_cluster(pixel, clusters, &mean_out);
            count[cluster] += 1;
            variance[cluster][0] += pixel[1] * pixel[1];
            variance[cluster][1] += pixel[2] * pixel[2];
            mean[cluster][0] += pixel[1];
            mean[cluster][1] += pixel[2];
        }
        for cluster in 0..clusters {
            if count[cluster] == 0 {
                continue;
            }
            mean_out[cluster][0] = mean[cluster][0] / count[cluster] as f32;
            mean_out[cluster][1] = mean[cluster][1] / count[cluster] as f32;
            var_out[cluster][0] = variance[cluster][0] / count[cluster] as f32
                - mean_out[cluster][0] * mean_out[cluster][0];
            var_out[cluster][1] = variance[cluster][1] / count[cluster] as f32
                - mean_out[cluster][1] * mean_out[cluster][1];
        }
        let total = count[..clusters].iter().sum::<i32>();
        for cluster in 0..clusters {
            weight_out[cluster] = if total > 0 {
                count[cluster] as f32 / total as f32
            } else {
                0.0
            };
        }
        if cancelled() {
            return Err(ColorMappingAnalysisError::Cancelled);
        }
    }

    for cluster in 0..clusters {
        if var_out[cluster][0] == 0.0 || var_out[cluster][1] == 0.0 {
            mean_out[cluster] = [0.0; 2];
            var_out[cluster] = [0.0; 2];
            weight_out[cluster] = 0.0;
        }
        var_out[cluster][0] = var_out[cluster][0].sqrt();
        var_out[cluster][1] = var_out[cluster][1].sqrt();
        for channel in 0..2 {
            if !mean_out[cluster][channel].is_finite()
                || !var_out[cluster][channel].is_finite()
                || !weight_out[cluster].is_finite()
            {
                return Err(ColorMappingAnalysisError::NonFiniteAnalysis { cluster, channel });
            }
        }
    }

    // Native convenience sort: ascending weight, stable for ties.
    for i in 0..clusters.saturating_sub(1) {
        for j in 0..clusters - 1 - i {
            if weight_out[j] > weight_out[j + 1] {
                mean_out.swap(j, j + 1);
                var_out.swap(j, j + 1);
                weight_out.swap(j, j + 1);
            }
        }
    }
    let _ = (width, height);
    if cancelled() {
        Err(ColorMappingAnalysisError::Cancelled)
    } else {
        Ok((mean_out, var_out, weight_out))
    }
}

fn get_cluster(pixel: &ColorMappingPixel, clusters: usize, mean: &[[f32; 2]; MAXN]) -> usize {
    let mut minimum_distance = f32::MAX;
    let mut cluster = 0;
    for (index, cluster_mean) in mean.iter().enumerate().take(clusters) {
        let distance = (pixel[1] - cluster_mean[0]) * (pixel[1] - cluster_mean[0])
            + (pixel[2] - cluster_mean[1]) * (pixel[2] - cluster_mean[1]);
        if distance < minimum_distance {
            minimum_distance = distance;
            cluster = index;
        }
    }
    cluster
}

fn native_min(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left < right {
        left
    } else if right < left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() || right.is_sign_negative() {
            -0.0
        } else {
            left
        }
    } else {
        left
    }
}

fn native_max(left: f32, right: f32) -> f32 {
    if left.is_nan() {
        right
    } else if right.is_nan() {
        left
    } else if left > right {
        left
    } else if right > left {
        right
    } else if left == 0.0 && right == 0.0 {
        if left.is_sign_negative() && right.is_sign_negative() {
            left
        } else {
            0.0
        }
    } else {
        left
    }
}

const fn channel_from_index(index: usize) -> ColorMappingChannel {
    match index {
        0 => ColorMappingChannel::Lightness,
        1 => ColorMappingChannel::A,
        2 => ColorMappingChannel::B,
        _ => ColorMappingChannel::Alpha,
    }
}

/// Errors while constructing a checked CPU plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingPlanError {
    InvalidScale,
}

impl fmt::Display for ColorMappingPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidScale => {
                formatter.write_str("Color Mapping scale must be finite and positive")
            }
        }
    }
}

impl std::error::Error for ColorMappingPlanError {}

/// Tiling values reproduced from the native `tiling_callback`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorMappingTiling {
    pub factor: f32,
    pub max_buffer: f32,
    pub overlap: u32,
    pub alignment: u32,
}

/// Errors from checked scalar execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingExecutionError {
    DimensionsMismatch {
        expected: usize,
        actual: usize,
    },
    SizeOverflow,
    InvalidThreadCount,
    AllocationFailed {
        required_bytes: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: ColorMappingChannel,
    },
    NonFiniteOutput {
        pixel: usize,
        channel: ColorMappingChannel,
    },
    InvalidBilateralParameter(&'static str),
    Cancelled,
}

impl fmt::Display for ColorMappingExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DimensionsMismatch { expected, actual } => write!(
                formatter,
                "Color Mapping expected {expected} pixels, got {actual}"
            ),
            Self::SizeOverflow => formatter.write_str("Color Mapping raster size overflowed"),
            Self::InvalidThreadCount => {
                formatter.write_str("Color Mapping tiling requires at least one CPU thread")
            }
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Color Mapping allocation failed for {required_bytes} bytes"
            ),
            Self::NonFiniteInput { pixel, channel } => {
                write!(
                    formatter,
                    "Color Mapping input pixel {pixel} has non-finite {channel:?}"
                )
            }
            Self::NonFiniteOutput { pixel, channel } => write!(
                formatter,
                "Color Mapping produced non-finite {channel:?} at pixel {pixel}"
            ),
            Self::InvalidBilateralParameter(name) => {
                write!(
                    formatter,
                    "Color Mapping bilateral parameter {name} is invalid"
                )
            }
            Self::Cancelled => formatter.write_str("Color Mapping execution was cancelled"),
        }
    }
}

impl std::error::Error for ColorMappingExecutionError {}

/// Capability state for the bounded leaf.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorMappingCapabilities {
    pub cpu_supported: bool,
    pub gpu_supported: bool,
    pub gtk_supported: bool,
    pub supports_blending: bool,
    pub masks_consumed: bool,
    pub tiling_supported: bool,
    pub outer_blending_deferred: bool,
    pub production_routing_deferred: bool,
}

impl ColorMappingCapabilities {
    #[must_use]
    pub const fn bounded_cpu_leaf() -> Self {
        Self {
            cpu_supported: true,
            gpu_supported: GPU_EXECUTABLE,
            gtk_supported: false,
            supports_blending: true,
            masks_consumed: false,
            tiling_supported: true,
            outer_blending_deferred: true,
            production_routing_deferred: false,
        }
    }

    pub const fn require_gpu(self) -> Result<(), ColorMappingCapabilityError> {
        if self.gpu_supported {
            Ok(())
        } else {
            Err(ColorMappingCapabilityError::GpuUnavailable)
        }
    }

    pub const fn require_gtk(self) -> Result<(), ColorMappingCapabilityError> {
        if self.gtk_supported {
            Ok(())
        } else {
            Err(ColorMappingCapabilityError::GtkUnavailable)
        }
    }

    pub const fn require_outer_blending(self) -> Result<(), ColorMappingCapabilityError> {
        if self.outer_blending_deferred {
            Err(ColorMappingCapabilityError::OuterBlendingDeferred)
        } else {
            Ok(())
        }
    }

    pub const fn require_production_routing(self) -> Result<(), ColorMappingCapabilityError> {
        if self.production_routing_deferred {
            Err(ColorMappingCapabilityError::ProductionRoutingDeferred)
        } else {
            Ok(())
        }
    }
}

#[must_use]
pub const fn capabilities() -> ColorMappingCapabilities {
    ColorMappingCapabilities::bounded_cpu_leaf()
}

/// Deferred capability boundary errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMappingCapabilityError {
    GpuUnavailable,
    GtkUnavailable,
    OuterBlendingDeferred,
    ProductionRoutingDeferred,
}

impl fmt::Display for ColorMappingCapabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GpuUnavailable => {
                formatter.write_str("Color Mapping GPU execution is unavailable")
            }
            Self::GtkUnavailable => {
                formatter.write_str("Color Mapping GTK controls are unavailable")
            }
            Self::OuterBlendingDeferred => {
                formatter.write_str("Color Mapping outer blending is deferred")
            }
            Self::ProductionRoutingDeferred => {
                formatter.write_str("Color Mapping production routing is deferred")
            }
        }
    }
}

impl std::error::Error for ColorMappingCapabilityError {}

/// Immutable bounded CPU plan.
#[derive(Debug, Clone)]
pub struct ColorMappingPlan {
    config: ColorMappingConfig,
    dimensions: RasterDimensions,
    scale: f32,
    memory_budget: usize,
}

impl ColorMappingPlan {
    pub fn new(
        config: ColorMappingConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, ColorMappingPlanError> {
        Self::new_with_scale(config, dimensions, 1.0)
    }

    pub fn new_with_scale(
        config: ColorMappingConfig,
        dimensions: RasterDimensions,
        scale: f32,
    ) -> Result<Self, ColorMappingPlanError> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(ColorMappingPlanError::InvalidScale);
        }
        Ok(Self {
            config,
            dimensions,
            scale,
            memory_budget: DEFAULT_MEMORY_BUDGET,
        })
    }

    #[must_use]
    pub const fn with_memory_budget(mut self, memory_budget: usize) -> Self {
        self.memory_budget = memory_budget;
        self
    }

    #[must_use]
    pub const fn config(&self) -> &ColorMappingConfig {
        &self.config
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn memory_budget(&self) -> usize {
        self.memory_budget
    }

    /// Reproduces the native tiling callback's scalar contract.
    ///
    /// `cpu_threads` is native `dt_get_num_threads()`. The CPU bilateral
    /// implementation reserves three `size_x * size_z` scratch planes per
    /// worker, so omitting this argument would advertise an understated
    /// memory contract.
    pub fn tiling(
        &self,
        cpu_threads: usize,
    ) -> Result<ColorMappingTiling, ColorMappingExecutionError> {
        if cpu_threads == 0 {
            return Err(ColorMappingExecutionError::InvalidThreadCount);
        }
        let (width, height, pixels) = self.resolved_dimensions()?;
        let sigma_s = BILATERAL_SPATIAL_SIGMA / self.scale;
        let geometry = BilateralGeometry::new(width, height, sigma_s, BILATERAL_RANGE_SIGMA)
            .map_err(map_bilateral_error)?;
        let grid_bytes = geometry
            .grid_values()
            .checked_mul(size_of::<f32>())
            .ok_or(ColorMappingExecutionError::SizeOverflow)?;
        let scratch_per_thread = geometry.grid_dimensions()[0]
            .checked_mul(geometry.grid_dimensions()[2])
            .and_then(|value| value.checked_mul(3))
            .and_then(|value| value.checked_mul(size_of::<f32>()))
            .ok_or(ColorMappingExecutionError::SizeOverflow)?;
        let memory = grid_bytes
            .checked_add(
                scratch_per_thread
                    .checked_mul(cpu_threads)
                    .ok_or(ColorMappingExecutionError::SizeOverflow)?,
            )
            .ok_or(ColorMappingExecutionError::SizeOverflow)?;
        let basebuffer = pixels
            .checked_mul(size_of::<ColorMappingPixel>())
            .ok_or(ColorMappingExecutionError::SizeOverflow)?;
        let basebuffer = basebuffer as f32;
        let memory_f = memory as f32;
        Ok(ColorMappingTiling {
            factor: 3.0 + memory_f / basebuffer,
            max_buffer: (memory_f / basebuffer).max(1.0),
            overlap: (4.0 * sigma_s).ceil() as u32,
            alignment: 1,
        })
    }

    pub fn execute(
        &self,
        input: &[ColorMappingPixel],
    ) -> Result<Vec<ColorMappingPixel>, ColorMappingExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Executes one identity-ROI tile and publishes only a complete output.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ColorMappingPixel],
        mut cancelled: F,
    ) -> Result<Vec<ColorMappingPixel>, ColorMappingExecutionError> {
        // Cancellation wins even when dimensions or pixel values are malformed:
        // an already-obsolete request must not spend time validating its input.
        if cancelled() {
            return Err(ColorMappingExecutionError::Cancelled);
        }
        let (width, height, expected) = self.execution_dimensions(input.len())?;
        validate_execution_input_with_cancel(input, &mut cancelled)?;
        let parameters = self.config.parameters();
        let complete = parameters.has_source() && parameters.has_target();
        let equalization = parameters.equalization / LIGHTNESS_MAXIMUM;
        let required_bytes =
            self.required_execution_bytes(width, height, expected, complete, equalization)?;
        if required_bytes > self.memory_budget {
            return Err(ColorMappingExecutionError::AllocationFailed { required_bytes });
        }
        if cancelled() {
            return Err(ColorMappingExecutionError::Cancelled);
        }
        let mut output = Vec::new();
        output
            .try_reserve_exact(expected)
            .map_err(|_| ColorMappingExecutionError::AllocationFailed { required_bytes })?;

        if !complete {
            for chunk in input.chunks(CANCEL_POLL_INTERVAL) {
                if cancelled() {
                    return Err(ColorMappingExecutionError::Cancelled);
                }
                output.extend_from_slice(chunk);
            }
            if cancelled() {
                return Err(ColorMappingExecutionError::Cancelled);
            }
            return Ok(output);
        }

        while output.len() < expected {
            if cancelled() {
                return Err(ColorMappingExecutionError::Cancelled);
            }
            let end = output
                .len()
                .saturating_add(CANCEL_POLL_INTERVAL)
                .min(expected);
            output.resize(end, [0.0; COLOR_MAPPING_CHANNELS]);
        }

        let clusters = usize::try_from(parameters.n).expect("validated cluster count");
        let dominance = parameters.dominance / 100.0;
        let mut mapio = [0_usize; MAXN];
        get_cluster_mapping(
            clusters,
            &parameters.target_mean,
            &parameters.target_weight,
            &parameters.source_mean,
            &parameters.source_weight,
            dominance,
            &mut mapio,
        );
        let mut var_ratio = [[0.0; 2]; MAXN];
        for cluster in 0..clusters {
            var_ratio[cluster][0] = if parameters.target_var[cluster][0] > 0.0 {
                parameters.source_var[mapio[cluster]][0] / parameters.target_var[cluster][0]
            } else {
                0.0
            };
            var_ratio[cluster][1] = if parameters.target_var[cluster][1] > 0.0 {
                parameters.source_var[mapio[cluster]][1] / parameters.target_var[cluster][1]
            } else {
                0.0
            };
        }

        // The first pass intentionally writes only L.  This is the same
        // temporary buffer that the native bilateral helper splats and slices.
        for (index, pixel) in input.iter().enumerate() {
            if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
                return Err(ColorMappingExecutionError::Cancelled);
            }
            let lightness = pixel[0];
            let bin = (HISTN as f32 * lightness / LIGHTNESS_MAXIMUM).clamp(0.0, HISTN as f32 - 1.0)
                as usize;
            let target_bin = usize::try_from(parameters.target_hist[bin])
                .expect("validated target histogram bin");
            output[index][0] = 0.5
                * (lightness * (1.0 - equalization)
                    + parameters.source_ihist[target_bin] * equalization
                    - lightness)
                + 50.0;
            output[index][0] = output[index][0].clamp(0.0, LIGHTNESS_MAXIMUM);
        }

        if equalization > 0.001 {
            let sigma_s = BILATERAL_SPATIAL_SIGMA / self.scale;
            let mut bilateral = BilateralGrid::new(width, height, sigma_s, BILATERAL_RANGE_SIGMA)
                .map_err(map_bilateral_error)?;
            bilateral
                .splat_with_cancel(&output, &mut cancelled)
                .map_err(map_bilateral_error)?;
            bilateral
                .blur_with_cancel(&mut cancelled)
                .map_err(map_bilateral_error)?;
            bilateral
                .slice_in_place_with_cancel(&mut output, -1.0, &mut cancelled)
                .map_err(map_bilateral_error)?;
        }

        for (index, pixel) in input.iter().enumerate() {
            if index % CANCEL_POLL_INTERVAL == 0 && cancelled() {
                return Err(ColorMappingExecutionError::Cancelled);
            }
            output[index][0] =
                (2.0 * (output[index][0] - 50.0) + pixel[0]).clamp(0.0, LIGHTNESS_MAXIMUM);
            let weights = get_clusters(pixel, clusters, &parameters.target_mean);
            output[index][1] = 0.0;
            output[index][2] = 0.0;
            for cluster in 0..clusters {
                output[index][1] += weights[cluster]
                    * ((pixel[1] - parameters.target_mean[cluster][0]) * var_ratio[cluster][0]
                        + parameters.source_mean[mapio[cluster]][0]);
                output[index][2] += weights[cluster]
                    * ((pixel[2] - parameters.target_mean[cluster][1]) * var_ratio[cluster][1]
                        + parameters.source_mean[mapio[cluster]][1]);
            }
            output[index][3] = pixel[3];
            // Alpha is native pass-through and may remain non-finite.
            for (channel, value) in output[index].iter().copied().enumerate().take(3) {
                if !value.is_finite() {
                    return Err(ColorMappingExecutionError::NonFiniteOutput {
                        pixel: index,
                        channel: channel_from_index(channel),
                    });
                }
            }
        }
        if cancelled() {
            return Err(ColorMappingExecutionError::Cancelled);
        }
        Ok(output)
    }

    fn execution_dimensions(
        &self,
        actual: usize,
    ) -> Result<(usize, usize, usize), ColorMappingExecutionError> {
        let (width, height, expected) = self.resolved_dimensions()?;
        if expected != actual {
            return Err(ColorMappingExecutionError::DimensionsMismatch { expected, actual });
        }
        Ok((width, height, expected))
    }

    fn resolved_dimensions(&self) -> Result<(usize, usize, usize), ColorMappingExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count())
            .map_err(|_| ColorMappingExecutionError::SizeOverflow)?;
        let width = usize::try_from(self.dimensions.width())
            .map_err(|_| ColorMappingExecutionError::SizeOverflow)?;
        let height = usize::try_from(self.dimensions.height())
            .map_err(|_| ColorMappingExecutionError::SizeOverflow)?;
        Ok((width, height, expected))
    }

    fn required_execution_bytes(
        &self,
        width: usize,
        height: usize,
        expected: usize,
        complete: bool,
        equalization: f32,
    ) -> Result<usize, ColorMappingExecutionError> {
        let output_bytes = expected
            .checked_mul(size_of::<ColorMappingPixel>())
            .ok_or(ColorMappingExecutionError::SizeOverflow)?;
        if complete && equalization > 0.001 {
            let sigma_s = BILATERAL_SPATIAL_SIGMA / self.scale;
            let bilateral_bytes =
                BilateralGrid::required_memory_bytes(width, height, sigma_s, BILATERAL_RANGE_SIGMA)
                    .map_err(map_bilateral_error)?;
            output_bytes
                .checked_add(bilateral_bytes)
                .ok_or(ColorMappingExecutionError::SizeOverflow)
        } else {
            Ok(output_bytes)
        }
    }
}

fn validate_execution_input_with_cancel<F: FnMut() -> bool>(
    input: &[ColorMappingPixel],
    cancelled: &mut F,
) -> Result<(), ColorMappingExecutionError> {
    for (chunk_index, chunk) in input.chunks(CANCEL_POLL_INTERVAL).enumerate() {
        if cancelled() {
            return Err(ColorMappingExecutionError::Cancelled);
        }
        let chunk_start = chunk_index * CANCEL_POLL_INTERVAL;
        for (offset, values) in chunk.iter().enumerate() {
            // Native mapping computes only Lab L/a/b and copies alpha through.
            for (channel, value) in values.iter().copied().enumerate().take(3) {
                if !value.is_finite() {
                    return Err(ColorMappingExecutionError::NonFiniteInput {
                        pixel: chunk_start + offset,
                        channel: channel_from_index(channel),
                    });
                }
            }
        }
    }
    if cancelled() {
        Err(ColorMappingExecutionError::Cancelled)
    } else {
        Ok(())
    }
}

const fn map_bilateral_error(error: BilateralError) -> ColorMappingExecutionError {
    match error {
        BilateralError::SizeOverflow => ColorMappingExecutionError::SizeOverflow,
        BilateralError::AllocationFailed { required_bytes } => {
            ColorMappingExecutionError::AllocationFailed { required_bytes }
        }
        BilateralError::Cancelled => ColorMappingExecutionError::Cancelled,
        BilateralError::NonFiniteLightness { pixel } => {
            ColorMappingExecutionError::NonFiniteInput {
                pixel,
                channel: ColorMappingChannel::Lightness,
            }
        }
        BilateralError::NonFiniteOutput { pixel } => ColorMappingExecutionError::NonFiniteOutput {
            pixel,
            channel: ColorMappingChannel::Lightness,
        },
        BilateralError::InvalidParameter(name) => {
            ColorMappingExecutionError::InvalidBilateralParameter(name)
        }
        BilateralError::InvalidDimensions => {
            ColorMappingExecutionError::InvalidBilateralParameter("dimensions")
        }
        BilateralError::BufferShape { .. } => {
            ColorMappingExecutionError::InvalidBilateralParameter("buffer shape")
        }
    }
}

fn get_cluster_mapping(
    clusters: usize,
    input_mean: &[[f32; 2]; MAXN],
    input_weight: &[f32; MAXN],
    output_mean: &[[f32; 2]; MAXN],
    output_weight: &[f32; MAXN],
    dominance: f32,
    mapio: &mut [usize; MAXN],
) {
    const WEIGHT_SCALE: f32 = 10_000.0;
    for input_cluster in 0..clusters {
        let mut minimum_distance = f32::MAX;
        for output_cluster in 0..clusters {
            let color_distance = (output_mean[output_cluster][0] - input_mean[input_cluster][0])
                * (output_mean[output_cluster][0] - input_mean[input_cluster][0])
                + (output_mean[output_cluster][1] - input_mean[input_cluster][1])
                    * (output_mean[output_cluster][1] - input_mean[input_cluster][1]);
            let weight_distance = WEIGHT_SCALE
                * (output_weight[output_cluster] - input_weight[input_cluster])
                * (output_weight[output_cluster] - input_weight[input_cluster]);
            let distance = color_distance * (1.0 - dominance) + weight_distance * dominance;
            if distance < minimum_distance {
                minimum_distance = distance;
                mapio[input_cluster] = output_cluster;
            }
        }
    }
}

fn get_clusters(
    pixel: &ColorMappingPixel,
    clusters: usize,
    mean: &[[f32; 2]; MAXN],
) -> [f32; MAXN] {
    let mut weights = [0.0; MAXN];
    let mut minimum_distance = f32::MAX;
    for cluster in 0..clusters {
        let distance = (pixel[1] - mean[cluster][0]) * (pixel[1] - mean[cluster][0])
            + (pixel[2] - mean[cluster][1]) * (pixel[2] - mean[cluster][1]);
        weights[cluster] = if distance > 1.0e-6 {
            1.0 / distance
        } else {
            -1.0
        };
        if distance < minimum_distance {
            minimum_distance = distance;
        }
    }
    if minimum_distance < 1.0e-6 {
        for weight in weights.iter_mut().take(clusters) {
            *weight = if *weight < 0.0 { 1.0 } else { 0.0 };
        }
    }
    let sum = weights[..clusters].iter().sum::<f32>();
    if sum > 0.0 {
        for weight in weights.iter_mut().take(clusters) {
            *weight /= sum;
        }
    }
    weights
}

struct Decoder<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Decoder<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.bytes_4())
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.bytes_4())
    }

    fn bytes_4(&mut self) -> [u8; 4] {
        let end = self.offset + size_of::<f32>();
        let bytes = self.bytes[self.offset..end]
            .try_into()
            .expect("payload length was checked before decoding");
        self.offset = end;
        bytes
    }
}

fn push_i32(bytes: &mut Vec<u8>, value: i32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
