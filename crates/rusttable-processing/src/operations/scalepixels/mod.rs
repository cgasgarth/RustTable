//! Pixel-aspect-ratio correction operation.
//!
//! The registry and pixelpipe own final wiring. This facade exports the
//! operation contract while its checked implementation is split by concern.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::chunks_exact_to_as_chunks,
    clippy::manual_is_multiple_of,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    unused_imports
)]

use crate::{LinearRgb, RasterDimensions};
use rusttable_image::Roi;
use std::fmt;

mod descriptor;
mod geometry;
mod image;
mod parameters;
mod resample;

pub(crate) use super::common::OperationExecutionError;

pub use descriptor::scalepixels_descriptor;
pub use geometry::{ScalePixelsGeometry, ScalePixelsGeometryError};
pub use image::{ScalePixelsImageError, ScalePixelsImageExecution};
pub use parameters::{
    ScalePixelsCodecError, ScalePixelsConfig, ScalePixelsConfigError, ScalePixelsHistory,
    ScalePixelsKernel, ScalePixelsParametersV1, ScalePixelsPreferences,
};
pub use resample::{WGSL_IMAGE_RESAMPLER, WGSL_MASK_RESAMPLER};

pub const SCALEPIXELS_COMPATIBILITY_ID: &str = "scalepixels";
pub const SCALEPIXELS_RUST_ID: &str = "rusttable.scalepixels";
pub const SCALEPIXELS_SCHEMA_VERSION: u16 = 1;
pub const SCALEPIXELS_PARAMETER_BYTES: usize = 4;
pub const MIN_PIXEL_ASPECT_RATIO: f32 = 0.5;
pub const MAX_PIXEL_ASPECT_RATIO: f32 = 2.0;
pub const MAX_OUTPUT_DIMENSION: u32 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScalePixelsGpuDispatch {
    input_roi: Roi,
    output_roi: Roi,
    input_row_stride: usize,
    output_row_stride: usize,
    image_support: u32,
    mask_support: u32,
    workgroups: (u32, u32),
    memory_bytes: usize,
}

impl ScalePixelsGpuDispatch {
    #[must_use]
    pub const fn input_roi(self) -> Roi {
        self.input_roi
    }

    #[must_use]
    pub const fn output_roi(self) -> Roi {
        self.output_roi
    }

    #[must_use]
    pub const fn input_row_stride(self) -> usize {
        self.input_row_stride
    }

    #[must_use]
    pub const fn output_row_stride(self) -> usize {
        self.output_row_stride
    }

    #[must_use]
    pub const fn image_support(self) -> u32 {
        self.image_support
    }

    #[must_use]
    pub const fn mask_support(self) -> u32 {
        self.mask_support
    }

    #[must_use]
    pub const fn workgroups(self) -> (u32, u32) {
        self.workgroups
    }

    #[must_use]
    pub const fn memory_bytes(self) -> usize {
        self.memory_bytes
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScalePixelsPlanError {
    Config(ScalePixelsConfigError),
    ArithmeticOverflow,
    OutputTooLarge,
    RoiOutsideSource,
    RoiOutsideOutput,
}

impl fmt::Display for ScalePixelsPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Config(_) => "invalid scalepixels configuration",
            Self::ArithmeticOverflow => "scalepixels geometry arithmetic overflowed",
            Self::OutputTooLarge => "scalepixels output dimensions are excessive",
            Self::RoiOutsideSource => "ROI is outside scalepixels source dimensions",
            Self::RoiOutsideOutput => "ROI is outside scalepixels output dimensions",
        })
    }
}

impl std::error::Error for ScalePixelsPlanError {}

impl From<ScalePixelsConfigError> for ScalePixelsPlanError {
    fn from(error: ScalePixelsConfigError) -> Self {
        Self::Config(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScalePixelsPlan {
    pub(super) config: ScalePixelsConfig,
    pub(super) source_dimensions: RasterDimensions,
    pub(super) output_dimensions: RasterDimensions,
    pub(super) x_scale: f32,
    pub(super) y_scale: f32,
    pub(super) preferences: ScalePixelsPreferences,
    pub(super) identity: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalePixelsExecution {
    pixels: Vec<LinearRgb>,
    dimensions: RasterDimensions,
    identity: [u8; 32],
}

impl ScalePixelsExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    #[must_use]
    pub const fn dimensions(&self) -> RasterDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalePixelsMaskError {
    DimensionsMismatch { expected: usize, actual: usize },
    NonFiniteInput,
    ArithmeticOverflow,
    RoiOutsideSource,
    RoiOutsideOutput,
}

impl fmt::Display for ScalePixelsMaskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DimensionsMismatch { .. } => "mask dimensions do not match the input ROI",
            Self::NonFiniteInput => "mask contains a non-finite sample",
            Self::ArithmeticOverflow => "mask arithmetic overflowed",
            Self::RoiOutsideSource => "mask input ROI is outside source dimensions",
            Self::RoiOutsideOutput => "mask output ROI is outside output dimensions",
        })
    }
}

impl std::error::Error for ScalePixelsMaskError {}
