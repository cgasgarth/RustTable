#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use super::common::OperationExecutionError;
use crate::{FiniteF32 as ProcessingFiniteF32, LinearRgb};
use rusttable_color::{
    ChromaticityMatrixError, FiniteF32, Matrix3, Primaries, WhitePoint, rgb_to_xyz_matrix,
    rotate_and_scale_primary,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const PRIMARIES_COMPATIBILITY_ID: &str = "primaries";
pub const PRIMARIES_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimariesGamutMode {
    Unbounded,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrimariesConfig {
    achromatic_tint_hue: FiniteF32,
    achromatic_tint_purity: FiniteF32,
    red_hue: FiniteF32,
    red_purity: FiniteF32,
    green_hue: FiniteF32,
    green_purity: FiniteF32,
    blue_hue: FiniteF32,
    blue_purity: FiniteF32,
    gamut: PrimariesGamutMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimariesConfigError {
    NonFinite,
    HueOutOfRange,
    PurityOutOfRange,
}

impl fmt::Display for PrimariesConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "primaries parameter is non-finite",
            Self::HueOutOfRange => "primaries hue must be within -pi..=pi",
            Self::PurityOutOfRange => "primaries purity is outside the registered range",
        })
    }
}

impl std::error::Error for PrimariesConfigError {}

impl PrimariesConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        achromatic_tint_hue: f32,
        achromatic_tint_purity: f32,
        red_hue: f32,
        red_purity: f32,
        green_hue: f32,
        green_purity: f32,
        blue_hue: f32,
        blue_purity: f32,
    ) -> Result<Self, PrimariesConfigError> {
        let values = [
            achromatic_tint_hue,
            achromatic_tint_purity,
            red_hue,
            red_purity,
            green_hue,
            green_purity,
            blue_hue,
            blue_purity,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(PrimariesConfigError::NonFinite);
        }
        if [achromatic_tint_hue, red_hue, green_hue, blue_hue]
            .iter()
            .any(|value| !(-std::f32::consts::PI..=std::f32::consts::PI).contains(value))
        {
            return Err(PrimariesConfigError::HueOutOfRange);
        }
        if !(0.0..=0.99).contains(&achromatic_tint_purity)
            || [red_purity, green_purity, blue_purity]
                .iter()
                .any(|value| !(0.01..=5.0).contains(value))
        {
            return Err(PrimariesConfigError::PurityOutOfRange);
        }
        Ok(Self {
            achromatic_tint_hue: finite(achromatic_tint_hue),
            achromatic_tint_purity: finite(achromatic_tint_purity),
            red_hue: finite(red_hue),
            red_purity: finite(red_purity),
            green_hue: finite(green_hue),
            green_purity: finite(green_purity),
            blue_hue: finite(blue_hue),
            blue_purity: finite(blue_purity),
            gamut: PrimariesGamutMode::Unbounded,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self {
            achromatic_tint_hue: finite(0.0),
            achromatic_tint_purity: finite(0.0),
            red_hue: finite(0.0),
            red_purity: finite(1.0),
            green_hue: finite(0.0),
            green_purity: finite(1.0),
            blue_hue: finite(0.0),
            blue_purity: finite(1.0),
            gamut: PrimariesGamutMode::Unbounded,
        }
    }

    #[must_use]
    pub const fn achromatic_tint_hue(self) -> FiniteF32 {
        self.achromatic_tint_hue
    }
    #[must_use]
    pub const fn achromatic_tint_purity(self) -> FiniteF32 {
        self.achromatic_tint_purity
    }
    #[must_use]
    pub const fn red_hue(self) -> FiniteF32 {
        self.red_hue
    }
    #[must_use]
    pub const fn red_purity(self) -> FiniteF32 {
        self.red_purity
    }
    #[must_use]
    pub const fn green_hue(self) -> FiniteF32 {
        self.green_hue
    }
    #[must_use]
    pub const fn green_purity(self) -> FiniteF32 {
        self.green_purity
    }
    #[must_use]
    pub const fn blue_hue(self) -> FiniteF32 {
        self.blue_hue
    }
    #[must_use]
    pub const fn blue_purity(self) -> FiniteF32 {
        self.blue_purity
    }
    #[must_use]
    pub const fn gamut(self) -> PrimariesGamutMode {
        self.gamut
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrimariesPlanError {
    InvalidConfig(PrimariesConfigError),
    Matrix(ChromaticityMatrixError),
    Dimensions(OperationExecutionError),
    Serialization(String),
}

impl fmt::Display for PrimariesPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "primaries plan error: {self:?}")
    }
}
impl std::error::Error for PrimariesPlanError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimariesPlan {
    schema_version: u16,
    config: PrimariesConfig,
    source: Primaries,
    output: [(FiniteF32, FiniteF32); 3],
    output_white: WhitePoint,
    matrix: Matrix3,
    identity: [u8; 32],
}

impl PrimariesPlan {
    pub fn new(config: PrimariesConfig, source: Primaries) -> Result<Self, PrimariesPlanError> {
        let output = [
            rotate_and_scale_primary(source, config.red_purity.get(), config.red_hue.get(), 0),
            rotate_and_scale_primary(source, config.green_purity.get(), config.green_hue.get(), 1),
            rotate_and_scale_primary(source, config.blue_purity.get(), config.blue_hue.get(), 2),
        ]
        .into_iter()
        .map(|value| value.map_err(PrimariesPlanError::Matrix))
        .collect::<Result<Vec<_>, _>>()?;
        let output: [(f32, f32); 3] = output
            .try_into()
            .map_err(|_| PrimariesPlanError::Matrix(ChromaticityMatrixError::NonFinite))?;
        let output_white = rotate_and_scale_primary(
            source,
            config.achromatic_tint_purity.get(),
            config.achromatic_tint_hue.get(),
            0,
        )
        .map_err(PrimariesPlanError::Matrix)?;
        let matrix = rgb_to_xyz_matrix(
            output,
            WhitePoint::custom(output_white.0, output_white.1)
                .map_err(|_| PrimariesPlanError::Matrix(ChromaticityMatrixError::NonFinite))?,
        )
        .map_err(PrimariesPlanError::Matrix)?
        .multiply(
            rgb_to_xyz_matrix(
                [
                    pair(source.red()),
                    pair(source.green()),
                    pair(source.blue()),
                ],
                source.white(),
            )
            .map_err(PrimariesPlanError::Matrix)?
            .inverse()
            .map_err(|_| PrimariesPlanError::Matrix(ChromaticityMatrixError::Singular))?,
        )
        .map_err(|_| PrimariesPlanError::Matrix(ChromaticityMatrixError::Singular))?;
        let output_finite = output.map(|(x, y)| (finite(x), finite(y)));
        let identity = plan_identity(config, source, output_finite, output_white, matrix)?;
        let output_white = WhitePoint::custom(output_white.0, output_white.1)
            .map_err(|_| PrimariesPlanError::Matrix(ChromaticityMatrixError::NonFinite))?;
        Ok(Self {
            schema_version: PRIMARIES_SCHEMA_VERSION,
            config,
            source,
            output: output_finite,
            output_white,
            matrix,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> PrimariesConfig {
        self.config
    }
    #[must_use]
    pub const fn source(&self) -> Primaries {
        self.source
    }
    #[must_use]
    pub const fn output_white(&self) -> WhitePoint {
        self.output_white
    }
    #[must_use]
    pub const fn matrix(&self) -> Matrix3 {
        self.matrix
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
    #[must_use]
    pub const fn output_primaries(&self) -> [(FiniteF32, FiniteF32); 3] {
        self.output
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<PrimariesExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<PrimariesExecution, OperationExecutionError> {
        let mut output = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let values =
                self.matrix
                    .apply([pixel.red().get(), pixel.green().get(), pixel.blue().get()]);
            let red = ProcessingFiniteF32::new(values[0]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Red,
                }
            })?;
            let green = ProcessingFiniteF32::new(values[1]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Green,
                }
            })?;
            let blue = ProcessingFiniteF32::new(values[2]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Blue,
                }
            })?;
            output.push(LinearRgb::new(red, green, blue));
        }
        let receipt = ExecutionReceipt::new(self.identity, input, &output);
        Ok(PrimariesExecution {
            pixels: output,
            receipt,
        })
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, PrimariesPlanError> {
        postcard::to_allocvec(self)
            .map_err(|error| PrimariesPlanError::Serialization(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimariesExecution {
    pixels: Vec<LinearRgb>,
    receipt: ExecutionReceipt,
}
impl PrimariesExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn receipt(&self) -> &ExecutionReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}
impl ExecutionReceipt {
    #[must_use]
    pub const fn plan_identity(&self) -> [u8; 32] {
        self.plan_identity
    }
    #[must_use]
    pub const fn input_digest(&self) -> [u8; 32] {
        self.input_digest
    }
    #[must_use]
    pub const fn output_digest(&self) -> [u8; 32] {
        self.output_digest
    }
}

fn plan_identity(
    config: PrimariesConfig,
    source: Primaries,
    output: [(FiniteF32, FiniteF32); 3],
    white: (f32, f32),
    matrix: Matrix3,
) -> Result<[u8; 32], PrimariesPlanError> {
    let bytes = postcard::to_allocvec(&(
        PRIMARIES_SCHEMA_VERSION,
        config,
        source,
        output,
        white,
        matrix,
    ))
    .map_err(|error| PrimariesPlanError::Serialization(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
}

fn pair(value: (rusttable_color::FiniteF32, rusttable_color::FiniteF32)) -> (f32, f32) {
    (value.0.get(), value.1.get())
}
fn finite(value: f32) -> FiniteF32 {
    FiniteF32::new(value).unwrap_or_else(|_| unreachable!())
}

fn digest_pixels(pixels: &[LinearRgb]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.primaries.raster.v1");
    for pixel in pixels {
        hasher.update(pixel.red().get().to_bits().to_le_bytes());
        hasher.update(pixel.green().get().to_bits().to_le_bytes());
        hasher.update(pixel.blue().get().to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

impl ExecutionReceipt {
    fn new(plan_identity: [u8; 32], input: &[LinearRgb], output: &[LinearRgb]) -> Self {
        Self {
            plan_identity,
            input_digest: digest_pixels(input),
            output_digest: digest_pixels(output),
        }
    }
}

pub const fn wgpu_passes() -> [&'static str; 1] {
    ["primaries_matrix"]
}
