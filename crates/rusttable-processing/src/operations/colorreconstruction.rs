//! Post-demosaic color reconstruction mapped from
//! `Darktable/src/iop/colorreconstruction.c`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::must_use_candidate
)]

use std::f32::consts::TAU;
use std::fmt;

use crate::{FiniteF32, LinearRgb, RasterDimensions};

use super::common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics,
    ReconstructionReceipt, checked_bytes, chroma, from_luma_chroma, luma, neighborhood,
    validate_shape,
};

pub const COLORRECONSTRUCTION_COMPATIBILITY_ID: &str = "colorreconstruction";
pub const COLORRECONSTRUCTION_SCHEMA_VERSION: u16 = 3;

/// `dt_iop_colorreconstruct_precedence_t`, retained numerically for imports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorReconstructionPrecedence {
    None,
    Chroma,
    Hue,
}

impl ColorReconstructionPrecedence {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }

    pub fn from_id(id: i32) -> Result<Self, ColorReconstructionParameterError> {
        match id {
            0 => Ok(Self::None),
            1 => Ok(Self::Chroma),
            2 => Ok(Self::Hue),
            _ => Err(ColorReconstructionParameterError::UnknownPrecedence(id)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorReconstructionConfig {
    threshold: FiniteF32,
    spatial: FiniteF32,
    range: FiniteF32,
    hue: FiniteF32,
    precedence: ColorReconstructionPrecedence,
}

impl ColorReconstructionConfig {
    pub fn new(
        threshold: f32,
        spatial: f32,
        range: f32,
        hue: f32,
        precedence: ColorReconstructionPrecedence,
    ) -> Result<Self, ColorReconstructionParameterError> {
        Ok(Self {
            threshold: bounded("threshold", threshold, 50.0, 150.0)?,
            spatial: bounded("spatial", spatial, 0.0, 1000.0)?,
            range: bounded("range", range, 0.0, 50.0)?,
            hue: bounded("hue", hue, 0.0, 1.0)?,
            precedence,
        })
    }

    #[must_use]
    pub const fn threshold(self) -> FiniteF32 {
        self.threshold
    }
    #[must_use]
    pub const fn spatial(self) -> FiniteF32 {
        self.spatial
    }
    #[must_use]
    pub const fn range(self) -> FiniteF32 {
        self.range
    }
    #[must_use]
    pub const fn hue(self) -> FiniteF32 {
        self.hue
    }
    #[must_use]
    pub const fn precedence(self) -> ColorReconstructionPrecedence {
        self.precedence
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorReconstructionParameterError {
    UnknownPrecedence(i32),
    OutOfRange { name: &'static str, value: u32 },
    NonFinite(&'static str),
}
impl fmt::Display for ColorReconstructionParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownPrecedence(id) => {
                write!(formatter, "unknown color reconstruction precedence {id}")
            }
            Self::OutOfRange { name, value } => {
                write!(formatter, "{name} is out of range ({value})")
            }
            Self::NonFinite(name) => write!(formatter, "{name} is non-finite"),
        }
    }
}
impl std::error::Error for ColorReconstructionParameterError {}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, ColorReconstructionParameterError> {
    let value =
        FiniteF32::new(value).map_err(|_| ColorReconstructionParameterError::NonFinite(name))?;
    if (minimum..=maximum).contains(&value.get()) {
        Ok(value)
    } else {
        Err(ColorReconstructionParameterError::OutOfRange {
            name,
            value: value.get().to_bits(),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV1 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV2 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
    pub precedence: i32,
}
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorReconstructionV3 {
    pub threshold: f32,
    pub spatial: f32,
    pub range: f32,
    pub hue: f32,
    pub precedence: i32,
}

pub fn migrate_v1(value: ColorReconstructionV1) -> ColorReconstructionV3 {
    ColorReconstructionV3 {
        threshold: value.threshold,
        spatial: value.spatial,
        range: value.range,
        hue: 0.66,
        precedence: 0,
    }
}
pub fn migrate_v2(value: ColorReconstructionV2) -> ColorReconstructionV3 {
    ColorReconstructionV3 {
        threshold: value.threshold,
        spatial: value.spatial,
        range: value.range,
        hue: 0.66,
        precedence: value.precedence,
    }
}
impl ColorReconstructionV3 {
    pub fn config(self) -> Result<ColorReconstructionConfig, ColorReconstructionParameterError> {
        ColorReconstructionConfig::new(
            self.threshold,
            self.spatial,
            self.range,
            self.hue,
            ColorReconstructionPrecedence::from_id(self.precedence)?,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorReconstructionPlan {
    config: ColorReconstructionConfig,
    dimensions: RasterDimensions,
    budget: ReconstructionBudget,
}

impl ColorReconstructionPlan {
    pub fn new(
        config: ColorReconstructionConfig,
        dimensions: RasterDimensions,
        budget: ReconstructionBudget,
    ) -> Result<Self, OperationExecutionError> {
        checked_bytes(
            usize::try_from(dimensions.pixel_count()).unwrap_or(usize::MAX),
            8,
            budget,
        )?;
        Ok(Self {
            config,
            dimensions,
            budget,
        })
    }

    #[must_use]
    pub const fn config(self) -> ColorReconstructionConfig {
        self.config
    }
    #[must_use]
    pub const fn dimensions(self) -> RasterDimensions {
        self.dimensions
    }
    #[must_use]
    pub const fn full_image_analysis(self) -> bool {
        true
    }
    #[must_use]
    pub const fn support_radius(self) -> u32 {
        self.config.spatial().get() as u32 / 10 + 1
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ColorReconstructionExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorReconstructionExecution, OperationExecutionError> {
        validate_shape(self.dimensions, input)?;
        checked_bytes(input.len(), 8, self.budget)?;
        let mut diagnostics = ReconstructionDiagnostics::new(input.len());
        let mut affected = vec![false; input.len()];
        let threshold = self.config.threshold().get();
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            // Darktable operates on Lab lightness in [0,100].  The working
            // contract is scene-linear RGB, so the compatibility boundary is
            // the same value expressed as percent luminance.
            affected[index] = luma(pixel) * 100.0 > threshold;
            diagnostics.affected[index] = affected[index];
        }
        if !affected.iter().any(|value| *value) {
            let receipt = ReconstructionReceipt::new(
                COLORRECONSTRUCTION_COMPATIBILITY_ID,
                COLORRECONSTRUCTION_SCHEMA_VERSION,
                input,
                input,
                &diagnostics,
            );
            return Ok(ColorReconstructionExecution {
                pixels: input.to_vec(),
                diagnostics,
                receipt,
            });
        }
        if affected.iter().all(|value| *value) {
            return Err(OperationExecutionError::NoReconstructionEvidence);
        }
        let radius = self
            .support_radius()
            .min(self.dimensions.width().max(self.dimensions.height()));
        let mut output = input.to_vec();
        for index in 0..input.len() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            if !affected[index] {
                continue;
            }
            let candidate = replacement(
                input,
                &affected,
                index,
                self.dimensions,
                radius,
                self.config,
            );
            let Some((chroma_value, confidence)) = candidate else {
                return Err(OperationExecutionError::NoReconstructionEvidence);
            };
            let result = from_luma_chroma(luma(input[index]), chroma_value)
                .ok_or(OperationExecutionError::NoReconstructionEvidence)?;
            diagnostics.candidate[index] = true;
            diagnostics.confidence[index] = confidence;
            diagnostics.contribution[index] = difference(input[index], result, index)?;
            output[index] = result;
        }
        let receipt = ReconstructionReceipt::new(
            COLORRECONSTRUCTION_COMPATIBILITY_ID,
            COLORRECONSTRUCTION_SCHEMA_VERSION,
            input,
            &output,
            &diagnostics,
        );
        Ok(ColorReconstructionExecution {
            pixels: output,
            diagnostics,
            receipt,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ColorReconstructionExecution {
    pixels: Vec<LinearRgb>,
    diagnostics: ReconstructionDiagnostics,
    receipt: ReconstructionReceipt,
}
impl ColorReconstructionExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn diagnostics(&self) -> &ReconstructionDiagnostics {
        &self.diagnostics
    }
    #[must_use]
    pub const fn receipt(&self) -> &ReconstructionReceipt {
        &self.receipt
    }
}

fn replacement(
    input: &[LinearRgb],
    affected: &[bool],
    index: usize,
    dimensions: RasterDimensions,
    radius: u32,
    config: ColorReconstructionConfig,
) -> Option<((f32, f32), f32)> {
    let source_luma = luma(input[index]);
    let target_angle = config.hue().get() * TAU;
    let range = config.range().get().max(0.1) / 100.0;
    let mut sum = (0.0, 0.0, 0.0);
    for neighbor in neighborhood(dimensions, index, radius) {
        if affected[neighbor] {
            continue;
        }
        let neighbor_luma = luma(input[neighbor]);
        let spatial =
            1.0 / (1.0 + index.abs_diff(neighbor) as f32 / (dimensions.width() as f32 + 1.0));
        let range_weight = (-(neighbor_luma - source_luma).abs() / range).exp();
        let (a, b) = chroma(input[neighbor]);
        let chroma_weight = (a * a + b * b).sqrt();
        let hue_weight = {
            let angle = b.atan2(a);
            let distance = (angle - target_angle).sin().abs();
            1.0 - distance
        };
        let precedence = match config.precedence() {
            ColorReconstructionPrecedence::None => 1.0,
            ColorReconstructionPrecedence::Chroma => 1.0 + chroma_weight,
            ColorReconstructionPrecedence::Hue => 1.0 + hue_weight.max(0.0),
        };
        let weight = spatial * range_weight * precedence;
        sum.0 += a * weight;
        sum.1 += b * weight;
        sum.2 += weight;
    }
    if sum.2 == 0.0 {
        None
    } else {
        let confidence = (sum.2 / (sum.2 + 1.0)).clamp(0.0, 1.0);
        Some(((sum.0 / sum.2, sum.1 / sum.2), confidence))
    }
}

fn difference(
    source: LinearRgb,
    output: LinearRgb,
    index: usize,
) -> Result<LinearRgb, OperationExecutionError> {
    let values = [
        output.red().get() - source.red().get(),
        output.green().get() - source.green().get(),
        output.blue().get() - source.blue().get(),
    ];
    Ok(LinearRgb::new(
        FiniteF32::new(values[0]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: crate::RgbChannel::Red,
        })?,
        FiniteF32::new(values[1]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: crate::RgbChannel::Green,
        })?,
        FiniteF32::new(values[2]).map_err(|_| OperationExecutionError::NonFiniteResult {
            pixel: index,
            channel: crate::RgbChannel::Blue,
        })?,
    ))
}

/// GPU parity metadata is shared with the backend-neutral registry binding.
#[must_use]
pub const fn wgpu_passes() -> [&'static str; 4] {
    [
        "colorreconstruction.mask",
        "colorreconstruction.propagate",
        "colorreconstruction.recombine",
        "colorreconstruction.diagnostics",
    ]
}
