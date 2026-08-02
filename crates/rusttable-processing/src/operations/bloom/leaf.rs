//! Bounded, source-faithful CPU leaf for retained Darktable `src/iop/bloom.c`.
//!
//! Direct numerical lineage also includes `src/common/box_filters.h`,
//! `src/common/box_filters.cc`, `src/common/imagebuf.h`, and
//! `src/common/imagebuf.c`. `data/kernels/bloom.cl` and its program-12
//! registration were inspected, but GPU execution is deliberately unavailable
//! here. This file is path-loaded by the focused integration test; it is not
//! exported through the shared operation module and does not claim production
//! registry, import, pixelpipe, mask/blend, GPU, or GTK routing.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    dead_code,
    reason = "the standalone leaf preserves source f32/int narrowing boundaries and is not exported yet"
)]

use std::{fmt, mem::size_of};

use rusttable_color::ColorEncoding;
use rusttable_processing::{
    RasterDimensions,
    common::box_filters::{
        BOX_ITERATIONS, BoxFilterError, CancellableBoxFilterError, box_mean_with_cancel,
        box_mean_with_cancel_scratch_bytes,
    },
    descriptor::{
        AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
        MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
        ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind,
        TilingContract,
    },
};

pub const BLOOM_COMPATIBILITY_ID: &str = "bloom";
pub const BLOOM_RUST_ID: &str = "rusttable.bloom";
pub const BLOOM_INTROSPECTION_VERSION: u16 = 1;
pub const BLOOM_PARAMETER_BYTES: usize = 3 * size_of::<f32>();
pub const BLOOM_MIGRATION_EDGES: &[(u16, u16)] = &[];

pub const BLOOM_PARAMETER_MINIMUM: f32 = 0.0;
pub const BLOOM_PARAMETER_MAXIMUM: f32 = 100.0;
pub const BLOOM_DEFAULT_SIZE: f32 = 20.0;
pub const BLOOM_DEFAULT_THRESHOLD: f32 = 90.0;
pub const BLOOM_DEFAULT_STRENGTH: f32 = 25.0;

pub const BLOOM_MAXIMUM_RADIUS: u32 = 256;
pub const BLOOM_BOX_ITERATIONS: u32 = BOX_ITERATIONS;
pub const BLOOM_OVERLAP_RADIUS_MULTIPLIER: u32 = 5;
pub const BLOOM_OPENCL_NUM_BUCKETS: u32 = 4;
pub const BLOOM_OPENCL_PROGRAM: u32 = 12;
pub const BLOOM_CPU_TILING_FACTOR_MILLI: u32 = 2_300;
pub const BLOOM_OPENCL_TILING_FACTOR_MILLI: u32 = 3_000;
pub const BLOOM_TILING_MAXBUF_MILLI: u32 = 1_000;
pub const BLOOM_TILING_ALIGNMENT: u32 = 1;
/// Retained `MAX_VECT`, used by `_alloc_scratch_space` for vertical passes.
pub const BLOOM_NATIVE_BOX_FILTER_MAX_VECT: usize = 16;
/// `DT_CACHELINE_BYTES` from `src/common/dttypes.h` on target Apple ARM.
pub const BLOOM_NATIVE_TARGET_CACHELINE_BYTES: usize = 128;
pub const BLOOM_DEFAULT_MEMORY_BUDGET: usize = 512 * 1024 * 1024;

/// Exact current `dt_iop_bloom_params_t` declaration order and scalar types.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomParametersV1 {
    pub size: f32,
    pub threshold: f32,
    pub strength: f32,
}

impl BloomParametersV1 {
    #[must_use]
    pub const fn new(size: f32, threshold: f32, strength: f32) -> Self {
        Self {
            size,
            threshold,
            strength,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            BLOOM_DEFAULT_SIZE,
            BLOOM_DEFAULT_THRESHOLD,
            BLOOM_DEFAULT_STRENGTH,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; BLOOM_PARAMETER_BYTES] {
        let mut bytes = [0; BLOOM_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.size.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.threshold.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.strength.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, BloomCodecError> {
        if bytes.len() != BLOOM_PARAMETER_BYTES {
            return Err(BloomCodecError::InvalidLength {
                expected: BLOOM_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
        ))
    }
}

/// Known v1 bytes are typed without making them executable. Future versions
/// remain opaque so a later migration can inspect their original bytes.
#[derive(Debug, Clone, PartialEq)]
pub enum BloomHistory {
    V1(BloomParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl BloomHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, BloomCodecError> {
        if version == BLOOM_INTROSPECTION_VERSION {
            Ok(Self::V1(BloomParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => BLOOM_INTROSPECTION_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    /// The retained source has no `legacy_params` branch, so current-history
    /// migration is identity-only and unknown versions cannot be guessed.
    pub const fn migrate_to_v1(&self) -> Result<BloomParametersV1, BloomMigrationError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(BloomMigrationError::OpaqueVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for BloomCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "bloom v1 payload has {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl std::error::Error for BloomCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomMigrationError {
    OpaqueVersion(u16),
}

impl fmt::Display for BloomMigrationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpaqueVersion(version) => {
                write!(formatter, "bloom history version {version} is opaque")
            }
        }
    }
}

impl std::error::Error for BloomMigrationError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for BloomParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "bloom parameter {name} is non-finite"),
        }
    }
}

impl std::error::Error for BloomParameterError {}

/// Finite committed values validated separately from byte-preserving history.
/// The introspection range is a UI contract; native `commit_params` copies every
/// finite `f32` without clamping or rejecting values outside that range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BloomConfig {
    size: f32,
    threshold: f32,
    strength: f32,
}

impl TryFrom<BloomParametersV1> for BloomConfig {
    type Error = BloomParameterError;

    fn try_from(parameters: BloomParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            size: finite_parameter("size", parameters.size)?,
            threshold: finite_parameter("threshold", parameters.threshold)?,
            strength: finite_parameter("strength", parameters.strength)?,
        })
    }
}

impl BloomConfig {
    pub fn new(size: f32, threshold: f32, strength: f32) -> Result<Self, BloomParameterError> {
        Self::try_from(BloomParametersV1::new(size, threshold, strength))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(BloomParametersV1::defaults()).expect("native bloom defaults are valid")
    }

    #[must_use]
    pub const fn size(self) -> f32 {
        self.size
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold
    }

    #[must_use]
    pub const fn strength(self) -> f32 {
        self.strength
    }

    #[must_use]
    pub const fn parameters(self) -> BloomParametersV1 {
        BloomParametersV1::new(self.size, self.threshold, self.strength)
    }
}

/// Leaf-local metadata. It is intentionally not inserted into the shared
/// descriptor registry.
#[must_use]
pub fn bloom_descriptor() -> OperationDescriptor {
    let image = ImagePredicate {
        channels: 4,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    };
    OperationDescriptor {
        id: DescriptorId::new(
            BLOOM_COMPATIBILITY_ID,
            BLOOM_RUST_ID,
            BLOOM_INTROSPECTION_VERSION,
            BLOOM_INTROSPECTION_VERSION,
            1,
        )
        .expect("static bloom descriptor identity"),
        parameters: vec![
            scalar_parameter("size", BLOOM_DEFAULT_SIZE),
            scalar_parameter("threshold", BLOOM_DEFAULT_THRESHOLD),
            scalar_parameter("strength", BLOOM_DEFAULT_STRENGTH),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::FULL_IMAGE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::BLENDING),
        stage: "display-referred-lab".to_owned(),
        roi: RoiKind::FullImage,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: BLOOM_TILING_ALIGNMENT,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 300,
            input_multiplier_milli: 1_000,
            output_multiplier_milli: 1_000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: vec!["lab-d50".to_owned()],
            required_formats: vec!["lab-f32x4".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: false,
            precision: "source-ordered scalar f32 CPU".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: image.clone(),
            output: image,
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: false,
            publishes_mask: false,
            blend_if: false,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![BLOOM_INTROSPECTION_VERSION],
            target_version: BLOOM_INTROSPECTION_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar_parameter(id: &str, default: f32) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar {
            minimum: f64::from(BLOOM_PARAMETER_MINIMUM),
            maximum: f64::from(BLOOM_PARAMETER_MAXIMUM),
        },
        default: ParameterDefault::Scalar(f64::from(default)),
        required: false,
        introduced_version: BLOOM_INTROSPECTION_VERSION,
        removed_version: None,
        unit: Some("%".to_owned()),
        step: None,
        precision: 2,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: false,
        ui_hint: None,
        condition: None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomCapabilities {
    pub cpu: bool,
    pub typed_history: bool,
    pub scaled_roi_radius: bool,
    pub allocation_copy_through: bool,
    pub gpu: bool,
    pub tiling_publication: bool,
    pub masks_and_blending: bool,
    pub production_routing: bool,
    pub ui: bool,
}

#[must_use]
pub const fn capabilities() -> BloomCapabilities {
    BloomCapabilities {
        cpu: true,
        typed_history: true,
        scaled_roi_radius: true,
        allocation_copy_through: true,
        gpu: false,
        tiling_publication: false,
        masks_and_blending: false,
        production_routing: false,
        ui: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BloomMemoryBudget {
    maximum_bytes: usize,
}

impl BloomMemoryBudget {
    #[must_use]
    pub const fn new(maximum_bytes: usize) -> Self {
        Self { maximum_bytes }
    }

    #[must_use]
    pub const fn maximum_bytes(self) -> usize {
        self.maximum_bytes
    }
}

impl Default for BloomMemoryBudget {
    fn default() -> Self {
        Self::new(BLOOM_DEFAULT_MEMORY_BUDGET)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomAllocationMode {
    Normal,
    /// Models `dt_iop_alloc_image_buffers` failing for `blurlightness`.
    FailLightnessBuffer,
    /// Models `_alloc_scratch_space` returning null inside `dt_box_mean`.
    FailBoxFilterScratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BloomPublication {
    Filtered,
    CopiedInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BloomError {
    SizeOverflow,
    InvalidScale(&'static str),
    InvalidBoxFilterRadius,
    InvalidParameter(BloomParameterError),
    OpaqueHistory(u16),
    DimensionsMismatch {
        buffer: &'static str,
        expected: usize,
        actual: usize,
    },
    NonFiniteInput {
        pixel: usize,
        channel: usize,
    },
    NonFiniteIntermediate {
        stage: &'static str,
        pixel: usize,
    },
    NonFiniteOutput {
        pixel: usize,
        channel: usize,
    },
    MemoryBudgetExceeded {
        required: usize,
        budget: usize,
    },
    AllocationFailed {
        buffer: &'static str,
        required: usize,
    },
    BoxFilter(BoxFilterError),
    Cancelled,
}

impl fmt::Display for BloomError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeOverflow => formatter.write_str("bloom raster size overflowed"),
            Self::InvalidScale(name) => {
                write!(formatter, "bloom {name} must be positive and finite")
            }
            Self::InvalidBoxFilterRadius => {
                formatter.write_str("bloom box-filter radius is negative or invalid")
            }
            Self::InvalidParameter(error) => error.fmt(formatter),
            Self::OpaqueHistory(version) => {
                write!(formatter, "bloom history version {version} cannot execute")
            }
            Self::DimensionsMismatch {
                buffer,
                expected,
                actual,
            } => write!(
                formatter,
                "bloom {buffer} expected {expected} pixels, got {actual}"
            ),
            Self::NonFiniteInput { pixel, channel } => write!(
                formatter,
                "bloom input channel {channel} at pixel {pixel} is non-finite"
            ),
            Self::NonFiniteIntermediate { stage, pixel } => write!(
                formatter,
                "bloom {stage} intermediate at pixel {pixel} is non-finite"
            ),
            Self::NonFiniteOutput { pixel, channel } => write!(
                formatter,
                "bloom output channel {channel} at pixel {pixel} is non-finite"
            ),
            Self::MemoryBudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "bloom requires {required} bytes; budget is {budget}"
                )
            }
            Self::AllocationFailed { buffer, required } => write!(
                formatter,
                "bloom could not allocate {required} bytes for {buffer}"
            ),
            Self::BoxFilter(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("bloom execution was cancelled"),
        }
    }
}

impl std::error::Error for BloomError {}

#[derive(Debug, Clone, PartialEq)]
pub struct BloomPlan {
    config: BloomConfig,
    dimensions: RasterDimensions,
    roi_scale: f32,
    piece_input_scale: f32,
    radius: u32,
    native_box_scratch_samples_per_worker: usize,
    native_box_scratch_requested_bytes_per_worker: usize,
    sequential_box_scratch_bytes: usize,
    required_memory_bytes: usize,
}

impl BloomPlan {
    pub fn from_history(
        history: &BloomHistory,
        dimensions: RasterDimensions,
    ) -> Result<Self, BloomError> {
        let parameters = history.migrate_to_v1().map_err(|error| match error {
            BloomMigrationError::OpaqueVersion(version) => BloomError::OpaqueHistory(version),
        })?;
        let config = BloomConfig::try_from(parameters).map_err(BloomError::InvalidParameter)?;
        Self::new(config, dimensions)
    }

    pub fn new(config: BloomConfig, dimensions: RasterDimensions) -> Result<Self, BloomError> {
        Self::new_with_scale_and_budget(config, dimensions, 1.0, 1.0, BloomMemoryBudget::default())
    }

    pub fn new_with_scale(
        config: BloomConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_input_scale: f32,
    ) -> Result<Self, BloomError> {
        Self::new_with_scale_and_budget(
            config,
            dimensions,
            roi_scale,
            piece_input_scale,
            BloomMemoryBudget::default(),
        )
    }

    pub fn new_with_scale_and_budget(
        config: BloomConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_input_scale: f32,
        budget: BloomMemoryBudget,
    ) -> Result<Self, BloomError> {
        if !roi_scale.is_finite() || roi_scale <= 0.0 {
            return Err(BloomError::InvalidScale("roi scale"));
        }
        if !piece_input_scale.is_finite() || piece_input_scale <= 0.0 {
            return Err(BloomError::InvalidScale("piece input scale"));
        }
        let width = dimension_width(dimensions)?;
        let height = dimension_height(dimensions)?;
        let pixel_count = pixel_count(dimensions)?;
        let radius = bloom_radius(config.size(), roi_scale, piece_input_scale)?;
        let radius_usize = usize::try_from(radius).map_err(|_| BloomError::SizeOverflow)?;
        let native_box_scratch_samples_per_worker =
            native_box_scratch_samples_per_worker(height, width, 1, radius_usize)?;
        let native_box_scratch_requested_bytes_per_worker =
            native_box_scratch_requested_bytes(native_box_scratch_samples_per_worker)?;
        let sequential_box_scratch_bytes = box_mean_with_cancel_scratch_bytes(
            height,
            width,
            1,
            radius_usize,
            BLOOM_BOX_ITERATIONS,
        )
        .map_err(BloomError::BoxFilter)?;
        let required_memory_bytes = checked_mul(pixel_count, size_of::<[f32; 4]>())?
            .checked_add(checked_mul(pixel_count, size_of::<f32>())?)
            .and_then(|bytes| bytes.checked_add(native_box_scratch_requested_bytes_per_worker))
            .ok_or(BloomError::SizeOverflow)?;
        if required_memory_bytes > budget.maximum_bytes() {
            return Err(BloomError::MemoryBudgetExceeded {
                required: required_memory_bytes,
                budget: budget.maximum_bytes(),
            });
        }
        Ok(Self {
            config,
            dimensions,
            roi_scale,
            piece_input_scale,
            radius,
            native_box_scratch_samples_per_worker,
            native_box_scratch_requested_bytes_per_worker,
            sequential_box_scratch_bytes,
            required_memory_bytes,
        })
    }

    #[must_use]
    pub const fn config(&self) -> BloomConfig {
        self.config
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn roi_scale(&self) -> f32 {
        self.roi_scale
    }

    #[must_use]
    pub const fn piece_input_scale(&self) -> f32 {
        self.piece_input_scale
    }

    #[must_use]
    pub const fn radius(&self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn overlap_pixels(&self) -> u32 {
        BLOOM_OVERLAP_RADIUS_MULTIPLIER * self.radius
    }

    /// Unpadded source-equivalent `_alloc_scratch_space` sample count for one
    /// native worker: `max(N * width, height, 16 * effective_height)`.
    #[must_use]
    pub const fn native_box_scratch_samples_per_worker(&self) -> usize {
        self.native_box_scratch_samples_per_worker
    }

    /// Bytes requested for one retained worker after `dt_alloc_perthread`
    /// rounds the sample footprint to the target's 128-byte cache line.
    #[must_use]
    pub const fn native_box_scratch_requested_bytes_per_worker(&self) -> usize {
        self.native_box_scratch_requested_bytes_per_worker
    }

    /// Actual unpadded scratch bytes requested by the shared sequential Rust
    /// helper. This adaptation is reported separately and is not substituted
    /// for the retained allocator request in the plan budget.
    #[must_use]
    pub const fn sequential_box_scratch_bytes(&self) -> usize {
        self.sequential_box_scratch_bytes
    }

    /// Transactional output, threshold lightness, and one cache-line-rounded
    /// source-equivalent native box-filter worker request.
    #[must_use]
    pub const fn required_memory_bytes(&self) -> usize {
        self.required_memory_bytes
    }

    #[must_use]
    pub fn lightness_scale(&self) -> f32 {
        bloom_lightness_scale(self.config.strength())
    }

    /// Executes into a new private vector. Allocation failure is fail-closed
    /// because there is no pre-existing native destination to copy into.
    pub fn execute(&self, input: &[[f32; 4]]) -> Result<Vec<[f32; 4]>, BloomError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        cancelled: F,
    ) -> Result<Vec<[f32; 4]>, BloomError> {
        self.validate_input(input, &cancelled)?;
        match self.execute_validated(input, BloomAllocationMode::Normal, false, &cancelled)? {
            BloomValidatedOutput::Filtered(output) => Ok(output),
            BloomValidatedOutput::CopyThrough => {
                unreachable!("Vec execution does not enable native copy-through")
            }
        }
    }

    pub fn execute_into(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
    ) -> Result<BloomPublication, BloomError> {
        self.execute_into_with_cancel(input, output, || false)
    }

    /// Keeps `output` unchanged unless a complete filtered result or the
    /// source's primary-allocation copy-through fallback is ready to publish.
    pub fn execute_into_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        cancelled: F,
    ) -> Result<BloomPublication, BloomError> {
        self.execute_into_with_cancel_and_allocation_mode(
            input,
            output,
            BloomAllocationMode::Normal,
            cancelled,
        )
    }

    /// Deterministic operation-local allocation seams preserve two distinct
    /// native failures: the primary lightness allocation copies through, while
    /// box-filter scratch failure silently leaves thresholded lightness
    /// unblurred and proceeds to the screen blend.
    pub fn execute_into_with_cancel_and_allocation_mode<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        output: &mut [[f32; 4]],
        allocation_mode: BloomAllocationMode,
        cancelled: F,
    ) -> Result<BloomPublication, BloomError> {
        let expected = pixel_count(self.dimensions)?;
        if output.len() != expected {
            return Err(BloomError::DimensionsMismatch {
                buffer: "output",
                expected,
                actual: output.len(),
            });
        }
        self.validate_input(input, &cancelled)?;
        let result = self.execute_validated(input, allocation_mode, true, &cancelled)?;
        if cancelled() {
            return Err(BloomError::Cancelled);
        }
        match result {
            BloomValidatedOutput::Filtered(filtered) => {
                output.copy_from_slice(&filtered);
                Ok(BloomPublication::Filtered)
            }
            BloomValidatedOutput::CopyThrough => {
                output.copy_from_slice(input);
                Ok(BloomPublication::CopiedInput)
            }
        }
    }

    fn validate_input<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        cancelled: &F,
    ) -> Result<(), BloomError> {
        let expected = pixel_count(self.dimensions)?;
        if input.len() != expected {
            return Err(BloomError::DimensionsMismatch {
                buffer: "input",
                expected,
                actual: input.len(),
            });
        }
        let width = dimension_width(self.dimensions)?;
        for (pixel, channels) in input.iter().enumerate() {
            if pixel % width == 0 && cancelled() {
                return Err(BloomError::Cancelled);
            }
            for (channel, value) in channels.iter().enumerate() {
                if !value.is_finite() {
                    return Err(BloomError::NonFiniteInput { pixel, channel });
                }
            }
        }
        if cancelled() {
            return Err(BloomError::Cancelled);
        }
        Ok(())
    }

    fn execute_validated<F: Fn() -> bool>(
        &self,
        input: &[[f32; 4]],
        allocation_mode: BloomAllocationMode,
        allow_copy_through: bool,
        cancelled: &F,
    ) -> Result<BloomValidatedOutput, BloomError> {
        if allocation_mode == BloomAllocationMode::FailLightnessBuffer {
            return if allow_copy_through {
                Ok(BloomValidatedOutput::CopyThrough)
            } else {
                Err(BloomError::AllocationFailed {
                    buffer: "thresholded lightness",
                    required: checked_mul(input.len(), size_of::<f32>())?,
                })
            };
        }

        let width = dimension_width(self.dimensions)?;
        let height = dimension_height(self.dimensions)?;
        let glow_bytes = checked_mul(input.len(), size_of::<f32>())?;
        let mut glow = match reserved_vec(input.len(), "thresholded lightness") {
            Ok(buffer) => buffer,
            Err(BloomError::AllocationFailed { .. }) if allow_copy_through => {
                return Ok(BloomValidatedOutput::CopyThrough);
            }
            Err(error) => return Err(error),
        };
        let scale = self.lightness_scale();
        let threshold = self.config.threshold();
        for (pixel, source) in input.iter().enumerate() {
            if pixel % width == 0 && cancelled() {
                return Err(BloomError::Cancelled);
            }
            let lightness = source[0] * scale;
            if !lightness.is_finite() {
                return Err(BloomError::NonFiniteIntermediate {
                    stage: "threshold",
                    pixel,
                });
            }
            glow.push(if lightness > threshold {
                lightness
            } else {
                0.0
            });
        }
        debug_assert_eq!(glow.len() * size_of::<f32>(), glow_bytes);

        if allocation_mode != BloomAllocationMode::FailBoxFilterScratch {
            match box_mean_with_cancel(
                &mut glow,
                height,
                width,
                1,
                usize::try_from(self.radius).map_err(|_| BloomError::SizeOverflow)?,
                BLOOM_BOX_ITERATIONS,
                cancelled,
            ) {
                // `_box_mean` returns immediately when scratch allocation fails;
                // `process` then screen-blends the still-thresholded buffer.
                Ok(())
                | Err(CancellableBoxFilterError::Filter(BoxFilterError::AllocationFailed {
                    ..
                })) => {}
                Err(CancellableBoxFilterError::Cancelled) => {
                    return Err(BloomError::Cancelled);
                }
                Err(CancellableBoxFilterError::Filter(error)) => {
                    return Err(BloomError::BoxFilter(error));
                }
            }
        }

        for (pixel, lightness) in glow.iter().enumerate() {
            if !lightness.is_finite() {
                return Err(BloomError::NonFiniteIntermediate {
                    stage: "box mean",
                    pixel,
                });
            }
        }

        let mut filtered = reserved_vec(input.len(), "transactional output")?;
        for (pixel, (source, blurred_lightness)) in input.iter().zip(glow).enumerate() {
            if pixel % width == 0 && cancelled() {
                return Err(BloomError::Cancelled);
            }
            let lightness = bloom_release_screen_mix(source[0], blurred_lightness);
            if !lightness.is_finite() {
                return Err(BloomError::NonFiniteOutput { pixel, channel: 0 });
            }
            let mut result = *source;
            result[0] = lightness;
            for (channel, value) in result.iter().enumerate() {
                if !value.is_finite() {
                    return Err(BloomError::NonFiniteOutput { pixel, channel });
                }
            }
            filtered.push(result);
        }
        if cancelled() {
            return Err(BloomError::Cancelled);
        }
        Ok(BloomValidatedOutput::Filtered(filtered))
    }
}

#[derive(Debug, PartialEq)]
enum BloomValidatedOutput {
    Filtered(Vec<[f32; 4]>),
    CopyThrough,
}

fn bloom_radius(size: f32, roi_scale: f32, piece_input_scale: f32) -> Result<u32, BloomError> {
    // The selected macOS packaging Release lane keeps the source's float
    // addition before `fmin` promotes to double, then folds the double
    // divide/multiply into one 2.56 factor before truncating to signed `int`.
    let size_plus_one = size + 1.0_f32;
    let bounded_size = f64::from(size_plus_one).min(f64::from(100.0_f32));
    let base_radius = bounded_size * 2.56_f64;
    if base_radius < f64::from(i32::MIN) || base_radius > f64::from(i32::MAX) {
        return Err(BloomError::InvalidBoxFilterRadius);
    }
    let base_radius = base_radius as i32;

    // Keep the native signed-radius boundary through the left-to-right float
    // scale, `ceilf`, and upper-only cap. Size -1 therefore remains radius zero,
    // while a negative or otherwise invalid final box-filter radius fails closed
    // instead of being totalized by Rust's saturating float-to-u32 cast.
    let scaled_radius = (base_radius as f32 * roi_scale / piece_input_scale).ceil();
    let capped_radius = scaled_radius.min(BLOOM_MAXIMUM_RADIUS as f32);
    if !capped_radius.is_finite() || capped_radius < 0.0 {
        return Err(BloomError::InvalidBoxFilterRadius);
    }
    u32::try_from(capped_radius as i32).map_err(|_| BloomError::InvalidBoxFilterRadius)
}

fn bloom_lightness_scale(strength: f32) -> f32 {
    // The selected Release compiler output combines reciprocal exp2 with the
    // negated exponent. It keeps the source's float addition and promoted double
    // fmin, folds division by 100 into a double 0.01 multiplication, narrows once
    // at the exp2f boundary, and evaluates positive exp2f directly.
    let strength_plus_one = strength + 1.0_f32;
    let bounded_strength = f64::from(strength_plus_one).min(f64::from(100.0_f32));
    let exponent = (bounded_strength * 0.01_f64) as f32;
    exponent.exp2()
}

fn bloom_release_screen_mix(input_lightness: f32, blurred_lightness: f32) -> f32 {
    // Selected Apple ARM Release compiler output contracts the source screen
    // expression into these two ordered fused operations. This explicit
    // compiler-output adaptation does not rely on `__FAST_MATH__`, which the
    // final `-fno-finite-math-only` option leaves undefined.
    input_lightness
        .mul_add(0.01_f32, -1.0_f32)
        .mul_add(100.0_f32 - blurred_lightness, 100.0_f32)
}

fn native_box_scratch_samples_per_worker(
    height: usize,
    width: usize,
    channels: usize,
    radius: usize,
) -> Result<usize, BloomError> {
    // Retained `_compute_effective_height` starts at two and doubles once for
    // every right shift of `2 * radius + 1`, then clamps to the image height.
    let mut window = checked_mul(radius, 2)?
        .checked_add(1)
        .ok_or(BloomError::SizeOverflow)?;
    let mut effective_height = 2_usize;
    while window > 1 {
        effective_height = checked_mul(effective_height, 2)?;
        window >>= 1;
    }
    effective_height = effective_height.min(height);

    // `_alloc_scratch_space`: max(N * width, height, MAX_VECT * eff_height)
    // floats in each native worker slice, before allocator padding.
    let horizontal_samples = checked_mul(channels, width)?;
    let vertical_samples = checked_mul(BLOOM_NATIVE_BOX_FILTER_MAX_VECT, effective_height)?;
    Ok(horizontal_samples.max(height).max(vertical_samples))
}

fn native_box_scratch_requested_bytes(samples: usize) -> Result<usize, BloomError> {
    let unpadded_bytes = checked_mul(samples, size_of::<f32>())?;
    unpadded_bytes
        .div_ceil(BLOOM_NATIVE_TARGET_CACHELINE_BYTES)
        .checked_mul(BLOOM_NATIVE_TARGET_CACHELINE_BYTES)
        .ok_or(BloomError::SizeOverflow)
}

fn finite_parameter(name: &'static str, value: f32) -> Result<f32, BloomParameterError> {
    if !value.is_finite() {
        return Err(BloomParameterError::NonFinite(name));
    }
    Ok(value)
}

fn dimension_width(dimensions: RasterDimensions) -> Result<usize, BloomError> {
    usize::try_from(dimensions.width()).map_err(|_| BloomError::SizeOverflow)
}

fn dimension_height(dimensions: RasterDimensions) -> Result<usize, BloomError> {
    usize::try_from(dimensions.height()).map_err(|_| BloomError::SizeOverflow)
}

fn pixel_count(dimensions: RasterDimensions) -> Result<usize, BloomError> {
    usize::try_from(dimensions.pixel_count()).map_err(|_| BloomError::SizeOverflow)
}

fn checked_mul(left: usize, right: usize) -> Result<usize, BloomError> {
    left.checked_mul(right).ok_or(BloomError::SizeOverflow)
}

fn reserved_vec<T>(capacity: usize, buffer: &'static str) -> Result<Vec<T>, BloomError> {
    let required = checked_mul(capacity, size_of::<T>())?;
    let mut values = Vec::new();
    values
        .try_reserve_exact(capacity)
        .map_err(|_| BloomError::AllocationFailed { buffer, required })?;
    Ok(values)
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + size_of::<f32>()]
            .try_into()
            .expect("fixed bloom field range"),
    )
}
