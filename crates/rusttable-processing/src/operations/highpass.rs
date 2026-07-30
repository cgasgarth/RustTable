//! Bounded CPU/Lab leaf port of Darktable's `src/iop/highpass.c`.
//!
//! Source lineage: `src/iop/highpass.c`, `src/common/box_filters.h`,
//! `src/common/box_filters.cc`, and `data/kernels/highpass.cl` from the pinned
//! Darktable baseline. This file intentionally stops at the operation-local
//! parameter, tiling, CPU, and backend-contract boundary. Registry, history
//! materialization, GPU dispatch, pixelpipe routing, blending, and GTK
//! integration remain deferred to their owning hubs.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    reason = "the source contract fixes native f32-to-integer and raster arithmetic"
)]

use std::fmt;

#[cfg(not(test))]
use super::{OperationExecutionError, ReconstructionBudget};
#[cfg(not(test))]
use crate::common::box_filters::{
    BOX_ITERATIONS, BoxFilterError, CancellableBoxFilterError, box_mean_with_cancel,
};
#[cfg(not(test))]
use crate::{FiniteF32, RasterDimensions, RgbChannel};
#[cfg(test)]
use rusttable_processing::common::box_filters::{
    BOX_ITERATIONS, BoxFilterError, CancellableBoxFilterError, box_mean_with_cancel,
};
#[cfg(test)]
use rusttable_processing::operations::{OperationExecutionError, ReconstructionBudget};
#[cfg(test)]
use rusttable_processing::{FiniteF32, RasterDimensions, RgbChannel};

/// Native module introspection version from `DT_MODULE_INTROSPECTION(1, ...)`.
pub const HIGHPASS_SCHEMA_VERSION: u16 = 1;
/// The native payload contains two contiguous `float`s.
pub const HIGHPASS_PARAMETER_BYTES: usize = 8;
/// Native maximum radius used by both CPU and GPU paths.
pub const HIGHPASS_MAX_RADIUS: u32 = 16;
/// Native parameter default for `sharpness`.
pub const HIGHPASS_DEFAULT_SHARPNESS: f32 = 50.0;
/// Native parameter default for `contrast`.
pub const HIGHPASS_DEFAULT_CONTRAST: f32 = 50.0;
/// Native UI lower bound for both percentage sliders.
pub const HIGHPASS_PARAMETER_MINIMUM: f32 = 0.0;
/// Native UI upper bound for both percentage sliders.
pub const HIGHPASS_PARAMETER_MAXIMUM: f32 = 100.0;
/// Native slider names, in declaration and UI order.
pub const HIGHPASS_PARAMETER_NAMES: [&str; 2] = ["sharpness", "contrast"];
/// Native tooltip for the sharpness slider.
pub const HIGHPASS_SHARPNESS_TOOLTIP: &str = "the sharpness of highpass filter";
/// Native tooltip for the contrast slider.
pub const HIGHPASS_CONTRAST_TOOLTIP: &str = "the contrast of highpass filter";
/// Native `OpenCL` program index from `init_global`.
pub const HIGHPASS_GPU_PROGRAM: u32 = 4;
/// Native `OpenCL` kernels, in the creation order from `init_global`.
pub const HIGHPASS_GPU_KERNELS: [&str; 4] = [
    "highpass_invert",
    "highpass_hblur",
    "highpass_vblur",
    "highpass_mix",
];
/// GPU is documented here but is not executable until the GPU owner binds it.
pub const HIGHPASS_GPU_EXECUTABLE: bool = false;
/// Native CPU writes zero to the fourth channel.
pub const HIGHPASS_CPU_ZEROES_FOURTH_CHANNEL: bool = true;
/// Native GPU mix preserves the input fourth channel.
pub const HIGHPASS_GPU_PRESERVES_FOURTH_CHANNEL: bool = true;
/// There are no native history migrations before or after version one.
pub const HIGHPASS_MIGRATION_EDGES: &[(u16, u16)] = &[];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighpassParametersV1 {
    pub sharpness: f32,
    pub contrast: f32,
}

impl HighpassParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            sharpness: HIGHPASS_DEFAULT_SHARPNESS,
            contrast: HIGHPASS_DEFAULT_CONTRAST,
        }
    }

    #[must_use]
    pub const fn new(sharpness: f32, contrast: f32) -> Self {
        Self {
            sharpness,
            contrast,
        }
    }

    /// Serializes the exact native field order and little-endian float bytes.
    #[must_use]
    pub fn to_bytes(self) -> [u8; HIGHPASS_PARAMETER_BYTES] {
        let mut bytes = [0_u8; HIGHPASS_PARAMETER_BYTES];
        bytes[..4].copy_from_slice(&self.sharpness.to_le_bytes());
        bytes[4..].copy_from_slice(&self.contrast.to_le_bytes());
        bytes
    }

    /// Decodes a v1 payload without applying UI range limits.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, HighpassCodecError> {
        if bytes.len() != HIGHPASS_PARAMETER_BYTES {
            return Err(HighpassCodecError::InvalidLength {
                expected: HIGHPASS_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let sharpness = f32::from_le_bytes(bytes[..4].try_into().expect("checked sharpness range"));
        let contrast = f32::from_le_bytes(bytes[4..].try_into().expect("checked contrast range"));
        Ok(Self::new(sharpness, contrast))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HighpassHistory {
    V1(HighpassParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighpassCodecError {
    InvalidLength { expected: usize, actual: usize },
}

impl fmt::Display for HighpassCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "highpass payload has {actual} bytes; expected {expected}"
                )
            }
        }
    }
}

impl std::error::Error for HighpassCodecError {}

impl HighpassHistory {
    /// Unknown versions remain byte-for-byte opaque and non-executable.
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, HighpassCodecError> {
        if version == HIGHPASS_SCHEMA_VERSION {
            Ok(Self::V1(HighpassParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => HIGHPASS_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighpassParameterError {
    NonFinite(&'static str),
}

impl fmt::Display for HighpassParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "highpass {name} is non-finite"),
        }
    }
}

impl std::error::Error for HighpassParameterError {}

/// Executable parameters retain every finite history value, including values
/// outside the native UI range. The UI metadata is not an execution clamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HighpassConfig {
    sharpness: FiniteF32,
    contrast: FiniteF32,
}

impl TryFrom<HighpassParametersV1> for HighpassConfig {
    type Error = HighpassParameterError;

    fn try_from(parameters: HighpassParametersV1) -> Result<Self, Self::Error> {
        Ok(Self {
            sharpness: finite_parameter("sharpness", parameters.sharpness)?,
            contrast: finite_parameter("contrast", parameters.contrast)?,
        })
    }
}

impl HighpassConfig {
    pub fn new(sharpness: f32, contrast: f32) -> Result<Self, HighpassParameterError> {
        Self::try_from(HighpassParametersV1::new(sharpness, contrast))
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(HighpassParametersV1::defaults()).expect("highpass defaults are finite")
    }

    #[must_use]
    pub const fn sharpness(self) -> f32 {
        self.sharpness.get()
    }

    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast.get()
    }
}

fn finite_parameter(name: &'static str, value: f32) -> Result<FiniteF32, HighpassParameterError> {
    FiniteF32::new(value).map_err(|_| HighpassParameterError::NonFinite(name))
}

/// Four-channel D50 Lab sample in native channel order: L, a, b, and spare.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighpassPixel {
    channels: [f32; 4],
}

impl HighpassPixel {
    #[must_use]
    pub const fn new(lightness: f32, a: f32, b: f32, fourth: f32) -> Self {
        Self {
            channels: [lightness, a, b, fourth],
        }
    }

    #[must_use]
    pub const fn from_channels(channels: [f32; 4]) -> Self {
        Self { channels }
    }

    #[must_use]
    pub const fn channels(self) -> [f32; 4] {
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
    pub const fn fourth(self) -> f32 {
        self.channels[3]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighpassTiling {
    pub factor: f32,
    pub factor_cl: f32,
    pub maxbuf: f32,
    pub overhead: usize,
    pub overlap: u32,
    pub align: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HighpassPlan {
    config: HighpassConfig,
    dimensions: RasterDimensions,
    radius: u32,
}

impl HighpassPlan {
    pub fn new(
        config: HighpassConfig,
        dimensions: RasterDimensions,
    ) -> Result<Self, OperationExecutionError> {
        Self::new_with_scale(config, dimensions, 1.0, 1.0)
    }

    /// Resolves the native `roi_in->scale / piece->iscale` radius once. A
    /// neighborhood tile may then reuse this plan without recomputing it from
    /// tile dimensions.
    pub fn new_with_scale(
        config: HighpassConfig,
        dimensions: RasterDimensions,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<Self, OperationExecutionError> {
        let radius = resolve_radius(config.sharpness(), roi_scale, piece_scale)?;
        let pixel_count = dimensions_pixel_count(dimensions)?;
        check_budget(pixel_count)?;
        Ok(Self {
            config,
            dimensions,
            radius,
        })
    }

    #[must_use]
    pub const fn radius(&self) -> u32 {
        self.radius
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    /// Returns the exact native tiling factors and Gaussian-equivalent overlap.
    pub fn tiling(
        config: HighpassConfig,
        roi_scale: f32,
        piece_scale: f32,
    ) -> Result<HighpassTiling, OperationExecutionError> {
        let radius = resolve_radius(config.sharpness(), roi_scale, piece_scale)?;
        Ok(HighpassTiling {
            factor: 2.1,
            factor_cl: 3.0,
            maxbuf: 1.0,
            overhead: 0,
            overlap: overlap_for_radius(radius),
            align: 1,
        })
    }

    /// Convenience form of [`Self::tiling`] for a committed plan.
    pub fn tiling_for_plan(&self) -> HighpassTiling {
        HighpassTiling {
            factor: 2.1,
            factor_cl: 3.0,
            maxbuf: 1.0,
            overhead: 0,
            overlap: overlap_for_radius(self.radius),
            align: 1,
        }
    }

    /// CPU execution over the committed full-frame shape.
    pub fn execute(
        &self,
        input: &[HighpassPixel],
        dimensions: RasterDimensions,
    ) -> Result<Vec<HighpassPixel>, OperationExecutionError> {
        self.execute_with_cancel(input, dimensions, || false)
    }

    /// CPU execution with cancellation. Work is kept in private temporaries;
    /// cancellation and all errors therefore publish no partial output.
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[HighpassPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<HighpassPixel>, OperationExecutionError> {
        if dimensions != self.dimensions {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected: dimensions_pixel_count(self.dimensions).unwrap_or(usize::MAX),
                actual: input.len(),
            });
        }
        self.execute_input_dimensions_with_cancel(input, dimensions, cancelled)
    }

    /// CPU execution over an expanded neighborhood tile. The radius is from
    /// the committed source-frame plan, while the box filter shape is the tile.
    pub fn execute_with_input_dimensions(
        &self,
        input: &[HighpassPixel],
        dimensions: RasterDimensions,
    ) -> Result<Vec<HighpassPixel>, OperationExecutionError> {
        self.execute_with_input_dimensions_with_cancel(input, dimensions, || false)
    }

    /// Cancellable expanded-tile execution.
    pub fn execute_with_input_dimensions_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[HighpassPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<HighpassPixel>, OperationExecutionError> {
        self.execute_input_dimensions_with_cancel(input, dimensions, cancelled)
    }

    fn execute_input_dimensions_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[HighpassPixel],
        dimensions: RasterDimensions,
        cancelled: F,
    ) -> Result<Vec<HighpassPixel>, OperationExecutionError> {
        let expected = dimensions_pixel_count(dimensions)?;
        if input.len() != expected {
            return Err(OperationExecutionError::DimensionsMismatch {
                expected,
                actual: input.len(),
            });
        }
        check_budget(expected)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        let width = usize::try_from(dimensions.width()).expect("validated width fits usize");
        let height = usize::try_from(dimensions.height()).expect("validated height fits usize");
        let mut blurred = try_reserve::<f32>(expected)?;
        for (index, pixel) in input.iter().enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let lightness = pixel.lightness();
            if !lightness.is_finite() {
                return Err(OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: RgbChannel::Red,
                });
            }
            blurred.push(100.0_f32 - lclip(lightness));
        }

        box_mean_with_cancel(
            &mut blurred,
            height,
            width,
            1,
            usize::try_from(self.radius).expect("highpass radius is bounded"),
            BOX_ITERATIONS,
            &cancelled,
        )
        .map_err(box_filter_error)?;
        if cancelled() {
            return Err(OperationExecutionError::Cancelled);
        }

        // Native `_blend` packs the blur at the front of the output and walks
        // backwards twice to avoid clobbering unread values. A private scalar
        // output is the safe equivalent and also handles the native <4-pixel
        // reverse-loop underflow without reproducing its non-terminating loop.
        let contrast_scale = (self.config.contrast() / 100.0_f32) * 7.5_f32 * 0.5_f32;
        let mut output = try_reserve::<HighpassPixel>(expected)?;
        for (index, (pixel, blurred_lightness)) in input.iter().zip(blurred).enumerate() {
            if index % width == 0 && cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let lightness = (blurred_lightness + pixel.lightness()) - 100.0_f32;
            let output_lightness = clamp_lab((lightness * contrast_scale) + 50.0_f32);
            output.push(HighpassPixel::new(output_lightness, 0.0, 0.0, 0.0));
        }
        Ok(output)
    }
}

fn dimensions_pixel_count(dimensions: RasterDimensions) -> Result<usize, OperationExecutionError> {
    usize::try_from(dimensions.pixel_count()).map_err(|_| {
        OperationExecutionError::DimensionsMismatch {
            expected: usize::MAX,
            actual: 0,
        }
    })
}

fn check_budget(pixel_count: usize) -> Result<(), OperationExecutionError> {
    let bytes_per_pixel = std::mem::size_of::<HighpassPixel>() + std::mem::size_of::<f32>();
    let required = pixel_count.checked_mul(bytes_per_pixel).ok_or(
        OperationExecutionError::MemoryBudgetExceeded {
            required: usize::MAX,
            budget: ReconstructionBudget::default().maximum_bytes(),
        },
    )?;
    if required <= ReconstructionBudget::default().maximum_bytes() {
        Ok(())
    } else {
        Err(OperationExecutionError::MemoryBudgetExceeded {
            required,
            budget: ReconstructionBudget::default().maximum_bytes(),
        })
    }
}

fn try_reserve<T>(length: usize) -> Result<Vec<T>, OperationExecutionError> {
    let required = length.saturating_mul(std::mem::size_of::<T>());
    let mut values = Vec::new();
    values
        .try_reserve_exact(length)
        .map_err(|_| OperationExecutionError::AllocationFailed { required })?;
    Ok(values)
}

fn resolve_radius(
    sharpness: f32,
    roi_scale: f32,
    piece_scale: f32,
) -> Result<u32, OperationExecutionError> {
    if !roi_scale.is_finite() || roi_scale <= 0.0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "highpass roi scale must be finite and positive",
        ));
    }
    if !piece_scale.is_finite() || piece_scale <= 0.0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "highpass piece scale must be finite and positive",
        ));
    }

    // `sharpness + 1` is formed as f32, then the unsuffixed C `100.0` in
    // fmin promotes that value to double before the native int truncation.
    let capped = f64::from((sharpness + 1.0_f32).min(100.0_f32));
    let rad = (f64::from(HIGHPASS_MAX_RADIUS) * (capped / 100.0_f64)) as i32;
    if rad < 0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "highpass sharpness produces a negative radius",
        ));
    }

    // `rad * roi_in->scale / piece->iscale` is a native f32 expression and
    // `ceilf` occurs before the integer cap.
    let scaled = (rad as f32 * roi_scale / piece_scale).ceil();
    if !scaled.is_finite() || scaled < 0.0 {
        return Err(OperationExecutionError::UnsupportedCapability(
            "highpass scaled radius is not finite and nonnegative",
        ));
    }
    Ok(HIGHPASS_MAX_RADIUS.min(scaled as u32))
}

fn overlap_for_radius(radius: u32) -> u32 {
    let numerator = radius * (radius + 1) * BOX_ITERATIONS + 2;
    let sigma = (numerator as f32 / 3.0_f32).sqrt();
    (3.0_f32 * sigma).ceil() as u32
}

#[allow(
    clippy::manual_clamp,
    reason = "native LCLIP preserves the bounded comparison ordering"
)]
fn lclip(value: f32) -> f32 {
    if value < 0.0_f32 {
        0.0_f32
    } else if value > 100.0_f32 {
        100.0_f32
    } else {
        value
    }
}

#[allow(
    clippy::manual_clamp,
    reason = "native CLAMPS maps NaN to the low bound instead of returning NaN"
)]
fn clamp_lab(value: f32) -> f32 {
    if value >= 0.0_f32 {
        if value <= 100.0_f32 { value } else { 100.0_f32 }
    } else {
        0.0_f32
    }
}

fn box_filter_error(error: CancellableBoxFilterError) -> OperationExecutionError {
    match error {
        CancellableBoxFilterError::Cancelled => OperationExecutionError::Cancelled,
        CancellableBoxFilterError::Filter(error) => match error {
            BoxFilterError::AllocationFailed { required_bytes } => {
                OperationExecutionError::AllocationFailed {
                    required: required_bytes,
                }
            }
            BoxFilterError::SizeOverflow => OperationExecutionError::MemoryBudgetExceeded {
                required: usize::MAX,
                budget: ReconstructionBudget::default().maximum_bytes(),
            },
            BoxFilterError::BufferShape { expected, actual } => {
                OperationExecutionError::DimensionsMismatch { expected, actual }
            }
            BoxFilterError::NonFiniteInput { sample } => OperationExecutionError::NonFiniteResult {
                pixel: sample,
                channel: RgbChannel::Red,
            },
            BoxFilterError::InvalidDimensions { .. }
            | BoxFilterError::UnsupportedChannels { .. }
            | BoxFilterError::ScratchShape { .. } => {
                OperationExecutionError::UnsupportedCapability(
                    "box mean rejected a validated highpass buffer",
                )
            }
        },
    }
}
