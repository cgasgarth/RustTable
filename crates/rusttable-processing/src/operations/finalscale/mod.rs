//! Final render scaling: typed request solving and deterministic resampling.
//!
//! The registry and render pipeline own activation and wiring. This module
//! owns the operation-specific history DTO, immutable size plan, ROI mapping,
//! and CPU image/mask execution.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::manual_is_multiple_of,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    unused_imports
)]

mod codec;
mod descriptor;
mod geometry;
mod image;
mod resample;

pub use codec::{
    FinalScaleHistory, FinalScaleKernel, FinalScaleParametersV1, RenderQuality, RenderQualityKind,
    RenderSizeRequest, RenderSizeRequestError,
};
pub use descriptor::finalscale_descriptor;
pub use image::{FinalScaleImageError, FinalScaleImageExecution};
pub use resample::{AxisCoefficients, ResampleTap};

use crate::{FiniteF32, LinearRgb, RasterDimensions};
use rusttable_image::Roi;
use sha2::{Digest, Sha256};
use std::fmt;

pub const FINALSCALE_COMPATIBILITY_ID: &str = "finalscale";
pub const FINALSCALE_RUST_ID: &str = "rusttable.finalscale";
pub const FINALSCALE_SCHEMA_VERSION: u16 = 1;
pub const FINALSCALE_PARAMETER_VERSION: u16 = 1;
pub const FINALSCALE_PARAMETER_BYTES: usize = 4;
pub const FINALSCALE_MAX_DIMENSION: u32 = 1 << 30;
pub const FINALSCALE_DEFAULT_MAX_PIXELS: u64 = 1 << 30;
pub const FINALSCALE_DEFAULT_MAX_BYTES: usize = 8 * 1024 * 1024 * 1024;

/// Checked resource limits applied before a final-size plan is published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FinalScaleLimits {
    max_pixels: u64,
    max_bytes: usize,
}

impl FinalScaleLimits {
    #[must_use]
    pub const fn new(max_pixels: u64, max_bytes: usize) -> Self {
        Self {
            max_pixels,
            max_bytes,
        }
    }

    #[must_use]
    pub const fn max_pixels(self) -> u64 {
        self.max_pixels
    }

    #[must_use]
    pub const fn max_bytes(self) -> usize {
        self.max_bytes
    }
}

impl Default for FinalScaleLimits {
    fn default() -> Self {
        Self::new(FINALSCALE_DEFAULT_MAX_PIXELS, FINALSCALE_DEFAULT_MAX_BYTES)
    }
}

/// Operation-specific options consumed by [`FinalScalePlan`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FinalScaleConfig {
    request: RenderSizeRequest,
    quality: RenderQuality,
    allow_upscale: bool,
    limits: FinalScaleLimits,
}

impl FinalScaleConfig {
    #[must_use]
    pub fn new(request: RenderSizeRequest) -> Self {
        Self {
            request,
            quality: RenderQuality::default(),
            allow_upscale: false,
            limits: FinalScaleLimits::default(),
        }
    }

    #[must_use]
    pub fn with_quality(mut self, quality: RenderQuality) -> Self {
        self.quality = quality;
        self
    }

    #[must_use]
    pub const fn with_upscale(mut self, allow_upscale: bool) -> Self {
        self.allow_upscale = allow_upscale;
        self
    }

    #[must_use]
    pub const fn with_limits(mut self, limits: FinalScaleLimits) -> Self {
        self.limits = limits;
        self
    }

    #[must_use]
    pub const fn request(&self) -> &RenderSizeRequest {
        &self.request
    }

    #[must_use]
    pub const fn quality(&self) -> RenderQuality {
        self.quality
    }

    #[must_use]
    pub const fn allow_upscale(&self) -> bool {
        self.allow_upscale
    }

    #[must_use]
    pub const fn limits(&self) -> FinalScaleLimits {
        self.limits
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalScalePlanError {
    InvalidRequest(RenderSizeRequestError),
    OutputTooLarge { pixels: u64, limit: u64 },
    OutputTooManyBytes { bytes: usize, limit: usize },
    ArithmeticOverflow,
    InvalidRoi,
}

impl fmt::Display for FinalScalePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(error) => write!(formatter, "invalid finalscale request: {error}"),
            Self::OutputTooLarge { pixels, limit } => {
                write!(
                    formatter,
                    "finalscale output has {pixels} pixels; limit is {limit}"
                )
            }
            Self::OutputTooManyBytes { bytes, limit } => {
                write!(
                    formatter,
                    "finalscale output needs {bytes} bytes; limit is {limit}"
                )
            }
            Self::ArithmeticOverflow => formatter.write_str("finalscale arithmetic overflowed"),
            Self::InvalidRoi => formatter.write_str("finalscale ROI is outside the planned frame"),
        }
    }
}

impl std::error::Error for FinalScalePlanError {}

impl From<RenderSizeRequestError> for FinalScalePlanError {
    fn from(error: RenderSizeRequestError) -> Self {
        Self::InvalidRequest(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct FinalScalePlan {
    config: FinalScaleConfig,
    source_dimensions: RasterDimensions,
    output_dimensions: RasterDimensions,
    scale_x: f64,
    scale_y: f64,
    source_roi: Roi,
    output_roi: Roi,
    coefficients_x: AxisCoefficients,
    coefficients_y: AxisCoefficients,
    identity: [u8; 32],
    upscale_suppressed: bool,
}

impl FinalScalePlan {
    /// Builds a plan with the default quality, limits, and no-upscale policy.
    pub fn new(
        source_dimensions: RasterDimensions,
        request: RenderSizeRequest,
    ) -> Result<Self, FinalScalePlanError> {
        Self::from_config(source_dimensions, FinalScaleConfig::new(request))
    }

    /// Builds a plan from all finalscale request options.
    pub fn from_config(
        source_dimensions: RasterDimensions,
        config: FinalScaleConfig,
    ) -> Result<Self, FinalScalePlanError> {
        let resolved = geometry::resolve_dimensions(
            source_dimensions,
            config.request(),
            config.allow_upscale(),
            config.limits(),
        )?;
        let source_roi = full_roi(source_dimensions)?;
        let output_roi = full_roi(resolved.output_dimensions)?;
        let coefficients_x = resample::AxisCoefficients::new(
            source_dimensions.width(),
            resolved.output_dimensions.width(),
            config.quality().kernel(),
        );
        let coefficients_y = resample::AxisCoefficients::new(
            source_dimensions.height(),
            resolved.output_dimensions.height(),
            config.quality().kernel(),
        );
        let identity = plan_identity(
            source_dimensions,
            resolved.output_dimensions,
            &config,
            resolved.upscale_suppressed,
        );
        Ok(Self {
            config,
            source_dimensions,
            output_dimensions: resolved.output_dimensions,
            scale_x: f64::from(resolved.output_dimensions.width())
                / f64::from(source_dimensions.width()),
            scale_y: f64::from(resolved.output_dimensions.height())
                / f64::from(source_dimensions.height()),
            source_roi,
            output_roi,
            coefficients_x,
            coefficients_y,
            identity,
            upscale_suppressed: resolved.upscale_suppressed,
        })
    }

    /// Convenience constructor for a non-default quality policy.
    pub fn with_quality(
        source_dimensions: RasterDimensions,
        request: RenderSizeRequest,
        quality: RenderQuality,
    ) -> Result<Self, FinalScalePlanError> {
        Self::from_config(
            source_dimensions,
            FinalScaleConfig::new(request).with_quality(quality),
        )
    }

    #[must_use]
    pub const fn config(&self) -> &FinalScaleConfig {
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
    pub const fn scale_x(&self) -> f64 {
        self.scale_x
    }

    #[must_use]
    pub const fn scale_y(&self) -> f64 {
        self.scale_y
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
    pub const fn quality(&self) -> RenderQuality {
        self.config.quality()
    }

    #[must_use]
    pub const fn kernel(&self) -> FinalScaleKernel {
        self.quality().kernel()
    }

    #[must_use]
    pub const fn is_identity(&self) -> bool {
        self.source_dimensions.width() == self.output_dimensions.width()
            && self.source_dimensions.height() == self.output_dimensions.height()
    }

    #[must_use]
    pub const fn upscale_suppressed(&self) -> bool {
        self.upscale_suppressed
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn coefficients_x(&self) -> &AxisCoefficients {
        &self.coefficients_x
    }

    #[must_use]
    pub const fn coefficients_y(&self) -> &AxisCoefficients {
        &self.coefficients_y
    }

    pub fn memory_estimate_bytes(&self, channels: usize) -> Result<usize, FinalScalePlanError> {
        let pixels = usize::try_from(self.output_dimensions.pixel_count())
            .map_err(|_| FinalScalePlanError::ArithmeticOverflow)?;
        pixels
            .checked_mul(channels)
            .and_then(|samples| samples.checked_mul(std::mem::size_of::<f32>()))
            .ok_or(FinalScalePlanError::ArithmeticOverflow)
    }

    /// Maps an output tile to the source tile required by the resampler.
    pub fn modify_roi_in(&self, output: Roi) -> Result<Roi, FinalScalePlanError> {
        geometry::roi_in(self, output)
    }

    /// Maps a source tile to the output region it can contribute to.
    pub fn modify_roi_out(&self, input: Roi) -> Result<Roi, FinalScalePlanError> {
        geometry::roi_out(self, input)
    }

    pub fn roi_in(&self, output: Roi) -> Result<Roi, FinalScalePlanError> {
        self.modify_roi_in(output)
    }

    pub fn roi_out(&self, input: Roi) -> Result<Roi, FinalScalePlanError> {
        self.modify_roi_out(input)
    }

    pub fn forward_transform(&self, points: &mut [f32]) -> Result<(), FinalScalePlanError> {
        geometry::transform_points(points, self.scale_x, self.scale_y)
    }

    pub fn back_transform(&self, points: &mut [f32]) -> Result<(), FinalScalePlanError> {
        geometry::transform_points(points, 1.0 / self.scale_x, 1.0 / self.scale_y)
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<FinalScaleExecution, FinalScaleExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<FinalScaleExecution, FinalScaleExecutionError> {
        let packed = input
            .iter()
            .flat_map(|pixel| [pixel.red().get(), pixel.green().get(), pixel.blue().get()])
            .collect::<Vec<_>>();
        let stride = usize::try_from(self.source_dimensions.width())
            .map_err(|_| FinalScaleExecutionError::ArithmeticOverflow)?
            .checked_mul(3)
            .ok_or(FinalScaleExecutionError::ArithmeticOverflow)?;
        let values = self.execute_interleaved_with_cancel(&packed, 3, stride, cancelled)?;
        let mut pixels = Vec::with_capacity(values.len() / 3);
        for (index, channels) in values.as_chunks::<3>().0.iter().enumerate() {
            pixels.push(LinearRgb::new(
                finite_channel(channels[0], index, FinalScaleChannel::Red)?,
                finite_channel(channels[1], index, FinalScaleChannel::Green)?,
                finite_channel(channels[2], index, FinalScaleChannel::Blue)?,
            ));
        }
        Ok(FinalScaleExecution {
            pixels,
            dimensions: self.output_dimensions,
            receipt: FinalScaleReceipt {
                plan_identity: self.identity,
                input_digest: digest_f32(&packed),
                output_digest: digest_f32(&values),
                upscale_suppressed: self.upscale_suppressed,
            },
        })
    }

    pub fn execute_interleaved(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
    ) -> Result<Vec<f32>, FinalScaleExecutionError> {
        self.execute_interleaved_with_cancel(input, channels, stride, || false)
    }

    pub fn execute_interleaved_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        channels: usize,
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, FinalScaleExecutionError> {
        resample::execute(
            self,
            input,
            self.source_roi,
            self.output_roi,
            channels,
            stride,
            cancelled,
        )
    }

    pub fn execute_roi(
        &self,
        input: &[f32],
        input_roi: Roi,
        output_roi: Roi,
        channels: usize,
        stride: usize,
    ) -> Result<Vec<f32>, FinalScaleExecutionError> {
        self.execute_roi_with_cancel(input, input_roi, output_roi, channels, stride, || false)
    }

    pub fn execute_roi_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[f32],
        input_roi: Roi,
        output_roi: Roi,
        channels: usize,
        stride: usize,
        cancelled: F,
    ) -> Result<Vec<f32>, FinalScaleExecutionError> {
        resample::execute(
            self, input, input_roi, output_roi, channels, stride, cancelled,
        )
    }

    pub fn execute_mask(
        &self,
        input: &[f32],
        input_roi: Roi,
        output_roi: Roi,
    ) -> Result<Vec<f32>, FinalScaleExecutionError> {
        self.execute_roi(input, input_roi, output_roi, 1, input_roi.width() as usize)
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| value.clamp(0.0, 1.0))
                    .collect()
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalScaleExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    receipt: FinalScaleReceipt,
}

impl FinalScaleExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn receipt(&self) -> &FinalScaleReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinalScaleReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
    upscale_suppressed: bool,
}

impl FinalScaleReceipt {
    #[must_use]
    pub const fn plan_identity(self) -> [u8; 32] {
        self.plan_identity
    }

    #[must_use]
    pub const fn input_digest(self) -> [u8; 32] {
        self.input_digest
    }

    #[must_use]
    pub const fn output_digest(self) -> [u8; 32] {
        self.output_digest
    }

    #[must_use]
    pub const fn upscale_suppressed(self) -> bool {
        self.upscale_suppressed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinalScaleExecutionError {
    InvalidShape {
        expected: usize,
        actual: usize,
    },
    InvalidStride {
        minimum: usize,
        actual: usize,
    },
    UnsupportedChannels(usize),
    NonFiniteInput,
    NonFiniteResult {
        pixel: usize,
        channel: FinalScaleChannel,
    },
    ArithmeticOverflow,
    Cancelled,
    InvalidRoi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinalScaleChannel {
    Red,
    Green,
    Blue,
}

impl fmt::Display for FinalScaleExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidShape { expected, actual } => {
                write!(
                    formatter,
                    "finalscale expected {expected} values, got {actual}"
                )
            }
            Self::InvalidStride { minimum, actual } => {
                write!(
                    formatter,
                    "finalscale stride {actual} is smaller than {minimum}"
                )
            }
            Self::UnsupportedChannels(channels) => {
                write!(formatter, "finalscale does not support {channels} channels")
            }
            Self::NonFiniteInput => formatter.write_str("finalscale input is non-finite"),
            Self::NonFiniteResult { pixel, channel } => {
                write!(
                    formatter,
                    "finalscale produced a non-finite {channel:?} at pixel {pixel}"
                )
            }
            Self::ArithmeticOverflow => formatter.write_str("finalscale arithmetic overflowed"),
            Self::Cancelled => formatter.write_str("finalscale execution was cancelled"),
            Self::InvalidRoi => formatter.write_str("finalscale ROI is invalid"),
        }
    }
}

impl std::error::Error for FinalScaleExecutionError {}

fn full_roi(dimensions: RasterDimensions) -> Result<Roi, FinalScalePlanError> {
    Roi::new(0, 0, dimensions.width(), dimensions.height())
        .map_err(|_| FinalScalePlanError::ArithmeticOverflow)
}

fn finite_channel(
    value: f32,
    pixel: usize,
    channel: FinalScaleChannel,
) -> Result<FiniteF32, FinalScaleExecutionError> {
    FiniteF32::new(value).map_err(|_| FinalScaleExecutionError::NonFiniteResult { pixel, channel })
}

fn digest_f32(values: &[f32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FINALSCALE_COMPATIBILITY_ID.as_bytes());
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

fn plan_identity(
    source: RasterDimensions,
    output: RasterDimensions,
    config: &FinalScaleConfig,
    upscale_suppressed: bool,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(FINALSCALE_RUST_ID.as_bytes());
    hasher.update(FINALSCALE_SCHEMA_VERSION.to_le_bytes());
    hasher.update(source.width().to_le_bytes());
    hasher.update(source.height().to_le_bytes());
    hasher.update(output.width().to_le_bytes());
    hasher.update(output.height().to_le_bytes());
    hasher.update(config.request().identity_bytes());
    hasher.update([
        config.quality().kind().tag(),
        config.quality().kernel().tag(),
    ]);
    hasher.update([
        u8::from(config.allow_upscale()),
        u8::from(upscale_suppressed),
    ]);
    hasher.update(config.limits().max_pixels().to_le_bytes());
    hasher.update((config.limits().max_bytes() as u64).to_le_bytes());
    hasher.finalize().into()
}
