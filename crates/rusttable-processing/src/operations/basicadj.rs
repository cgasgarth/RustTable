//! Darktable's legacy `basicadj` composite adjustment operation.
//!
//! The operation stays atomic because Darktable applies these stages in a
//! compatibility-sensitive order.  The current `RustTable` operation boundary
//! supplies deterministic point execution, so the auto-levels controls are
//! retained in the persisted configuration while analysis remains outside
//! this slice.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

use super::common::OperationExecutionError;
use crate::{FiniteF32, LinearRgb, RgbChannel};
use sha2::{Digest, Sha256};
use std::fmt;

pub const BASICADJ_COMPATIBILITY_ID: &str = "basicadj";
pub const BASICADJ_SCHEMA_VERSION: u16 = 2;
const DEFAULT_MIDDLE_GREY: f32 = 18.42;
const CAMERA_LUMINANCE: [f32; 3] = [0.222_504_5, 0.716_878_6, 0.060_616_9];

/// Darktable's stable RGB norm IDs used by the color-preserving contrast path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreserveColors {
    None,
    Luminance,
    Max,
    Average,
    Sum,
    Norm,
    Power,
}

impl PreserveColors {
    #[must_use]
    pub const fn id(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Luminance => 1,
            Self::Max => 2,
            Self::Average => 3,
            Self::Sum => 4,
            Self::Norm => 5,
            Self::Power => 6,
        }
    }

    pub fn from_id(id: i32) -> Result<Self, BasicAdjConfigError> {
        match id {
            0 => Ok(Self::None),
            1 => Ok(Self::Luminance),
            2 => Ok(Self::Max),
            3 => Ok(Self::Average),
            4 => Ok(Self::Sum),
            5 => Ok(Self::Norm),
            6 => Ok(Self::Power),
            value => Err(BasicAdjConfigError::UnknownPreserveColors(value)),
        }
    }
}

/// Version 1 of Darktable's persisted `basicadj` parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV1 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: i32,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub clip: f32,
}

/// Version 2 added the independent vibrance control.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BasicAdjParametersV2 {
    pub black_point: f32,
    pub exposure: f32,
    pub hlcompr: f32,
    pub hlcomprthresh: f32,
    pub contrast: f32,
    pub preserve_colors: i32,
    pub middle_grey: f32,
    pub brightness: f32,
    pub saturation: f32,
    pub vibrance: f32,
    pub clip: f32,
}

impl BasicAdjParametersV1 {
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            black_point: 0.0,
            exposure: 0.0,
            hlcompr: 0.0,
            hlcomprthresh: 0.0,
            contrast: 0.0,
            preserve_colors: 1,
            middle_grey: DEFAULT_MIDDLE_GREY,
            brightness: 0.0,
            saturation: 0.0,
            clip: 0.0,
        }
    }
}

impl BasicAdjParametersV2 {
    #[must_use]
    pub const fn defaults() -> Self {
        BasicAdjParametersV1::defaults_v2()
    }
}

impl BasicAdjParametersV1 {
    const fn defaults_v2() -> BasicAdjParametersV2 {
        BasicAdjParametersV2 {
            black_point: 0.0,
            exposure: 0.0,
            hlcompr: 0.0,
            hlcomprthresh: 0.0,
            contrast: 0.0,
            preserve_colors: 1,
            middle_grey: DEFAULT_MIDDLE_GREY,
            brightness: 0.0,
            saturation: 0.0,
            vibrance: 0.0,
            clip: 0.0,
        }
    }
}

#[must_use]
pub const fn migrate_v1_to_v2(value: BasicAdjParametersV1) -> BasicAdjParametersV2 {
    BasicAdjParametersV2 {
        black_point: value.black_point,
        exposure: value.exposure,
        hlcompr: value.hlcompr,
        hlcomprthresh: value.hlcomprthresh,
        contrast: value.contrast,
        preserve_colors: value.preserve_colors,
        middle_grey: value.middle_grey,
        brightness: value.brightness,
        saturation: value.saturation,
        vibrance: 0.0,
        clip: value.clip,
    }
}

/// Checked, immutable configuration for one legacy `basicadj` history node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BasicAdjConfig {
    black_point: FiniteF32,
    exposure: FiniteF32,
    hlcompr: FiniteF32,
    hlcomprthresh: FiniteF32,
    contrast: FiniteF32,
    preserve_colors: PreserveColors,
    middle_grey: FiniteF32,
    brightness: FiniteF32,
    saturation: FiniteF32,
    vibrance: FiniteF32,
    clip: FiniteF32,
}

impl BasicAdjConfig {
    pub fn new(value: BasicAdjParametersV2) -> Result<Self, BasicAdjConfigError> {
        Ok(Self {
            black_point: bounded("black_point", value.black_point, -1.0, 1.0)?,
            exposure: bounded("exposure", value.exposure, -18.0, 18.0)?,
            hlcompr: bounded("hlcompr", value.hlcompr, 0.0, 500.0)?,
            hlcomprthresh: bounded("hlcomprthresh", value.hlcomprthresh, 0.0, 100.0)?,
            contrast: bounded("contrast", value.contrast, -1.0, 5.0)?,
            preserve_colors: PreserveColors::from_id(value.preserve_colors)?,
            middle_grey: bounded("middle_grey", value.middle_grey, 0.05, 100.0)?,
            brightness: bounded("brightness", value.brightness, -4.0, 4.0)?,
            saturation: bounded("saturation", value.saturation, -1.0, 1.0)?,
            vibrance: bounded("vibrance", value.vibrance, -1.0, 1.0)?,
            clip: bounded("clip", value.clip, -1.0, 1.0)?,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(BasicAdjParametersV2::defaults()).expect("basicadj defaults are valid")
    }

    #[must_use]
    pub const fn black_point(self) -> f32 {
        self.black_point.get()
    }
    #[must_use]
    pub const fn exposure(self) -> f32 {
        self.exposure.get()
    }
    #[must_use]
    pub const fn hlcompr(self) -> f32 {
        self.hlcompr.get()
    }
    #[must_use]
    pub const fn hlcomprthresh(self) -> f32 {
        self.hlcomprthresh.get()
    }
    #[must_use]
    pub const fn contrast(self) -> f32 {
        self.contrast.get()
    }
    #[must_use]
    pub const fn preserve_colors(self) -> PreserveColors {
        self.preserve_colors
    }
    #[must_use]
    pub const fn middle_grey(self) -> f32 {
        self.middle_grey.get()
    }
    #[must_use]
    pub const fn brightness(self) -> f32 {
        self.brightness.get()
    }
    #[must_use]
    pub const fn saturation(self) -> f32 {
        self.saturation.get()
    }
    #[must_use]
    pub const fn vibrance(self) -> f32 {
        self.vibrance.get()
    }
    #[must_use]
    pub const fn clip(self) -> f32 {
        self.clip.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjConfigError {
    NonFinite(&'static str),
    OutOfRange(&'static str),
    UnknownPreserveColors(i32),
}

impl fmt::Display for BasicAdjConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFinite(name) => write!(formatter, "basicadj {name} is non-finite"),
            Self::OutOfRange(name) => write!(formatter, "basicadj {name} is outside its range"),
            Self::UnknownPreserveColors(value) => {
                write!(
                    formatter,
                    "basicadj preserve-colors mode {value} is unknown"
                )
            }
        }
    }
}

impl std::error::Error for BasicAdjConfigError {}

fn bounded(
    name: &'static str,
    value: f32,
    minimum: f32,
    maximum: f32,
) -> Result<FiniteF32, BasicAdjConfigError> {
    let value = FiniteF32::new(value).map_err(|_| BasicAdjConfigError::NonFinite(name))?;
    if !(minimum..=maximum).contains(&value.get()) {
        return Err(BasicAdjConfigError::OutOfRange(name));
    }
    Ok(value)
}

/// Immutable derived point-operation state.  The stage sequence is frozen in
/// `apply_pixel` and the identity covers both controls and derived constants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BasicAdjPlan {
    config: BasicAdjConfig,
    scale: FiniteF32,
    gamma: FiniteF32,
    middle_grey: FiniteF32,
    contrast: FiniteF32,
    hlcomp: FiniteF32,
    hlrange: FiniteF32,
    identity: [u8; 32],
}

impl BasicAdjPlan {
    pub fn new(config: BasicAdjConfig) -> Result<Self, BasicAdjPlanError> {
        let white = (-config.exposure()).exp2();
        let denominator = white - config.black_point();
        let scale = FiniteF32::new(1.0 / denominator)
            .map_err(|_| BasicAdjPlanError::InvalidExposureScale)?;
        let middle_grey = if config.middle_grey() > 0.0 {
            config.middle_grey() / 100.0
        } else {
            0.1842
        };
        let middle_grey = FiniteF32::new(middle_grey)
            .map_err(|_| BasicAdjPlanError::InvalidDerivedValue("middle_grey"))?;
        let brightness = config.brightness() * 2.0;
        let gamma = if brightness >= 0.0 {
            1.0 / (1.0 + brightness)
        } else {
            1.0 - brightness
        };
        let gamma =
            FiniteF32::new(gamma).map_err(|_| BasicAdjPlanError::InvalidDerivedValue("gamma"))?;
        let hlcomp = FiniteF32::from_proven_finite(config.hlcompr() / 100.0);
        let shoulder = config.hlcomprthresh() / 800.0 + 0.1;
        let hlrange = FiniteF32::new(1.0 - shoulder)
            .map_err(|_| BasicAdjPlanError::InvalidDerivedValue("highlight range"))?;
        let contrast = FiniteF32::from_proven_finite(config.contrast() + 1.0);
        let identity = plan_identity(
            &config,
            scale,
            gamma,
            middle_grey,
            contrast,
            hlcomp,
            hlrange,
        );
        Ok(Self {
            config,
            scale,
            gamma,
            middle_grey,
            contrast,
            hlcomp,
            hlrange,
            identity,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    #[must_use]
    pub const fn config(&self) -> BasicAdjConfig {
        self.config
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
        pixel_index_offset: usize,
    ) -> Result<Vec<LinearRgb>, OperationExecutionError> {
        input
            .iter()
            .copied()
            .enumerate()
            .map(|(index, pixel)| {
                let values = self.apply_pixel(pixel);
                Ok(LinearRgb::new(
                    checked(values[0], pixel_index_offset + index, RgbChannel::Red)?,
                    checked(values[1], pixel_index_offset + index, RgbChannel::Green)?,
                    checked(values[2], pixel_index_offset + index, RgbChannel::Blue)?,
                ))
            })
            .collect()
    }

    fn apply_pixel(&self, pixel: LinearRgb) -> [f32; 3] {
        let mut values = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
        let black = self.config.black_point();
        for value in &mut values {
            *value = (*value - black) * self.scale.get();
        }

        if self.config.hlcompr() > 0.0 {
            let luminance = Self::norm(values, PreserveColors::Luminance);
            if luminance > 0.0 {
                let ratio = hlcurve(luminance, self.hlcomp.get(), self.hlrange.get());
                for value in &mut values {
                    *value *= ratio;
                }
            }
        }

        if self.config.brightness() != 0.0 {
            for value in &mut values {
                if *value > 0.0 {
                    *value = value.powf(self.gamma.get());
                }
            }
        }

        if self.config.preserve_colors() == PreserveColors::None && self.config.contrast() != 0.0 {
            for value in &mut values {
                if *value > 0.0 {
                    *value = (*value / self.middle_grey.get()).powf(self.contrast.get())
                        * self.middle_grey.get();
                }
            }
        } else if self.config.preserve_colors() != PreserveColors::None
            && self.config.contrast() != 0.0
        {
            let luminance = Self::norm(values, self.config.preserve_colors());
            if luminance > 0.0 {
                let contrast_luminance = (luminance / self.middle_grey.get())
                    .powf(self.contrast.get())
                    * self.middle_grey.get();
                let ratio = contrast_luminance / luminance;
                for value in &mut values {
                    *value *= ratio;
                }
            }
        }

        if self.config.saturation() != 0.0 || self.config.vibrance() != 0.0 {
            let average = (values[0] + values[1] + values[2]) / 3.0;
            let delta = ((values[0] - average).powi(2)
                + (values[1] - average).powi(2)
                + (values[2] - average).powi(2))
            .sqrt();
            let vibrance = self.config.vibrance() / 1.4;
            let boost = vibrance * (1.0 - delta.powf(vibrance.abs()));
            let factor = self.config.saturation() + 1.0 + boost;
            for value in &mut values {
                *value = average + factor * (*value - average);
            }
        }
        values
    }

    fn norm(values: [f32; 3], mode: PreserveColors) -> f32 {
        match mode {
            PreserveColors::None | PreserveColors::Average => {
                (values[0] + values[1] + values[2]) / 3.0
            }
            PreserveColors::Luminance => {
                values[0] * CAMERA_LUMINANCE[0]
                    + values[1] * CAMERA_LUMINANCE[1]
                    + values[2] * CAMERA_LUMINANCE[2]
            }
            PreserveColors::Max => values.into_iter().fold(f32::NEG_INFINITY, f32::max),
            PreserveColors::Sum => values.into_iter().sum(),
            PreserveColors::Norm => values
                .into_iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt(),
            PreserveColors::Power => {
                let squares = values.map(|value| value * value);
                (values[0] * squares[0] + values[1] * squares[1] + values[2] * squares[2])
                    / squares.into_iter().sum::<f32>()
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BasicAdjPlanError {
    InvalidExposureScale,
    InvalidDerivedValue(&'static str),
}

impl fmt::Display for BasicAdjPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidExposureScale => {
                formatter.write_str("basicadj exposure scale is non-finite")
            }
            Self::InvalidDerivedValue(name) => {
                write!(formatter, "basicadj derived {name} is invalid")
            }
        }
    }
}
impl std::error::Error for BasicAdjPlanError {}

fn checked(
    value: f32,
    pixel: usize,
    channel: RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
}

fn hlcurve(level: f32, hlcomp: f32, hlrange: f32) -> f32 {
    if hlcomp <= 0.0 {
        return 1.0;
    }
    let mut value = level + (hlrange - 1.0);
    if value == 0.0 {
        value = 0.000_001;
    }
    let mut y = value / hlrange * hlcomp;
    if y <= -1.0 {
        y = -0.999_999;
    }
    let ratio = hlrange / (value * hlcomp);
    y.ln_1p() * ratio
}

fn plan_identity(
    config: &BasicAdjConfig,
    scale: FiniteF32,
    gamma: FiniteF32,
    middle_grey: FiniteF32,
    contrast: FiniteF32,
    hlcomp: FiniteF32,
    hlrange: FiniteF32,
) -> [u8; 32] {
    let fields = [
        config.black_point(),
        config.exposure(),
        config.hlcompr(),
        config.hlcomprthresh(),
        config.contrast(),
        config.middle_grey(),
        config.brightness(),
        config.saturation(),
        config.vibrance(),
        config.clip(),
        scale.get(),
        gamma.get(),
        middle_grey.get(),
        contrast.get(),
        hlcomp.get(),
        hlrange.get(),
    ];
    let mut hasher = Sha256::new();
    hasher.update(BASICADJ_SCHEMA_VERSION.to_le_bytes());
    hasher.update(config.preserve_colors().id().to_le_bytes());
    for field in fields {
        hasher.update(field.to_bits().to_le_bytes());
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v1_migration_adds_neutral_vibrance() {
        assert_eq!(
            migrate_v1_to_v2(BasicAdjParametersV1::defaults())
                .vibrance
                .to_bits(),
            0.0_f32.to_bits()
        );
    }

    #[test]
    fn default_plan_is_identity_and_deterministic() {
        let plan = BasicAdjPlan::new(BasicAdjConfig::defaults()).expect("defaults");
        let pixel = LinearRgb::new(
            FiniteF32::new(0.2).expect("finite"),
            FiniteF32::new(0.4).expect("finite"),
            FiniteF32::new(0.6).expect("finite"),
        );
        assert_eq!(plan.execute(&[pixel], 0).expect("execution"), vec![pixel]);
        assert_eq!(
            plan.identity(),
            BasicAdjPlan::new(BasicAdjConfig::defaults())
                .expect("defaults")
                .identity()
        );
    }

    #[test]
    fn preserve_colors_changes_only_contrast_luminance() {
        let mut parameters = BasicAdjParametersV2::defaults();
        parameters.contrast = 1.0;
        let plan =
            BasicAdjPlan::new(BasicAdjConfig::new(parameters).expect("parameters")).expect("plan");
        let pixel = LinearRgb::new(
            FiniteF32::new(0.1).expect("finite"),
            FiniteF32::new(0.3).expect("finite"),
            FiniteF32::new(0.6).expect("finite"),
        );
        let output = plan.execute(&[pixel], 0).expect("execution")[0];
        let ratio = output.red().get() / pixel.red().get();
        assert!((output.green().get() / pixel.green().get() - ratio).abs() < 0.000_01);
        assert!((output.blue().get() / pixel.blue().get() - ratio).abs() < 0.000_01);
    }
}
