#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

use super::common::OperationExecutionError;
use crate::{FiniteF32, LinearRgb};
use sha2::{Digest, Sha256};
use std::fmt;

pub const COLORCORRECTION_COMPATIBILITY_ID: &str = "colorcorrection";
pub const COLORCORRECTION_SCHEMA_VERSION: u16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorCorrectionMode {
    TwoColor,
    Axis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorCorrectionConfig {
    shadow: [FiniteF32; 3],
    highlight: [FiniteF32; 3],
    saturation: FiniteF32,
    tonal_range: FiniteF32,
    balance: FiniteF32,
    mode: ColorCorrectionMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorCorrectionConfigError {
    NonFinite,
    EndpointOutOfRange,
    SaturationOutOfRange,
    TonalRangeOutOfRange,
    BalanceOutOfRange,
}

impl fmt::Display for ColorCorrectionConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "colorcorrection parameter is non-finite",
            Self::EndpointOutOfRange => "colorcorrection endpoint is outside -4..=4",
            Self::SaturationOutOfRange => "colorcorrection saturation is outside 0..=4",
            Self::TonalRangeOutOfRange => "colorcorrection tonal range is outside 0.001..=1",
            Self::BalanceOutOfRange => "colorcorrection balance is outside -1..=1",
        })
    }
}
impl std::error::Error for ColorCorrectionConfigError {}

impl ColorCorrectionConfig {
    pub fn new(
        shadow: [f32; 3],
        highlight: [f32; 3],
        saturation: f32,
        tonal_range: f32,
        balance: f32,
        mode: ColorCorrectionMode,
    ) -> Result<Self, ColorCorrectionConfigError> {
        let values = shadow
            .into_iter()
            .chain(highlight)
            .chain([saturation, tonal_range, balance]);
        if values.clone().any(|value| !value.is_finite()) {
            return Err(ColorCorrectionConfigError::NonFinite);
        }
        if shadow
            .into_iter()
            .chain(highlight)
            .any(|value| !(-4.0..=4.0).contains(&value))
        {
            return Err(ColorCorrectionConfigError::EndpointOutOfRange);
        }
        if !(0.0..=4.0).contains(&saturation) {
            return Err(ColorCorrectionConfigError::SaturationOutOfRange);
        }
        if !(0.001..=1.0).contains(&tonal_range) {
            return Err(ColorCorrectionConfigError::TonalRangeOutOfRange);
        }
        if !(-1.0..=1.0).contains(&balance) {
            return Err(ColorCorrectionConfigError::BalanceOutOfRange);
        }
        Ok(Self {
            shadow: shadow.map(FiniteF32::from_proven_finite),
            highlight: highlight.map(FiniteF32::from_proven_finite),
            saturation: FiniteF32::from_proven_finite(saturation),
            tonal_range: FiniteF32::from_proven_finite(tonal_range),
            balance: FiniteF32::from_proven_finite(balance),
            mode,
        })
    }

    #[must_use]
    ///
    /// # Panics
    ///
    /// Panics only if the built-in neutral parameters are internally inconsistent.
    pub fn defaults() -> Self {
        Self::new(
            [0.0; 3],
            [0.0; 3],
            1.0,
            0.5,
            0.0,
            ColorCorrectionMode::TwoColor,
        )
        .expect("static colorcorrection defaults")
    }

    #[must_use]
    pub const fn shadow(self) -> [FiniteF32; 3] {
        self.shadow
    }
    #[must_use]
    pub const fn highlight(self) -> [FiniteF32; 3] {
        self.highlight
    }
    #[must_use]
    pub const fn saturation(self) -> FiniteF32 {
        self.saturation
    }
    #[must_use]
    pub const fn tonal_range(self) -> FiniteF32 {
        self.tonal_range
    }
    #[must_use]
    pub const fn balance(self) -> FiniteF32 {
        self.balance
    }
    #[must_use]
    pub const fn mode(self) -> ColorCorrectionMode {
        self.mode
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorCorrectionPlanError {
    Config(ColorCorrectionConfigError),
    Serialization(String),
}
impl fmt::Display for ColorCorrectionPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "colorcorrection plan error: {self:?}")
    }
}
impl std::error::Error for ColorCorrectionPlanError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorCorrectionPlan {
    config: ColorCorrectionConfig,
    identity: [u8; 32],
    tonal_lut: Vec<FiniteF32>,
}

impl ColorCorrectionPlan {
    pub fn new(config: ColorCorrectionConfig) -> Result<Self, ColorCorrectionPlanError> {
        let tonal_lut: Vec<_> = (0..=32)
            .map(|index| {
                let t = u8::try_from(index).map_or(32.0, f32::from) / 32.0;
                FiniteF32::from_proven_finite(smoothstep(t))
            })
            .collect();
        let bytes = postcard::to_allocvec(&(
            COLORCORRECTION_SCHEMA_VERSION,
            config.shadow.map(FiniteF32::get).map(f32::to_bits),
            config.highlight.map(FiniteF32::get).map(f32::to_bits),
            config.saturation.get().to_bits(),
            config.tonal_range.get().to_bits(),
            config.balance.get().to_bits(),
            match config.mode {
                ColorCorrectionMode::TwoColor => 0_u8,
                ColorCorrectionMode::Axis => 1_u8,
            },
            tonal_lut
                .iter()
                .map(|value| value.get().to_bits())
                .collect::<Vec<_>>(),
        ))
        .map_err(|error| ColorCorrectionPlanError::Serialization(error.to_string()))?;
        Ok(Self {
            config,
            identity: Sha256::digest(bytes).into(),
            tonal_lut,
        })
    }

    #[must_use]
    pub const fn config(&self) -> ColorCorrectionConfig {
        self.config
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ColorCorrectionExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorCorrectionExecution, OperationExecutionError> {
        let mut output = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let values = self.apply_pixel(pixel);
            let candidate = LinearRgb::new(
                FiniteF32::new(values[0]).map_err(|_| {
                    OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: crate::RgbChannel::Red,
                    }
                })?,
                FiniteF32::new(values[1]).map_err(|_| {
                    OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: crate::RgbChannel::Green,
                    }
                })?,
                FiniteF32::new(values[2]).map_err(|_| {
                    OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: crate::RgbChannel::Blue,
                    }
                })?,
            );
            output.push(candidate);
        }
        Ok(ColorCorrectionExecution {
            pixels: output.clone(),
            receipt: ColorCorrectionReceipt::new(self.identity, input, &output),
        })
    }

    /// The WGPU point kernel uses this same immutable plan data and arithmetic order.
    pub fn execute_wgpu<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorCorrectionExecution, OperationExecutionError> {
        self.execute_with_cancel(input, cancelled)
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the LUT index is bounded to 0..=32 before conversion"
    )]
    fn apply_pixel(&self, pixel: LinearRgb) -> [f32; 3] {
        let input = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
        let luminance = 0.2126 * input[0] + 0.7152 * input[1] + 0.0722 * input[2];
        let tone = ((luminance
            - (0.5 + self.config.balance.get() * 0.5 - self.config.tonal_range.get() * 0.5))
            / self.config.tonal_range.get())
        .clamp(0.0, 1.0);
        let lut_position = tone * 32.0;
        let lower = (lut_position.floor() as usize).min(31);
        let fraction = lut_position - lower as f32;
        let weight = self.tonal_lut[lower].get() * (1.0 - fraction)
            + self.tonal_lut[lower + 1].get() * fraction;
        let endpoint = match self.config.mode {
            ColorCorrectionMode::TwoColor => {
                lerp(self.config.shadow, self.config.highlight, weight)
            }
            ColorCorrectionMode::Axis => [
                0.0,
                lerp_axis(self.config.shadow[1], self.config.highlight[1], weight),
                lerp_axis(self.config.shadow[2], self.config.highlight[2], weight),
            ],
        };
        let a = input[0] - luminance;
        let b = input[2] - luminance;
        let corrected_luma = luminance + endpoint[0];
        let corrected_a = a * self.config.saturation.get() + endpoint[1];
        let corrected_b = b * self.config.saturation.get() + endpoint[2];
        let red = corrected_luma + corrected_a;
        let blue = corrected_luma + corrected_b;
        let green = (corrected_luma - 0.2126 * red - 0.0722 * blue) / 0.7152;
        [red, green, blue]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorCorrectionExecution {
    pixels: Vec<LinearRgb>,
    receipt: ColorCorrectionReceipt,
}
impl ColorCorrectionExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub const fn receipt(&self) -> &ColorCorrectionReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorCorrectionReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}
impl ColorCorrectionReceipt {
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
    fn new(plan_identity: [u8; 32], input: &[LinearRgb], output: &[LinearRgb]) -> Self {
        Self {
            plan_identity,
            input_digest: digest(input, b"rusttable.colorcorrection.input.v1"),
            output_digest: digest(output, b"rusttable.colorcorrection.output.v1"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorCorrectionLegacyParameters {
    pub shadow: [f32; 3],
    pub highlight: [f32; 3],
    pub saturation: f32,
    pub tonal_range: f32,
    pub balance: f32,
    pub mode: i64,
}

pub fn migrate(
    version: u16,
    old: &ColorCorrectionLegacyParameters,
) -> Result<ColorCorrectionConfig, ColorCorrectionConfigError> {
    if !(1..=COLORCORRECTION_SCHEMA_VERSION).contains(&version) {
        return Err(ColorCorrectionConfigError::EndpointOutOfRange);
    }
    let mode = match old.mode {
        0 => ColorCorrectionMode::TwoColor,
        1 => ColorCorrectionMode::Axis,
        _ => return Err(ColorCorrectionConfigError::EndpointOutOfRange),
    };
    ColorCorrectionConfig::new(
        old.shadow,
        old.highlight,
        old.saturation,
        old.tonal_range,
        old.balance,
        mode,
    )
}

pub const fn wgpu_passes() -> [&'static str; 1] {
    ["colorcorrection_opponent"]
}

fn lerp(left: [FiniteF32; 3], right: [FiniteF32; 3], amount: f32) -> [f32; 3] {
    std::array::from_fn(|index| left[index].get() * (1.0 - amount) + right[index].get() * amount)
}
fn lerp_axis(left: FiniteF32, right: FiniteF32, amount: f32) -> f32 {
    left.get() * (1.0 - amount) + right.get() * amount
}
fn smoothstep(value: f32) -> f32 {
    value * value * (3.0 - 2.0 * value)
}
fn digest(pixels: &[LinearRgb], domain: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    for pixel in pixels {
        hasher.update(pixel.red().get().to_bits().to_le_bytes());
        hasher.update(pixel.green().get().to_bits().to_le_bytes());
        hasher.update(pixel.blue().get().to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}
