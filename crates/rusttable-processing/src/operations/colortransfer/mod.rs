#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Transfer arithmetic order is preserved for IEEE-754 parity."
)]

//! Bounded CPU compatibility leaf for Darktable's deprecated color transfer.
//!
//! The exact v1 payload, typed history import, scalar evaluator, and full-frame
//! CPU pixelpipe route are integrated. The deprecated operation's preview
//! acquisition lifecycle, process-global points owner, GPU surface, UI, masks,
//! and outer blending remain explicitly unavailable.
//!
//! Source lineage: `src/iop/colortransfer.c` and `src/common/points.h` from the
//! pinned Darktable baseline. The native operation stores a 2048-entry L
//! histogram and five two-dimensional a/b cluster slots in its parameter
//! payload. Its CPU path performs ten stochastic k-means iterations and then
//! applies the L histogram and weighted a/b cluster transfer in Lab.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::manual_slice_size_calculation,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::type_complexity,
    reason = "the source contract intentionally fixes f32 arithmetic, C casts, and array layout"
)]

use std::cell::RefCell;
use std::fmt;

use rusttable_processing::RasterDimensions;
use rusttable_processing::operations::{OperationExecutionError, ReconstructionBudget};

/// Native compatibility name.
pub const COLORTRANSFER_COMPATIBILITY_ID: &str = "colortransfer";
/// Stable Rust registry identity.
pub const COLORTRANSFER_RUST_ID: &str = "rusttable.colortransfer";
/// Native module-introspection version.
pub const COLORTRANSFER_SCHEMA_VERSION: u16 = 1;
/// Number of entries in the native L histogram.
pub const COLORTRANSFER_HISTOGRAM_BINS: usize = 1 << 11;
/// Maximum number of native a/b clusters.
pub const COLORTRANSFER_MAX_CLUSTERS: usize = 5;
/// Number of Lab storage lanes at the process boundary, including alpha.
pub const COLORTRANSFER_CHANNELS: usize = 4;
/// Native helper stride used by histogram capture and k-means.
pub const COLORTRANSFER_CAPTURE_STRIDE: usize = 3;
/// Native k-means iteration count.
pub const COLORTRANSFER_KMEANS_ITERATIONS: usize = 10;
/// Native fraction of the raster sampled per k-means iteration.
pub const COLORTRANSFER_SAMPLE_FRACTION: f64 = 0.2;
/// Native default cluster count.
pub const COLORTRANSFER_DEFAULT_CLUSTERS: i32 = 3;
/// The bounded leaf has a CPU implementation.
pub const COLORTRANSFER_CPU_SUPPORTED: bool = true;
/// The deprecated native module has no GPU implementation in this leaf.
pub const COLORTRANSFER_GPU_SUPPORTED: bool = false;
/// The deprecated native module has no Rust UI implementation in this leaf.
pub const COLORTRANSFER_UI_SUPPORTED: bool = false;
/// The typed CPU operation is present in the Rust operation registry.
pub const COLORTRANSFER_REGISTERED: bool = true;

const FLAG_OFFSET: usize = 0;
const HISTOGRAM_OFFSET: usize = FLAG_OFFSET + std::mem::size_of::<i32>();
const MEAN_OFFSET: usize =
    HISTOGRAM_OFFSET + COLORTRANSFER_HISTOGRAM_BINS * std::mem::size_of::<f32>();
const VARIANCE_OFFSET: usize =
    MEAN_OFFSET + COLORTRANSFER_MAX_CLUSTERS * 2 * std::mem::size_of::<f32>();
const CLUSTER_COUNT_OFFSET: usize =
    VARIANCE_OFFSET + COLORTRANSFER_MAX_CLUSTERS * 2 * std::mem::size_of::<f32>();

/// Native little-endian parameter payload size for `dt_iop_colortransfer_params_t`.
///
/// The declaration in `src/iop/colortransfer.c` is 8280 bytes: one four-byte
/// flag, `hist[2048]`, five two-component means, five two-component variances,
/// and one four-byte cluster count. The architecture manifest and this codec
/// share that exact declaration.
pub const COLORTRANSFER_NATIVE_PARAMETER_BYTES: usize =
    CLUSTER_COUNT_OFFSET + std::mem::size_of::<i32>();

/// ABI witness for the native declaration.
///
/// `dt_iop_colortransfer_flag_t` is represented as a four-byte C integer on
/// the supported native targets. The arrays have four-byte alignment, so the
/// C layout has no inter-field padding beyond the declared field sizes.
#[repr(C)]
struct NativeColorTransferParameters {
    flag: i32,
    histogram: [f32; COLORTRANSFER_HISTOGRAM_BINS],
    mean: [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
    variance: [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
    clusters: i32,
}

const _: [(); COLORTRANSFER_NATIVE_PARAMETER_BYTES] =
    [(); std::mem::size_of::<NativeColorTransferParameters>()];

/// Native four-lane Lab storage used by `process`.
///
/// The first three lanes are L, a, and b. The fourth lane is alpha and is
/// copied without numerical modification by the apply path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorTransferPixel {
    channels: [f32; COLORTRANSFER_CHANNELS],
}

impl ColorTransferPixel {
    /// Creates one four-lane Lab pixel.
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    /// Creates one pixel from the native lane order `[L, a, b, alpha]`.
    #[must_use]
    pub const fn from_channels(channels: [f32; COLORTRANSFER_CHANNELS]) -> Self {
        Self { channels }
    }

    /// Returns the native lane order `[L, a, b, alpha]`.
    #[must_use]
    pub const fn channels(self) -> [f32; COLORTRANSFER_CHANNELS] {
        self.channels
    }

    /// Returns the L lane.
    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    /// Returns the a lane.
    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    /// Returns the b lane.
    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    /// Returns the alpha lane.
    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

/// Values of `dt_iop_colortransfer_flag_t`, including unknown retained-history
/// values that must survive an opaque round trip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTransferFlag {
    Acquire,
    Acquire2,
    Acquire3,
    Acquired,
    Apply,
    Neutral,
    Unknown(i32),
}

impl ColorTransferFlag {
    /// Decodes the native integer value without discarding unknown states.
    #[must_use]
    pub const fn from_native(value: i32) -> Self {
        match value {
            0 => Self::Acquire,
            1 => Self::Acquire2,
            2 => Self::Acquire3,
            3 => Self::Acquired,
            4 => Self::Apply,
            5 => Self::Neutral,
            other => Self::Unknown(other),
        }
    }

    /// Encodes the native integer value.
    #[must_use]
    pub const fn native_value(self) -> i32 {
        match self {
            Self::Acquire => 0,
            Self::Acquire2 => 1,
            Self::Acquire3 => 2,
            Self::Acquired => 3,
            Self::Apply => 4,
            Self::Neutral => 5,
            Self::Unknown(value) => value,
        }
    }
}

/// Exact native v1 parameter state.
///
/// Histogram and cluster results are part of the persisted native payload;
/// they are not recomputed or normalized during byte decoding.
#[derive(Clone, Debug)]
pub struct ColorTransferParameters {
    flag: ColorTransferFlag,
    histogram: [f32; COLORTRANSFER_HISTOGRAM_BINS],
    mean: [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
    variance: [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
    clusters: i32,
}

impl PartialEq for ColorTransferParameters {
    fn eq(&self, other: &Self) -> bool {
        self.to_bytes() == other.to_bytes()
    }
}

impl Eq for ColorTransferParameters {}

impl std::hash::Hash for ColorTransferParameters {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(&self.to_bytes(), state);
    }
}

impl Default for ColorTransferParameters {
    fn default() -> Self {
        Self {
            flag: ColorTransferFlag::Neutral,
            histogram: [0.0; COLORTRANSFER_HISTOGRAM_BINS],
            mean: [[0.0; 2]; COLORTRANSFER_MAX_CLUSTERS],
            variance: [[0.0; 2]; COLORTRANSFER_MAX_CLUSTERS],
            clusters: COLORTRANSFER_DEFAULT_CLUSTERS,
        }
    }
}

impl ColorTransferParameters {
    /// Decodes the exact native little-endian v1 payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorTransferCodecError> {
        if bytes.len() != COLORTRANSFER_NATIVE_PARAMETER_BYTES {
            return Err(ColorTransferCodecError::InvalidLength {
                expected: COLORTRANSFER_NATIVE_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }

        let mut parameters = Self {
            flag: ColorTransferFlag::from_native(read_i32(bytes, FLAG_OFFSET)),
            ..Self::default()
        };
        for (index, value) in parameters.histogram.iter_mut().enumerate() {
            *value = read_f32(bytes, HISTOGRAM_OFFSET + index * 4);
        }
        for (index, value) in parameters.mean.iter_mut().flatten().enumerate() {
            *value = read_f32(bytes, MEAN_OFFSET + index * 4);
        }
        for (index, value) in parameters.variance.iter_mut().flatten().enumerate() {
            *value = read_f32(bytes, VARIANCE_OFFSET + index * 4);
        }
        parameters.clusters = read_i32(bytes, CLUSTER_COUNT_OFFSET);
        Ok(parameters)
    }

    /// Encodes the exact native little-endian v1 payload.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0_u8; COLORTRANSFER_NATIVE_PARAMETER_BYTES];
        bytes[FLAG_OFFSET..FLAG_OFFSET + 4]
            .copy_from_slice(&self.flag.native_value().to_le_bytes());
        for (index, value) in self.histogram.iter().enumerate() {
            write_f32(&mut bytes, HISTOGRAM_OFFSET + index * 4, *value);
        }
        for (index, value) in self.mean.iter().flatten().enumerate() {
            write_f32(&mut bytes, MEAN_OFFSET + index * 4, *value);
        }
        for (index, value) in self.variance.iter().flatten().enumerate() {
            write_f32(&mut bytes, VARIANCE_OFFSET + index * 4, *value);
        }
        bytes[CLUSTER_COUNT_OFFSET..CLUSTER_COUNT_OFFSET + 4]
            .copy_from_slice(&self.clusters.to_le_bytes());
        bytes
    }

    /// Returns the native flag state.
    #[must_use]
    pub const fn flag(&self) -> ColorTransferFlag {
        self.flag
    }

    /// Returns the native cluster count.
    #[must_use]
    pub const fn clusters(&self) -> i32 {
        self.clusters
    }

    /// Returns the stored inverse-L histogram.
    #[must_use]
    pub const fn histogram(&self) -> &[f32; COLORTRANSFER_HISTOGRAM_BINS] {
        &self.histogram
    }

    /// Returns the stored target cluster means.
    #[must_use]
    pub const fn means(&self) -> &[[f32; 2]; COLORTRANSFER_MAX_CLUSTERS] {
        &self.mean
    }

    /// Returns the stored target cluster standard deviations.
    #[must_use]
    pub const fn variances(&self) -> &[[f32; 2]; COLORTRANSFER_MAX_CLUSTERS] {
        &self.variance
    }

    /// Changes only the native flag to `APPLY` for an explicit apply plan.
    ///
    /// Native acquisition leaves the parameter copy in `ACQUIRE2` while the
    /// pipe-side state becomes `ACQUIRED`; applying a saved target is a later
    /// `APPLY` state transition. The histogram and cluster state is retained
    /// byte-for-byte by this operation-local convenience method.
    #[must_use]
    pub fn for_apply(&self) -> Self {
        let mut parameters = self.clone();
        parameters.flag = ColorTransferFlag::Apply;
        parameters
    }

    /// Captures a target image with the native default cluster count.
    pub fn acquire(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        Self::acquire_with_clusters_and_cancel(
            input,
            dimensions,
            COLORTRANSFER_DEFAULT_CLUSTERS,
            || false,
        )
    }

    /// Captures a target image with the native one-through-five cluster count.
    pub fn acquire_with_clusters(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        clusters: i32,
    ) -> Result<Self, OperationExecutionError> {
        Self::acquire_with_clusters_and_cancel(input, dimensions, clusters, || false)
    }

    /// Captures a target image while polling cancellation.
    pub fn acquire_with_cancel<F: FnMut() -> bool>(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Self, OperationExecutionError> {
        Self::acquire_with_clusters_and_cancel(
            input,
            dimensions,
            COLORTRANSFER_DEFAULT_CLUSTERS,
            cancelled,
        )
    }

    /// Cancellable target capture using the native cluster control.
    ///
    /// No partially acquired parameter state is returned when cancellation is
    /// observed.
    pub fn acquire_with_clusters_and_cancel<F: FnMut() -> bool>(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        clusters: i32,
        cancelled: F,
    ) -> Result<Self, OperationExecutionError> {
        with_points_rng(|rng| {
            Self::acquire_with_clusters_and_cancel_with_rng(
                input, dimensions, clusters, rng, cancelled,
            )
        })
    }

    /// Cancellable target capture using caller-owned native thread state.
    pub fn acquire_with_clusters_and_cancel_with_rng<F: FnMut() -> bool>(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        clusters: i32,
        rng: &mut PointsRng,
        cancelled: F,
    ) -> Result<Self, OperationExecutionError> {
        Self::acquire_with_clusters_and_budget_and_cancel_with_rng(
            input,
            dimensions,
            clusters,
            ReconstructionBudget::default(),
            rng,
            cancelled,
        )
    }

    /// Cancellable target capture using a caller-owned budget and thread state.
    pub fn acquire_with_clusters_and_budget_and_cancel_with_rng<F: FnMut() -> bool>(
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        clusters: i32,
        budget: ReconstructionBudget,
        rng: &mut PointsRng,
        mut cancelled: F,
    ) -> Result<Self, OperationExecutionError> {
        let clusters = valid_cluster_count(clusters)?;
        validate_input(input, dimensions)?;
        check_memory(dimensions, budget)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        let histogram = capture_histogram(input, dimensions, &mut cancelled)?;
        let mut inverted = [0.0_f32; COLORTRANSFER_HISTOGRAM_BINS];
        invert_histogram(&histogram, &mut inverted);
        let (mean, variance) = kmeans(input, dimensions, clusters, rng, &mut cancelled)?;
        Ok(Self {
            // This is the parameter-side state written by native process after
            // the pipe-side state changes to ACQUIRED.
            flag: ColorTransferFlag::Acquire2,
            histogram: inverted,
            mean,
            variance,
            clusters: i32::try_from(clusters).expect("validated cluster count fits i32"),
        })
    }

    /// Creates an explicit CPU plan for the native process states.
    ///
    /// `APPLY` executes the Lab transfer. `ACQUIRE` and every other non-`APPLY`
    /// state use the native pass-through branch when the plan is executed;
    /// preview acquisition is exposed by [`Self::process`]. The registry and
    /// pixelpipe use this immutable plan and do not claim the preview mutation.
    pub fn plan(
        &self,
        dimensions: RasterDimensions,
    ) -> Result<ColorTransferPlan, OperationExecutionError> {
        self.plan_with_budget(dimensions, ReconstructionBudget::default())
    }

    /// Creates an application plan with an explicit allocation budget.
    pub fn plan_with_budget(
        &self,
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<ColorTransferPlan, OperationExecutionError> {
        check_memory(dimensions, budget)?;
        let clusters = if self.flag == ColorTransferFlag::Apply {
            Some(valid_cluster_count(self.clusters)?)
        } else {
            None
        };
        Ok(ColorTransferPlan {
            parameters: self.clone(),
            dimensions,
            clusters,
        })
    }
}

/// Result of one native-style process call.
///
/// The native module has a parameter-side flag and a separate pipe-side copy.
/// Acquisition on the preview pipe changes the former to `ACQUIRE2` while the
/// latter becomes `ACQUIRED`; all other non-`APPLY` states pass the pixels
/// through unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorTransferProcessResult {
    output: Vec<ColorTransferPixel>,
    parameter_flag: ColorTransferFlag,
    pipe_parameters: ColorTransferParameters,
}

impl ColorTransferProcessResult {
    /// Returns the output pixels produced by the native process branch.
    #[must_use]
    pub fn output(&self) -> &[ColorTransferPixel] {
        &self.output
    }

    /// Returns the module-parameter flag after the process call.
    #[must_use]
    pub const fn parameter_flag(&self) -> ColorTransferFlag {
        self.parameter_flag
    }

    /// Returns the pipe-side flag after the process call.
    #[must_use]
    pub const fn pipe_flag(&self) -> ColorTransferFlag {
        self.pipe_parameters.flag()
    }

    /// Returns the pipe-side state, including acquired histogram and clusters.
    #[must_use]
    pub const fn pipe_parameters(&self) -> &ColorTransferParameters {
        &self.pipe_parameters
    }
}

impl ColorTransferParameters {
    /// Executes the native preview/acquisition/pass-through state machine.
    pub fn process(
        &mut self,
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        preview: bool,
    ) -> Result<ColorTransferProcessResult, OperationExecutionError> {
        self.process_with_cancel(input, dimensions, preview, || false)
    }

    /// Executes the native state machine while polling cancellation.
    pub fn process_with_cancel<F: FnMut() -> bool>(
        &mut self,
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        preview: bool,
        cancelled: F,
    ) -> Result<ColorTransferProcessResult, OperationExecutionError> {
        with_points_rng(|rng| {
            self.process_with_budget_and_cancel_with_rng(
                input,
                dimensions,
                preview,
                ReconstructionBudget::default(),
                rng,
                cancelled,
            )
        })
    }

    /// Executes the native state machine with a caller-owned allocation budget.
    pub fn process_with_budget_and_cancel<F: FnMut() -> bool>(
        &mut self,
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        preview: bool,
        budget: ReconstructionBudget,
        cancelled: F,
    ) -> Result<ColorTransferProcessResult, OperationExecutionError> {
        with_points_rng(|rng| {
            self.process_with_budget_and_cancel_with_rng(
                input, dimensions, preview, budget, rng, cancelled,
            )
        })
    }

    /// Executes the native state machine with caller-owned thread RNG state.
    pub fn process_with_budget_and_cancel_with_rng<F: FnMut() -> bool>(
        &mut self,
        input: &[ColorTransferPixel],
        dimensions: RasterDimensions,
        preview: bool,
        budget: ReconstructionBudget,
        rng: &mut PointsRng,
        mut cancelled: F,
    ) -> Result<ColorTransferProcessResult, OperationExecutionError> {
        validate_input(input, dimensions)?;
        check_memory(dimensions, budget)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        let parameter_flag = self.flag;
        if parameter_flag == ColorTransferFlag::Acquire && preview {
            let acquired = Self::acquire_with_clusters_and_budget_and_cancel_with_rng(
                input,
                dimensions,
                self.clusters,
                budget,
                rng,
                &mut cancelled,
            )?;
            let mut pipe_parameters = acquired;
            pipe_parameters.flag = ColorTransferFlag::Acquired;
            let output = copy_passthrough(input, dimensions, &mut cancelled)?;
            self.flag = ColorTransferFlag::Acquire2;
            return Ok(ColorTransferProcessResult {
                output,
                parameter_flag: self.flag,
                pipe_parameters,
            });
        }

        if parameter_flag == ColorTransferFlag::Apply {
            let clusters = valid_cluster_count(self.clusters)?;
            let plan = ColorTransferPlan {
                parameters: self.clone(),
                dimensions,
                clusters: Some(clusters),
            };
            let output = plan.execute_with_cancel_with_rng(input, rng, cancelled)?;
            return Ok(ColorTransferProcessResult {
                output,
                parameter_flag: self.flag,
                pipe_parameters: self.clone(),
            });
        }

        let output = copy_passthrough(input, dimensions, &mut cancelled)?;
        Ok(ColorTransferProcessResult {
            output,
            parameter_flag: self.flag,
            pipe_parameters: self.clone(),
        })
    }
}

/// Errors from the isolated native parameter codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorTransferCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for ColorTransferCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "colortransfer v1 payload has {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl std::error::Error for ColorTransferCodecError {}

/// Checked, bounded CPU application plan for one full-frame Lab raster.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColorTransferPlan {
    parameters: ColorTransferParameters,
    dimensions: RasterDimensions,
    clusters: Option<usize>,
}

impl ColorTransferPlan {
    /// Returns the full-frame dimensions captured by this plan.
    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    /// Applies native L histogram transfer and weighted Gaussian a/b mapping.
    pub fn execute(
        &self,
        input: &[ColorTransferPixel],
    ) -> Result<Vec<ColorTransferPixel>, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Applies the plan while polling cancellation at bounded loop points.
    ///
    /// Histogram capture, L publication, k-means, and a/b publication all stay
    /// local to this call. A cancelled call never returns its partially built
    /// output.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[ColorTransferPixel],
        cancelled: F,
    ) -> Result<Vec<ColorTransferPixel>, OperationExecutionError> {
        with_points_rng(|rng| self.execute_with_cancel_with_rng(input, rng, cancelled))
    }

    /// Applies the plan using caller-owned native thread state.
    pub fn execute_with_cancel_with_rng<F: FnMut() -> bool>(
        &self,
        input: &[ColorTransferPixel],
        rng: &mut PointsRng,
        mut cancelled: F,
    ) -> Result<Vec<ColorTransferPixel>, OperationExecutionError> {
        validate_input(input, self.dimensions)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        if self.parameters.flag != ColorTransferFlag::Apply {
            return copy_passthrough(input, self.dimensions, &mut cancelled);
        }
        let clusters = self
            .clusters
            .expect("APPLY plans always validate their cluster count");

        // Native APPLY captures and inverts the source L histogram first.
        let histogram = capture_histogram(input, self.dimensions, &mut cancelled)?;
        let mut output = Vec::new();
        let pixel_count = input.len();
        output.try_reserve_exact(pixel_count).map_err(|_| {
            OperationExecutionError::AllocationFailed {
                required: pixel_count.saturating_mul(std::mem::size_of::<ColorTransferPixel>()),
            }
        })?;

        // Native writes all L values before allocating and running k-means.
        let width = usize::try_from(self.dimensions.width()).expect("validated width fits usize");
        let height =
            usize::try_from(self.dimensions.height()).expect("validated height fits usize");
        for row in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let start = row * width;
            for pixel in &input[start..start + width] {
                let lightness_bin = histogram_bin(pixel.lightness());
                let output_lightness =
                    self.parameters.histogram[histogram[lightness_bin] as usize].clamp(0.0, 100.0);
                output.push(ColorTransferPixel::new(
                    output_lightness,
                    0.0,
                    0.0,
                    pixel.alpha(),
                ));
            }
        }

        let (source_mean, source_variance) =
            kmeans(input, self.dimensions, clusters, rng, &mut cancelled)?;
        let mapping = cluster_mapping(clusters, &source_mean, &self.parameters.mean);

        // Native then fills a and b from the mapped source clusters and keeps
        // the fourth lane from the input unchanged.
        for row in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let start = row * width;
            for index in start..start + width {
                let pixel = input[index];
                let weights = cluster_weights(pixel, clusters, &source_mean);
                let mut output_a = 0.0_f32;
                let mut output_b = 0.0_f32;
                for cluster in 0..clusters {
                    let target = mapping[cluster];
                    output_a += weights[cluster]
                        * ((pixel.a() - source_mean[cluster][0])
                            * self.parameters.variance[target][0]
                            / source_variance[cluster][0]
                            + self.parameters.mean[target][0]);
                    output_b += weights[cluster]
                        * ((pixel.b() - source_mean[cluster][1])
                            * self.parameters.variance[target][1]
                            / source_variance[cluster][1]
                            + self.parameters.mean[target][1]);
                }
                output[index] = ColorTransferPixel::new(
                    output[index].lightness(),
                    output_a,
                    output_b,
                    pixel.alpha(),
                );
            }
        }
        Ok(output)
    }
}

/// Persistent xorshift128+ state for one native processing thread.
///
/// Darktable initializes one state per thread and does not reseed it for each
/// acquisition or application.  The bounded Rust leaf uses one thread-local
/// instance for its convenience methods; callers that own a worker thread can
/// pass an explicit instance to the `*_with_rng` methods.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointsRng {
    pub state0: u64,
    pub state1: u64,
}

impl PointsRng {
    /// Creates the state native `dt_points_init` assigns to `thread_index`.
    #[must_use]
    pub const fn for_thread(thread_index: u64) -> Self {
        Self {
            state0: 1_u64.wrapping_add(thread_index),
            state1: 2_u64.wrapping_add(thread_index),
        }
    }

    /// Matches `dt_points_get_for` for one native processing thread.
    pub fn next_f32(&mut self) -> f32 {
        let mut first = self.state0;
        let second = self.state1;
        self.state0 = second;
        first ^= first << 23;
        first ^= first >> 17;
        first ^= second;
        first ^= second >> 26;
        self.state1 = first;
        let bits = 0x3f80_0000_u32 | ((self.state0.wrapping_add(self.state1) >> 41) as u32);
        f32::from_bits(bits) - 1.0
    }
}

impl Default for PointsRng {
    fn default() -> Self {
        Self::for_thread(0)
    }
}

thread_local! {
    static NATIVE_POINTS: RefCell<PointsRng> = RefCell::new(PointsRng::default());
}

fn with_points_rng<T>(function: impl FnOnce(&mut PointsRng) -> T) -> T {
    NATIVE_POINTS.with(|state| function(&mut state.borrow_mut()))
}

fn copy_passthrough<F: FnMut() -> bool>(
    input: &[ColorTransferPixel],
    dimensions: RasterDimensions,
    cancelled: &mut F,
) -> Result<Vec<ColorTransferPixel>, OperationExecutionError> {
    let mut output = Vec::new();
    output.try_reserve_exact(input.len()).map_err(|_| {
        OperationExecutionError::AllocationFailed {
            required: input
                .len()
                .saturating_mul(std::mem::size_of::<ColorTransferPixel>()),
        }
    })?;
    let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
    let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
    for row in 0..height {
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        let start = row * width;
        output.extend_from_slice(&input[start..start + width]);
    }
    Ok(output)
}

fn validate_input(
    input: &[ColorTransferPixel],
    dimensions: RasterDimensions,
) -> Result<(), OperationExecutionError> {
    let expected = usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX);
    if input.len() != expected {
        return Err(OperationExecutionError::DimensionsMismatch {
            expected,
            actual: input.len(),
        });
    }
    Ok(())
}

fn check_memory(
    dimensions: RasterDimensions,
    budget: ReconstructionBudget,
) -> Result<(), OperationExecutionError> {
    let pixels = usize::try_from(dimensions.pixel_count()).map_err(|_| {
        OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        }
    })?;
    let required = pixels
        .checked_mul(std::mem::size_of::<ColorTransferPixel>())
        .and_then(|bytes| bytes.checked_add(COLORTRANSFER_HISTOGRAM_BINS * 4))
        .and_then(|bytes| bytes.checked_add(COLORTRANSFER_MAX_CLUSTERS * (2 * 4 + 2 * 4 + 4 + 4)))
        .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: budget.maximum_bytes(),
        })?;
    if required > budget.maximum_bytes() {
        return Err(OperationExecutionError::MemoryBudgetExceeded {
            required,
            budget: budget.maximum_bytes(),
        });
    }
    Ok(())
}

fn valid_cluster_count(value: i32) -> Result<usize, OperationExecutionError> {
    let maximum = i32::try_from(COLORTRANSFER_MAX_CLUSTERS).expect("cluster maximum fits i32");
    if (1..=maximum).contains(&value) {
        Ok(value as usize)
    } else {
        Err(OperationExecutionError::UnsupportedCapability(
            "colortransfer cluster count must be between one and five",
        ))
    }
}

fn histogram_bin(lightness: f32) -> usize {
    (COLORTRANSFER_HISTOGRAM_BINS as f32 * lightness / 100.0)
        .clamp(0.0, (COLORTRANSFER_HISTOGRAM_BINS - 1) as f32) as usize
}

#[derive(Clone, Copy)]
struct CapturedLab {
    lightness: f32,
    a: f32,
    b: f32,
}

/// Reads the native helper's hard-coded three-float view of the process input.
///
/// `process()` later advances by `piece->colors` (four lanes) for application,
/// but `capture_histogram()` and `kmeans()` use `3 * pixel` in the retained C
/// source.  Keeping this raw view is intentional, including the resulting
/// overlap with the fourth lane of four-lane input.
const fn captured_lab(input: &[ColorTransferPixel], pixel_index: usize) -> CapturedLab {
    let base = pixel_index
        .checked_mul(COLORTRANSFER_CAPTURE_STRIDE)
        .expect("validated raster fits the bounded capture view");
    CapturedLab {
        lightness: input[base / COLORTRANSFER_CHANNELS].channels[base % COLORTRANSFER_CHANNELS],
        a: input[(base + 1) / COLORTRANSFER_CHANNELS].channels[(base + 1) % COLORTRANSFER_CHANNELS],
        b: input[(base + 2) / COLORTRANSFER_CHANNELS].channels[(base + 2) % COLORTRANSFER_CHANNELS],
    }
}

fn capture_histogram<F: FnMut() -> bool>(
    input: &[ColorTransferPixel],
    dimensions: RasterDimensions,
    cancelled: &mut F,
) -> Result<[i32; COLORTRANSFER_HISTOGRAM_BINS], OperationExecutionError> {
    let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
    let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
    let mut histogram = [0_i32; COLORTRANSFER_HISTOGRAM_BINS];
    for row in 0..height {
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        let start = row * width;
        for index in start..start + width {
            histogram[histogram_bin(captured_lab(input, index).lightness)] += 1;
        }
    }
    for bin in 1..COLORTRANSFER_HISTOGRAM_BINS {
        histogram[bin] += histogram[bin - 1];
    }
    let total = histogram[COLORTRANSFER_HISTOGRAM_BINS - 1];
    if total == 0 {
        return Ok(histogram);
    }
    for value in &mut histogram {
        *value = ((*value as f32) * (COLORTRANSFER_HISTOGRAM_BINS as f32 / total as f32))
            .clamp(0.0, (COLORTRANSFER_HISTOGRAM_BINS - 1) as f32) as i32;
    }
    Ok(histogram)
}

fn invert_histogram(
    histogram: &[i32; COLORTRANSFER_HISTOGRAM_BINS],
    inverted: &mut [f32; COLORTRANSFER_HISTOGRAM_BINS],
) {
    let mut last = 31_usize;
    for (index, value) in inverted.iter_mut().take(last + 1).enumerate() {
        *value = (100.0_f64 * index as f64 / COLORTRANSFER_HISTOGRAM_BINS as f64) as f32;
    }
    for (index, value) in inverted.iter_mut().enumerate().skip(last + 1) {
        let index = i32::try_from(index).expect("histogram index fits i32");
        for (bin, count) in histogram.iter().enumerate().skip(last) {
            if *count >= index {
                last = bin;
                *value = (100.0_f64 * bin as f64 / COLORTRANSFER_HISTOGRAM_BINS as f64) as f32;
                break;
            }
        }
    }
}

fn kmeans<F: FnMut() -> bool>(
    input: &[ColorTransferPixel],
    dimensions: RasterDimensions,
    clusters: usize,
    rng: &mut PointsRng,
    cancelled: &mut F,
) -> Result<
    (
        [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
        [[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
    ),
    OperationExecutionError,
> {
    let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
    let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
    let samples = (dimensions.pixel_count() as f64 * COLORTRANSFER_SAMPLE_FRACTION) as usize;
    let mut means = [[0.0_f32; 2]; COLORTRANSFER_MAX_CLUSTERS];
    let mut variances = [[0.0_f32; 2]; COLORTRANSFER_MAX_CLUSTERS];
    let mut working_means = [[0.0_f32; 2]; COLORTRANSFER_MAX_CLUSTERS];
    let mut working_variances = [[0.0_f32; 2]; COLORTRANSFER_MAX_CLUSTERS];
    let mut counts = [0_i32; COLORTRANSFER_MAX_CLUSTERS];

    for mean in means.iter_mut().take(clusters) {
        mean[0] = 20.0_f32 - 40.0_f32 * rng.next_f32();
        mean[1] = 20.0_f32 - 40.0_f32 * rng.next_f32();
    }
    for _iteration in 0..COLORTRANSFER_KMEANS_ITERATIONS {
        counts[..clusters].fill(0);
        for _sample in 0..samples {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let row = (rng.next_f32() * height as f32) as usize;
            let column = (rng.next_f32() * width as f32) as usize;
            let pixel = captured_lab(input, row * width + column);
            // The native source repeats the same update inside `for(k = 0;
            // k < n; k++)`. Keep that ordering rather than correcting the
            // deprecated algorithm into one update per sample.
            for _cluster_iteration in 0..clusters {
                let cluster = nearest_cluster(pixel, clusters, &means);
                counts[cluster] += 1;
                working_variances[cluster][0] += pixel.a * pixel.a;
                working_variances[cluster][1] += pixel.b * pixel.b;
                working_means[cluster][0] += pixel.a;
                working_means[cluster][1] += pixel.b;
            }
        }
        for cluster in 0..clusters {
            if counts[cluster] == 0 {
                continue;
            }
            means[cluster][0] = working_means[cluster][0] / counts[cluster] as f32;
            means[cluster][1] = working_means[cluster][1] / counts[cluster] as f32;
            variances[cluster][0] = working_variances[cluster][0] / counts[cluster] as f32
                - means[cluster][0] * means[cluster][0];
            variances[cluster][1] = working_variances[cluster][1] / counts[cluster] as f32
                - means[cluster][1] * means[cluster][1];
            working_means[cluster] = [0.0; 2];
            working_variances[cluster] = [0.0; 2];
        }
    }
    for variance in variances.iter_mut().take(clusters).flatten() {
        // Native wants standard deviations, including its natural NaN result
        // when roundoff makes a variance slightly negative.
        *variance = variance.sqrt();
    }
    Ok((means, variances))
}

fn nearest_cluster(pixel: CapturedLab, clusters: usize, means: &[[f32; 2]; 5]) -> usize {
    let mut minimum_distance = f32::MAX;
    let mut cluster = 0;
    for (index, mean) in means.iter().enumerate().take(clusters) {
        let distance =
            (pixel.a - mean[0]) * (pixel.a - mean[0]) + (pixel.b - mean[1]) * (pixel.b - mean[1]);
        if distance < minimum_distance {
            minimum_distance = distance;
            cluster = index;
        }
    }
    cluster
}

fn cluster_weights(
    pixel: ColorTransferPixel,
    clusters: usize,
    means: &[[f32; 2]; 5],
) -> [f32; COLORTRANSFER_MAX_CLUSTERS] {
    let mut weights = [0.0_f32; COLORTRANSFER_MAX_CLUSTERS];
    let mut minimum_distance = f32::MAX;
    let mut maximum_distance = 0.0_f32;
    for (index, mean) in means.iter().enumerate().take(clusters) {
        let distance = (pixel.a() - mean[0]) * (pixel.a() - mean[0])
            + (pixel.b() - mean[1]) * (pixel.b() - mean[1]);
        weights[index] = distance;
        minimum_distance = minimum_distance.min(distance);
        maximum_distance = maximum_distance.max(distance);
    }
    if maximum_distance - minimum_distance > 0.0 {
        for weight in weights.iter_mut().take(clusters) {
            *weight = (*weight - minimum_distance) / (maximum_distance - minimum_distance);
        }
    }
    let sum: f32 = weights.iter().take(clusters).sum();
    if sum > 0.0 {
        for weight in weights.iter_mut().take(clusters) {
            *weight /= sum;
        }
    }
    weights
}

fn cluster_mapping(
    clusters: usize,
    input_means: &[[f32; 2]; 5],
    target_means: &[[f32; 2]; COLORTRANSFER_MAX_CLUSTERS],
) -> [usize; COLORTRANSFER_MAX_CLUSTERS] {
    let mut mapping = [0_usize; COLORTRANSFER_MAX_CLUSTERS];
    for (input_index, input_mean) in input_means.iter().enumerate().take(clusters) {
        let mut minimum_distance = f32::MAX;
        for (target_index, target_mean) in target_means.iter().enumerate().take(clusters) {
            let distance = (target_mean[0] - input_mean[0]) * (target_mean[0] - input_mean[0])
                + (target_mean[1] - input_mean[1]) * (target_mean[1] - input_mean[1]);
            if distance < minimum_distance {
                minimum_distance = distance;
                mapping[input_index] = target_index;
            }
        }
    }
    mapping
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated payload length"),
    )
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated payload length"),
    )
}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}
