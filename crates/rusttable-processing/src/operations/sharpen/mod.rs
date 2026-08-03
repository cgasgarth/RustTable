#![expect(
    clippy::suboptimal_flops,
    reason = "Native Sharpen arithmetic order is preserved for IEEE-754 parity."
)]

//! Source-faithful CPU Sharpen port for four-channel D50 Lab frames.
//!
//! This leaf is ported from `src/iop/sharpen.c`.  The canonical operation
//! registry, pixelpipe dispatch, GPU implementation, and GTK module remain
//! integration responsibilities outside this file.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::fmt;

use rusttable_color::ColorEncoding;

use crate::descriptor::{
    AlphaPolicy, CapabilityContract, DescriptorId, ImagePredicate, InputOutputContract,
    MaskBlendContract, MigrationContract, NonFinitePolicy, OperationDescriptor, OperationFlags,
    ParameterDefault, ParameterDescriptor, ParameterKind, ParameterRole, RoiKind, TilingContract,
};
use crate::operations::{OperationExecutionError, ReconstructionBudget};
use crate::{FiniteF32, RasterDimensions, RgbChannel};

pub mod source_map;
pub mod tiling;

/// Operation-local source fixture for the native v1 default payload.
pub const DEFAULT_V1_FIXTURE: &str = include_str!("fixtures/default_v1.hex");

/// Stable compatibility identity of the retained native module.
pub const SHARPEN_COMPATIBILITY_ID: &str = "sharpen";
/// Stable Rust operation identity used by the registry and edit graph.
pub const SHARPEN_RUST_ID: &str = "rusttable.sharpen";
/// Native parameter schema version from `DT_MODULE_INTROSPECTION(1, ...)`.
pub const SHARPEN_SCHEMA_VERSION: u16 = 1;
/// Three native `float` fields in declaration order.
pub const SHARPEN_PARAMETER_BYTES: usize = 12;
/// Native integer cap on the sampled blur radius.
pub const SHARPEN_MAXR: u32 = 12;
/// `commit_params` expands the user radius to fit 2.5 sigma inside the mask.
pub const SHARPEN_COMMIT_RADIUS_SCALE: f32 = 2.5;
pub const SHARPEN_DEFAULT_RADIUS: f32 = 2.0;
pub const SHARPEN_DEFAULT_AMOUNT: f32 = 0.5;
pub const SHARPEN_DEFAULT_THRESHOLD: f32 = 0.5;
pub const SHARPEN_RADIUS_MINIMUM: f32 = 0.0;
pub const SHARPEN_RADIUS_MAXIMUM: f32 = 99.0;
pub const SHARPEN_AMOUNT_MINIMUM: f32 = 0.0;
pub const SHARPEN_AMOUNT_MAXIMUM: f32 = 2.0;
pub const SHARPEN_THRESHOLD_MINIMUM: f32 = 0.0;
pub const SHARPEN_THRESHOLD_MAXIMUM: f32 = 100.0;
/// The native module consumes four-channel Lab pixels: L, a, b, and alpha.
pub const SHARPEN_CHANNELS: usize = 4;
/// Human-readable source color boundary for the canonical descriptor.
pub const SHARPEN_INPUT_ENCODING: &str = "Lab D50";

/// Current native parameter payload in exact declaration order.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenParametersV1 {
    pub radius: f32,
    pub amount: f32,
    pub threshold: f32,
}

const _: () = assert!(std::mem::size_of::<SharpenParametersV1>() == SHARPEN_PARAMETER_BYTES);

impl SharpenParametersV1 {
    #[must_use]
    pub const fn new(radius: f32, amount: f32, threshold: f32) -> Self {
        Self {
            radius,
            amount,
            threshold,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            SHARPEN_DEFAULT_RADIUS,
            SHARPEN_DEFAULT_AMOUNT,
            SHARPEN_DEFAULT_THRESHOLD,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; SHARPEN_PARAMETER_BYTES] {
        let mut bytes = [0; SHARPEN_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.radius.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.amount.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.threshold.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, SharpenCodecError> {
        if bytes.len() != SHARPEN_PARAMETER_BYTES {
            return Err(SharpenCodecError::InvalidLength {
                expected: SHARPEN_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let read = |start| {
            f32::from_le_bytes(
                bytes[start..start + std::mem::size_of::<f32>()]
                    .try_into()
                    .expect("validated parameter byte range"),
            )
        };
        let parameters = Self::new(read(0), read(4), read(8));
        parameters
            .validate_finite()
            .map_err(SharpenCodecError::Parameters)?;
        Ok(parameters)
    }

    fn validate_finite(self) -> Result<(), SharpenParameterError> {
        for (name, value) in [
            ("radius", self.radius),
            ("amount", self.amount),
            ("threshold", self.threshold),
        ] {
            FiniteF32::new(value).map_err(|_| SharpenParameterError::NonFinite(name))?;
        }
        Ok(())
    }
}

/// Typed current history with byte-preserving retention for unknown versions.
#[derive(Debug, Clone, PartialEq)]
pub enum SharpenHistory {
    V1(SharpenParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl SharpenHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, SharpenCodecError> {
        if version == SHARPEN_SCHEMA_VERSION {
            Ok(Self::V1(SharpenParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => SHARPEN_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    pub const fn current(&self) -> Result<SharpenParametersV1, SharpenCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(SharpenCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharpenCodecError {
    InvalidLength { expected: usize, actual: usize },
    Parameters(SharpenParameterError),
    UnsupportedVersion(u16),
}

impl fmt::Display for SharpenCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "sharpen payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Parameters(error) => write!(formatter, "invalid sharpen parameters: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "sharpen version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for SharpenCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharpenParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for SharpenParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "sharpen {name} is non-finite"),
        }
    }
}

impl std::error::Error for SharpenParameterError {}

/// Finite execution parameters; the native slider ranges remain descriptor metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SharpenConfig {
    radius: FiniteF32,
    amount: FiniteF32,
    threshold: FiniteF32,
}

impl SharpenConfig {
    pub fn new(radius: f32, amount: f32, threshold: f32) -> Result<Self, SharpenParameterError> {
        Self::try_from(SharpenParametersV1::new(radius, amount, threshold))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(SharpenParametersV1::defaults()).expect("sharpen defaults are valid")
    }

    #[must_use]
    pub const fn radius(self) -> f32 {
        self.radius.get()
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.amount.get()
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold.get()
    }

    #[must_use]
    pub const fn parameters(self) -> SharpenParametersV1 {
        SharpenParametersV1::new(self.radius(), self.amount(), self.threshold())
    }

    /// Applies the native `commit_params` radius expansion without quantizing it.
    #[must_use]
    pub const fn commit(self) -> CommittedSharpen {
        CommittedSharpen {
            radius: self.radius() * SHARPEN_COMMIT_RADIUS_SCALE,
            amount: self.amount(),
            threshold: self.threshold(),
        }
    }
}

impl TryFrom<SharpenParametersV1> for SharpenConfig {
    type Error = SharpenParameterError;

    fn try_from(parameters: SharpenParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            radius: finite_parameter("radius", parameters.radius)?,
            amount: finite_parameter("amount", parameters.amount)?,
            threshold: finite_parameter("threshold", parameters.threshold)?,
        })
    }
}

/// Frozen data produced by the native `commit_params` step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CommittedSharpen {
    radius: f32,
    amount: f32,
    threshold: f32,
}

impl CommittedSharpen {
    #[must_use]
    pub const fn radius(self) -> f32 {
        self.radius
    }

    #[must_use]
    pub const fn amount(self) -> f32 {
        self.amount
    }

    #[must_use]
    pub const fn threshold(self) -> f32 {
        self.threshold
    }
}

/// One four-channel D50 Lab sample in native channel order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenPixel {
    channels: [f32; SHARPEN_CHANNELS],
}

impl SharpenPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, alpha: f32) -> Self {
        Self {
            channels: [lightness, a, b, alpha],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; SHARPEN_CHANNELS]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; SHARPEN_CHANNELS] {
        self.channels
    }

    #[must_use]
    pub const fn lightness(self) -> f32 {
        self.channels[0]
    }

    #[must_use]
    pub const fn a(self) -> f32 {
        self.channels[1]
    }

    #[must_use]
    pub const fn b(self) -> f32 {
        self.channels[2]
    }

    #[must_use]
    pub const fn alpha(self) -> f32 {
        self.channels[3]
    }
}

/// Immutable scalar execution plan for one full frame or scale-1 tile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SharpenPlan {
    committed: CommittedSharpen,
    dimensions: RasterDimensions,
    roi_scale: f32,
    piece_iscale: f32,
    radius: u32,
    budget: ReconstructionBudget,
}

impl SharpenPlan {
    /// Plans native radius quantization from `roi_in.scale / piece->iscale`.
    pub fn new(
        config: SharpenConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_iscale: f32,
    ) -> Result<Self, OperationExecutionError> {
        Self::new_with_budget(
            config,
            dimensions,
            roi_scale,
            piece_iscale,
            ReconstructionBudget::default(),
        )
    }

    /// As `new`, with an explicit operation allocation budget for admission tests.
    pub fn new_with_budget(
        config: SharpenConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_iscale: f32,
        budget: ReconstructionBudget,
    ) -> Result<Self, OperationExecutionError> {
        if !roi_scale.is_finite()
            || !piece_iscale.is_finite()
            || roi_scale <= 0.0
            || piece_iscale <= 0.0
        {
            return Err(OperationExecutionError::UnsupportedCapability(
                "sharpen requires finite positive ROI scale and piece iscale",
            ));
        }
        let committed = config.commit();
        let scaled_radius = committed.radius() * roi_scale / piece_iscale;
        if scaled_radius.is_nan() || scaled_radius < 0.0 {
            return Err(OperationExecutionError::UnsupportedCapability(
                "sharpen requires a non-negative scaled radius",
            ));
        }
        let radius = if scaled_radius >= SHARPEN_MAXR as f32 {
            SHARPEN_MAXR
        } else {
            scaled_radius.ceil() as u32
        };
        let plan = Self {
            committed,
            dimensions,
            roi_scale,
            piece_iscale,
            radius,
            budget,
        };
        plan.check_budget()?;
        Ok(plan)
    }

    #[must_use]
    pub const fn committed(self) -> CommittedSharpen {
        self.committed
    }

    #[must_use]
    pub const fn radius(self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn effective_radius(self) -> f32 {
        self.committed.radius() * self.roi_scale / self.piece_iscale
    }

    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }

    /// Returns the padded native kernel allocation with normalized active weights.
    #[must_use]
    pub fn gaussian_weights(self) -> Vec<f32> {
        gaussian_kernel(self.radius, self.effective_radius()).unwrap_or_default()
    }

    /// Executes the native USM equation with deterministic row-major scalar loops.
    pub fn execute(
        &self,
        input: &[SharpenPixel],
    ) -> Result<Vec<SharpenPixel>, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    /// Executes with a bounded cancellation boundary before each output row.
    pub fn execute_with_cancel<F: FnMut() -> bool>(
        &self,
        input: &[SharpenPixel],
        cancelled: F,
    ) -> Result<Vec<SharpenPixel>, OperationExecutionError> {
        self.execute_with_cancel_and_kernel_reservation(input, cancelled, |length| length)
    }

    fn execute_with_cancel_and_kernel_reservation<F, R>(
        &self,
        input: &[SharpenPixel],
        mut cancelled: F,
        kernel_reservation_length: R,
    ) -> Result<Vec<SharpenPixel>, OperationExecutionError>
    where
        F: FnMut() -> bool,
        R: FnOnce(usize) -> usize,
    {
        let (width, height, expected) = self.shape(input)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }
        Self::validate_input(input, width, height, &mut cancelled)?;

        if self.radius == 0
            || width < usize::try_from(2 * self.radius + 1).expect("radius is bounded")
            || height < usize::try_from(2 * self.radius + 1).expect("radius is bounded")
        {
            return clone_pixels(input, expected);
        }

        // Native `process()` already owns the output ROI when its internal
        // buffers are requested. Preserve that pass-through destination before
        // attempting the kernel and per-row scratch allocations: every native
        // resource failure copies the input ROI through and returns success.
        let mut output = clone_pixels(input, expected)?;
        let Some(kernel) = gaussian_kernel_with_reservation(
            self.radius,
            self.effective_radius(),
            kernel_reservation_length,
        ) else {
            return Ok(output);
        };
        let mut temporary = Vec::<f32>::new();
        if temporary.try_reserve_exact(width).is_err() {
            return Ok(output);
        }
        temporary.resize(width, 0.0);

        let radius = usize::try_from(self.radius).expect("radius is bounded");
        let threshold = self.committed.threshold();
        let amount = self.committed.amount();
        for row in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            if row < radius || row >= height - radius {
                continue;
            }

            let start_row = row - radius;
            let end_row = row + radius;
            for column in 0..width {
                let mut sum = 0.0f32;
                for source_row in start_row..=end_row {
                    let weight = kernel[source_row - start_row];
                    sum += weight * input[source_row * width + column].lightness();
                }
                temporary[column] = sum;
            }

            for column in radius..width - radius {
                let mut sum = 0.0f32;
                for source_column in column - radius..=column + radius {
                    sum += kernel[source_column - (column - radius)] * temporary[source_column];
                }
                let index = row * width + column;
                let source = input[index].channels();
                let difference = source[0] - sum;
                let absolute_difference = difference.abs();
                let detail = if absolute_difference > threshold {
                    (absolute_difference - threshold)
                        .max(0.0)
                        .copysign(difference)
                } else {
                    0.0
                };
                let lightness = source[0] + detail * amount;
                if !lightness.is_finite() {
                    return Err(OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: RgbChannel::Red,
                    });
                }
                output[index] =
                    SharpenPixel::from_channels([lightness, source[1], source[2], source[3]]);
            }
        }
        Ok(output)
    }

    fn shape(
        &self,
        input: &[SharpenPixel],
    ) -> Result<(usize, usize, usize), OperationExecutionError> {
        let width = usize::try_from(self.dimensions.width()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        let height = usize::try_from(self.dimensions.height()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.len(),
            }
        })?;
        let expected = width.checked_mul(height).ok_or_else(|| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: self.budget.maximum_bytes(),
            }
        })?;
        if expected != input.len() {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        Ok((width, height, expected))
    }

    fn validate_input<F: FnMut() -> bool>(
        input: &[SharpenPixel],
        width: usize,
        height: usize,
        cancelled: &mut F,
    ) -> Result<(), OperationExecutionError> {
        for row in 0..height {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            for column in 0..width {
                let index = row * width + column;
                for (channel, value) in input[index].channels().into_iter().enumerate() {
                    if !value.is_finite() {
                        return Err(OperationExecutionError::NonFiniteResult {
                            pixel: index,
                            channel: lab_channel(channel),
                        });
                    }
                }
            }
        }
        Ok(())
    }

    fn check_budget(&self) -> Result<(), OperationExecutionError> {
        let expected = usize::try_from(self.dimensions.pixel_count()).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: self.budget.maximum_bytes(),
            }
        })?;
        let output_bytes = expected
            .checked_mul(std::mem::size_of::<SharpenPixel>())
            .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: self.budget.maximum_bytes(),
            })?;
        let width = usize::try_from(self.dimensions.width()).map_err(|_| {
            OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: self.budget.maximum_bytes(),
            }
        })?;
        let height = self.dimensions.height();
        let kernel_width = self.radius.saturating_mul(2).saturating_add(1);
        let identity =
            self.radius == 0 || self.dimensions.width() < kernel_width || height < kernel_width;
        let required = if identity {
            output_bytes
        } else {
            let temporary_bytes =
                width
                    .checked_mul(std::mem::size_of::<f32>())
                    .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
                        required: usize::MAX,
                        budget: self.budget.maximum_bytes(),
                    })?;
            output_bytes
                .checked_add(temporary_bytes)
                .and_then(|bytes| bytes.checked_add(kernel_bytes(self.radius)))
                .ok_or_else(|| OperationExecutionError::MemoryBudgetExceeded {
                    required: usize::MAX,
                    budget: self.budget.maximum_bytes(),
                })?
        };
        if required > self.budget.maximum_bytes() {
            return Err(OperationExecutionError::MemoryBudgetExceeded {
                required,
                budget: self.budget.maximum_bytes(),
            });
        }
        Ok(())
    }
}

/// Canonical CPU-only Sharpen descriptor.
///
/// `overlap_pixels == 0` is intentional: the static field is not used for this
/// neighborhood operation. The executor resolves overlap from the committed
/// radius and immutable snapshot scale context for every node.
#[must_use]
pub fn sharpen_descriptor() -> OperationDescriptor {
    OperationDescriptor {
        id: DescriptorId {
            compatibility_name: SHARPEN_COMPATIBILITY_ID.to_owned(),
            rust_id: SHARPEN_RUST_ID.to_owned(),
            schema_version: SHARPEN_SCHEMA_VERSION,
            parameter_version: SHARPEN_SCHEMA_VERSION,
            implementation_version: 1,
        },
        parameters: vec![
            scalar_parameter(
                "radius",
                f64::from(SHARPEN_RADIUS_MINIMUM),
                f64::from(SHARPEN_RADIUS_MAXIMUM),
                f64::from(SHARPEN_DEFAULT_RADIUS),
                "pixels",
            ),
            scalar_parameter(
                "amount",
                f64::from(SHARPEN_AMOUNT_MINIMUM),
                f64::from(SHARPEN_AMOUNT_MAXIMUM),
                f64::from(SHARPEN_DEFAULT_AMOUNT),
                "factor",
            ),
            scalar_parameter(
                "threshold",
                f64::from(SHARPEN_THRESHOLD_MINIMUM),
                f64::from(SHARPEN_THRESHOLD_MAXIMUM),
                f64::from(SHARPEN_DEFAULT_THRESHOLD),
                "Lab L",
            ),
        ],
        flags: OperationFlags::MULTI_INSTANCE
            .insert(OperationFlags::STYLE_ELIGIBLE)
            .insert(OperationFlags::HISTORY_VISIBLE)
            .insert(OperationFlags::TILEABLE)
            .insert(OperationFlags::DETERMINISTIC_CPU)
            .insert(OperationFlags::COLOR)
            .insert(OperationFlags::MASKS)
            .insert(OperationFlags::BLENDING),
        stage: "frequential-lab-d50".to_owned(),
        roi: RoiKind::Neighborhood,
        tiling: TilingContract {
            overlap_pixels: 0,
            alignment_pixels: tiling::SHARPEN_TILE_ALIGNMENT,
            minimum_tile_edge: 1,
            preferred_tile_edge: 256,
            temporary_multiplier_milli: 2100,
            input_multiplier_milli: 1000,
            output_multiplier_milli: 1000,
        },
        capability: CapabilityContract {
            cpu_supported: true,
            gpu_tier: None,
            required_features: Vec::new(),
            required_formats: vec!["rgba32float".to_owned()],
            deterministic_cpu: true,
            deterministic_gpu: false,
            fallback_to_cpu: true,
            precision: "native scalar f32 Lab D50 unsharp mask".to_owned(),
            modes: vec!["preview".to_owned(), "full".to_owned(), "export".to_owned()],
        },
        io: InputOutputContract {
            input: lab_predicate(),
            output: lab_predicate(),
            derives_output_encoding: false,
        },
        mask_blend: MaskBlendContract {
            consumes_mask: true,
            publishes_mask: false,
            blend_if: true,
            geometry: false,
            analysis: false,
        },
        migration: MigrationContract {
            source_versions: vec![SHARPEN_SCHEMA_VERSION],
            target_version: SHARPEN_SCHEMA_VERSION,
            opaque_unknown_allowed: true,
        },
        ui: None,
    }
}

fn scalar_parameter(
    id: &str,
    minimum: f64,
    maximum: f64,
    default: f64,
    unit: &str,
) -> ParameterDescriptor {
    ParameterDescriptor {
        id: id.to_owned(),
        kind: ParameterKind::Scalar { minimum, maximum },
        default: ParameterDefault::Scalar(default),
        required: false,
        introduced_version: SHARPEN_SCHEMA_VERSION,
        removed_version: None,
        unit: Some(unit.to_owned()),
        step: Some(0.001),
        precision: 3,
        role: ParameterRole::Processing,
        cache_affecting: true,
        animatable: true,
        ui_hint: Some("slider".to_owned()),
        condition: None,
    }
}

fn lab_predicate() -> ImagePredicate {
    ImagePredicate {
        channels: SHARPEN_CHANNELS as u8,
        alpha: AlphaPolicy::Preserve,
        encodings: vec![ColorEncoding::LabD50],
        nonfinite: NonFinitePolicy::Reject,
    }
}

fn finite_parameter(name: &'static str, value: f32) -> Result<FiniteF32, SharpenParameterError> {
    FiniteF32::new(value).map_err(|_| SharpenParameterError::NonFinite(name))
}

fn gaussian_kernel(radius: u32, effective_radius: f32) -> Option<Vec<f32>> {
    gaussian_kernel_with_reservation(radius, effective_radius, |length| length)
}

fn gaussian_kernel_with_reservation<R>(
    radius: u32,
    effective_radius: f32,
    kernel_reservation_length: R,
) -> Option<Vec<f32>>
where
    R: FnOnce(usize) -> usize,
{
    if radius == 0 {
        return Some(vec![1.0, 0.0, 0.0, 0.0]);
    }
    let width = radius.checked_mul(2)?.checked_add(1)?;
    let padded_words = if width & 3 != 0 {
        (width >> 2).checked_add(1)?
    } else {
        width >> 2
    };
    let storage_len = usize::try_from(padded_words.checked_mul(4)?).ok()?;
    let active_len = usize::try_from(width).ok()?;
    // In sharpen.c the unsuffixed 2.5 constants make the coefficient a double.
    // That promotes each repeated f32 scaled-radius result for both ordered
    // multiplications, followed by one conversion for the `float sigma2` assignment.
    let effective_radius = f64::from(effective_radius);
    let sigma2 = ((1.0_f64 / (2.5_f64 * 2.5_f64)) * effective_radius * effective_radius) as f32;
    // Native `expf` still yields a valid flat kernel when sigma2 overflows to
    // +infinity. Only zero and NaN make the source normalization unusable.
    if sigma2.is_nan() || sigma2 <= 0.0 {
        return None;
    }
    let mut kernel = Vec::new();
    let reservation_length = kernel_reservation_length(storage_len);
    kernel.try_reserve_exact(reservation_length).ok()?;
    kernel.resize(storage_len, 0.0);
    let mut weight = 0.0f32;
    let radius_i32 = i32::try_from(radius).ok()?;
    for offset in -radius_i32..=radius_i32 {
        let offset_f = offset as f32;
        let value = (-(offset_f * offset_f) / (2.0f32 * sigma2)).exp();
        let index = usize::try_from(offset + radius_i32).ok()?;
        kernel[index] = value;
        weight += value;
    }
    if !weight.is_finite() || weight <= 0.0 {
        return None;
    }
    for offset in 0..active_len {
        kernel[offset] /= weight;
    }
    Some(kernel)
}

fn kernel_bytes(radius: u32) -> usize {
    let width = radius.saturating_mul(2).saturating_add(1);
    let words = if width & 3 != 0 {
        (width >> 2).saturating_add(1)
    } else {
        width >> 2
    };
    usize::try_from(words)
        .unwrap_or(usize::MAX)
        .saturating_mul(4)
        .saturating_mul(std::mem::size_of::<f32>())
}

fn clone_pixels(
    input: &[SharpenPixel],
    expected: usize,
) -> Result<Vec<SharpenPixel>, OperationExecutionError> {
    let required = expected.saturating_mul(std::mem::size_of::<SharpenPixel>());
    let mut output = Vec::new();
    output
        .try_reserve_exact(expected)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    output.extend_from_slice(input);
    Ok(output)
}

const fn lab_channel(channel: usize) -> RgbChannel {
    match channel {
        0 => RgbChannel::Red,
        1 => RgbChannel::Green,
        _ => RgbChannel::Blue,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_try_reserve_failure_copies_input_roi_and_succeeds() {
        let dimensions = RasterDimensions::new(3, 3).expect("nonzero dimensions");
        let plan = SharpenPlan::new(
            SharpenConfig::new(0.4, 1.0, 0.0).expect("finite parameters"),
            dimensions,
            1.0,
            1.0,
        )
        .expect("valid plan");
        let input = vec![SharpenPixel::new(50.0, -2.0, 3.0, 0.25); 9];
        let mut allocation_attempts = 0;

        let output = plan
            .execute_with_cancel_and_kernel_reservation(
                &input,
                || false,
                |requested| {
                    allocation_attempts += 1;
                    assert_eq!(requested, 4);
                    // The production `try_reserve_exact` call receives this and
                    // deterministically returns `TryReserveError::CapacityOverflow`.
                    usize::MAX
                },
            )
            .expect("native kernel allocation failure copies through");

        assert_eq!(allocation_attempts, 1);
        assert_eq!(output, input);
    }
}
