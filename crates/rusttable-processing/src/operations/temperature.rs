//! Darktable-compatible temperature/white-balance operation.
//!
//! The persisted form is deliberately coefficient-first.  Temperature and
//! tint are UI representations; the pixel paths consume the resolved,
//! green-normalized multipliers and never perform a mutable camera-database
//! lookup.

#![allow(
    clippy::cast_precision_loss,
    clippy::excessive_precision,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

use super::common::OperationExecutionError;
use crate::{FiniteF32 as ProcessingFiniteF32, LinearRgb};
use rusttable_color::{Adaptation, AdaptationMethod, WhitePoint};
use rusttable_image::{CfaColor, CfaDescriptor};
use sha2::{Digest, Sha256};
use std::fmt;

pub const TEMPERATURE_COMPATIBILITY_ID: &str = "temperature";
pub const TEMPERATURE_SCHEMA_VERSION: u16 = 4;
pub const LOWEST_TEMPERATURE_KELVIN: f32 = 1_901.0;
pub const HIGHEST_TEMPERATURE_KELVIN: f32 = 25_000.0;
pub const LOWEST_TINT: f32 = 0.135;
pub const HIGHEST_TINT: f32 = 2.326;
pub const MAXIMUM_MULTIPLIER: f32 = 8.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhiteBalanceSource {
    CameraReference,
    AsShot,
    DaylightReference,
    Preset,
    TemperatureTint,
    Spot,
    Custom,
}

impl WhiteBalanceSource {
    pub const fn tag(self) -> &'static str {
        match self {
            Self::CameraReference => "camera_reference",
            Self::AsShot => "as_shot",
            Self::DaylightReference => "daylight_reference",
            Self::Preset => "preset",
            Self::TemperatureTint => "temperature_tint",
            Self::Spot => "spot",
            Self::Custom => "custom",
        }
    }

    pub fn parse(value: &str) -> Result<Self, TemperatureConfigError> {
        match value {
            "camera_reference" | "camera-reference" | "reference" => Ok(Self::CameraReference),
            "as_shot" | "as-shot" => Ok(Self::AsShot),
            "daylight_reference" | "daylight-reference" | "d65" => Ok(Self::DaylightReference),
            "preset" => Ok(Self::Preset),
            "temperature_tint" | "temperature-tint" | "user" => Ok(Self::TemperatureTint),
            "spot" => Ok(Self::Spot),
            "custom" => Ok(Self::Custom),
            _ => Err(TemperatureConfigError::UnknownSource(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WhiteBalanceStage {
    PreDemosaic,
    PostDemosaic,
}

impl WhiteBalanceStage {
    pub const fn tag(self) -> &'static str {
        match self {
            Self::PreDemosaic => "pre_demosaic",
            Self::PostDemosaic => "post_demosaic",
        }
    }

    pub fn parse(value: &str) -> Result<Self, TemperatureConfigError> {
        match value {
            "pre_demosaic" | "pre-demosaic" | "raw" => Ok(Self::PreDemosaic),
            "post_demosaic" | "post-demosaic" | "rgb" => Ok(Self::PostDemosaic),
            _ => Err(TemperatureConfigError::UnknownStage(value.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PresetProvenance {
    camera_alias: String,
    preset_identifier: String,
    tuning: i16,
    source_table_revision: u64,
}

impl PresetProvenance {
    pub fn new(
        camera_alias: impl Into<String>,
        preset_identifier: impl Into<String>,
        tuning: i16,
        source_table_revision: u64,
    ) -> Result<Self, TemperatureConfigError> {
        let camera_alias = camera_alias.into();
        let preset_identifier = preset_identifier.into();
        if camera_alias.is_empty() || preset_identifier.is_empty() {
            return Err(TemperatureConfigError::MissingPresetProvenance);
        }
        if camera_alias.len() > 512 || preset_identifier.len() > 256 {
            return Err(TemperatureConfigError::PresetProvenanceTooLong);
        }
        Ok(Self {
            camera_alias,
            preset_identifier,
            tuning,
            source_table_revision,
        })
    }

    pub fn camera_alias(&self) -> &str {
        &self.camera_alias
    }

    pub fn preset_identifier(&self) -> &str {
        &self.preset_identifier
    }

    pub const fn tuning(&self) -> i16 {
        self.tuning
    }

    pub const fn source_table_revision(&self) -> u64 {
        self.source_table_revision
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelMultipliers {
    red: ProcessingFiniteF32,
    green: ProcessingFiniteF32,
    blue: ProcessingFiniteF32,
    spare: ProcessingFiniteF32,
}

impl ChannelMultipliers {
    /// Creates canonical multipliers.  The green channel is the normalization
    /// reference and must be exactly one in the persisted representation.
    pub fn new(values: [f32; 4]) -> Result<Self, MultiplierError> {
        let values = values
            .into_iter()
            .map(bounded_multiplier)
            .collect::<Result<Vec<_>, _>>()?;
        let values: [ProcessingFiniteF32; 4] = values
            .try_into()
            .map_err(|_| MultiplierError::InvalidChannelCount)?;
        if values[1].get().to_bits() != 1.0_f32.to_bits() {
            return Err(MultiplierError::GreenNotNormalized);
        }
        Ok(Self {
            red: values[0],
            green: values[1],
            blue: values[2],
            spare: values[3],
        })
    }

    /// Converts Darktable-style coefficients to the canonical green-normalized form.
    pub fn from_coefficients(values: [f32; 4]) -> Result<Self, MultiplierError> {
        let values = values
            .into_iter()
            .map(|value| ProcessingFiniteF32::new(value).map_err(|_| MultiplierError::NonFinite))
            .collect::<Result<Vec<_>, _>>()?;
        let values: [ProcessingFiniteF32; 4] = values
            .try_into()
            .map_err(|_| MultiplierError::InvalidChannelCount)?;
        if values.iter().any(|value| value.get() <= 0.0) {
            return Err(MultiplierError::NotPositive);
        }
        let green = values[1].get();
        let normalized = values.map(|value| value.get() / green);
        Self::new(normalized)
    }

    pub const fn red(self) -> ProcessingFiniteF32 {
        self.red
    }

    pub const fn green(self) -> ProcessingFiniteF32 {
        self.green
    }

    pub const fn blue(self) -> ProcessingFiniteF32 {
        self.blue
    }

    pub const fn spare(self) -> ProcessingFiniteF32 {
        self.spare
    }

    pub const fn as_array(self) -> [ProcessingFiniteF32; 4] {
        [self.red, self.green, self.blue, self.spare]
    }

    fn for_cfa_color(self, color: CfaColor) -> ProcessingFiniteF32 {
        match color {
            CfaColor::Red => self.red,
            CfaColor::Green => self.green,
            CfaColor::Blue => self.blue,
            CfaColor::Clear => self.spare,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TemperatureTint {
    temperature_kelvin: ProcessingFiniteF32,
    tint: ProcessingFiniteF32,
}

impl TemperatureTint {
    pub fn new(temperature_kelvin: f32, tint: f32) -> Result<Self, TemperatureConversionError> {
        let temperature_kelvin = bounded(
            temperature_kelvin,
            LOWEST_TEMPERATURE_KELVIN,
            HIGHEST_TEMPERATURE_KELVIN,
            TemperatureConversionError::TemperatureOutOfRange,
        )?;
        let tint = bounded(
            tint,
            LOWEST_TINT,
            HIGHEST_TINT,
            TemperatureConversionError::TintOutOfRange,
        )?;
        Ok(Self {
            temperature_kelvin,
            tint,
        })
    }

    pub const fn temperature_kelvin(self) -> ProcessingFiniteF32 {
        self.temperature_kelvin
    }

    pub const fn tint(self) -> ProcessingFiniteF32 {
        self.tint
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TemperatureConfig {
    multipliers: ChannelMultipliers,
    source: WhiteBalanceSource,
    stage: WhiteBalanceStage,
    temperature_tint: Option<TemperatureTint>,
    preset_provenance: Option<PresetProvenance>,
}

impl TemperatureConfig {
    pub fn new(
        multipliers: ChannelMultipliers,
        source: WhiteBalanceSource,
    ) -> Result<Self, TemperatureConfigError> {
        Self::with_details(
            multipliers,
            source,
            WhiteBalanceStage::PreDemosaic,
            None,
            None,
        )
    }

    pub fn with_details(
        multipliers: ChannelMultipliers,
        source: WhiteBalanceSource,
        stage: WhiteBalanceStage,
        temperature_tint: Option<TemperatureTint>,
        preset_provenance: Option<PresetProvenance>,
    ) -> Result<Self, TemperatureConfigError> {
        if matches!(source, WhiteBalanceSource::Preset) && preset_provenance.is_none() {
            return Err(TemperatureConfigError::MissingPresetProvenance);
        }
        if !matches!(source, WhiteBalanceSource::TemperatureTint) && temperature_tint.is_some() {
            return Err(TemperatureConfigError::UnexpectedTemperatureTint);
        }
        Ok(Self {
            multipliers,
            source,
            stage,
            temperature_tint,
            preset_provenance,
        })
    }

    pub fn from_coefficients(
        coefficients: [f32; 4],
        source: WhiteBalanceSource,
    ) -> Result<Self, TemperatureConfigError> {
        Self::new(ChannelMultipliers::from_coefficients(coefficients)?, source)
    }

    pub const fn multipliers(&self) -> ChannelMultipliers {
        self.multipliers
    }

    pub const fn source(&self) -> WhiteBalanceSource {
        self.source
    }

    pub const fn stage(&self) -> WhiteBalanceStage {
        self.stage
    }

    pub const fn temperature_tint(&self) -> Option<TemperatureTint> {
        self.temperature_tint
    }

    pub fn preset_provenance(&self) -> Option<&PresetProvenance> {
        self.preset_provenance.as_ref()
    }
}

impl Default for TemperatureConfig {
    fn default() -> Self {
        Self::new(
            ChannelMultipliers::new([1.0; 4]).expect("identity multipliers"),
            WhiteBalanceSource::AsShot,
        )
        .expect("valid default temperature config")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemperatureConfigError {
    Multiplier(MultiplierError),
    UnknownSource(String),
    UnknownStage(String),
    MissingPresetProvenance,
    PresetProvenanceTooLong,
    UnexpectedTemperatureTint,
}

impl fmt::Display for TemperatureConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Multiplier(error) => {
                write!(formatter, "invalid white-balance multiplier: {error}")
            }
            Self::UnknownSource(value) => write!(formatter, "unknown white-balance source {value}"),
            Self::UnknownStage(value) => write!(formatter, "unknown white-balance stage {value}"),
            Self::MissingPresetProvenance => formatter.write_str("preset provenance is required"),
            Self::PresetProvenanceTooLong => {
                formatter.write_str("preset provenance exceeds its bound")
            }
            Self::UnexpectedTemperatureTint => formatter
                .write_str("temperature/tint is only valid for the temperature-tint source"),
        }
    }
}

impl std::error::Error for TemperatureConfigError {}

impl From<MultiplierError> for TemperatureConfigError {
    fn from(error: MultiplierError) -> Self {
        Self::Multiplier(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiplierError {
    NonFinite,
    NotPositive,
    AboveMaximum,
    GreenNotNormalized,
    InvalidChannelCount,
}

impl fmt::Display for MultiplierError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "multiplier is non-finite",
            Self::NotPositive => "multiplier must be positive",
            Self::AboveMaximum => "multiplier is above 8.0",
            Self::GreenNotNormalized => "green multiplier must be exactly 1.0",
            Self::InvalidChannelCount => "four channel multipliers are required",
        })
    }
}

impl std::error::Error for MultiplierError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemperatureConversionError {
    NonFinite,
    TemperatureOutOfRange,
    TintOutOfRange,
    InvalidChromaticity,
    InvalidMatrix,
    InvalidMultipliers,
}

impl fmt::Display for TemperatureConversionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "white-balance conversion produced a non-finite value",
            Self::TemperatureOutOfRange => "temperature is outside 1901..=25000 K",
            Self::TintOutOfRange => "tint is outside 0.135..=2.326",
            Self::InvalidChromaticity => "temperature conversion produced invalid chromaticity",
            Self::InvalidMatrix => "temperature conversion matrix is invalid",
            Self::InvalidMultipliers => "multipliers cannot be converted to temperature/tint",
        })
    }
}

impl std::error::Error for TemperatureConversionError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureExecution {
    pixels: Vec<LinearRgb>,
    receipt: WhiteBalanceReceipt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhiteBalanceReceipt {
    source: WhiteBalanceSource,
    stage: WhiteBalanceStage,
    multipliers: ChannelMultipliers,
    identity: [u8; 32],
}

impl WhiteBalanceReceipt {
    pub const fn source(self) -> WhiteBalanceSource {
        self.source
    }

    pub const fn stage(self) -> WhiteBalanceStage {
        self.stage
    }

    pub const fn multipliers(self) -> ChannelMultipliers {
        self.multipliers
    }

    pub const fn identity(self) -> [u8; 32] {
        self.identity
    }
}

impl TemperatureExecution {
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub const fn receipt(&self) -> WhiteBalanceReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperatureRawExecution {
    samples: Vec<ProcessingFiniteF32>,
    dimensions: rusttable_image::ImageDimensions,
    cfa: CfaDescriptor,
    receipt: WhiteBalanceReceipt,
}

impl TemperatureRawExecution {
    pub fn samples(&self) -> &[ProcessingFiniteF32] {
        &self.samples
    }

    pub const fn dimensions(&self) -> rusttable_image::ImageDimensions {
        self.dimensions
    }

    pub const fn cfa(&self) -> CfaDescriptor {
        self.cfa
    }

    pub const fn receipt(&self) -> WhiteBalanceReceipt {
        self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemperaturePlanError {
    Config(TemperatureConfigError),
    Conversion(TemperatureConversionError),
    Serialization,
}

impl fmt::Display for TemperaturePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(formatter, "temperature configuration error: {error}"),
            Self::Conversion(error) => write!(formatter, "temperature conversion error: {error}"),
            Self::Serialization => {
                formatter.write_str("temperature plan identity serialization failed")
            }
        }
    }
}

impl std::error::Error for TemperaturePlanError {}

impl From<TemperatureConfigError> for TemperaturePlanError {
    fn from(error: TemperatureConfigError) -> Self {
        Self::Config(error)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemperaturePlan {
    config: TemperatureConfig,
    receipt: WhiteBalanceReceipt,
}

impl TemperaturePlan {
    pub fn new(config: TemperatureConfig) -> Result<Self, TemperaturePlanError> {
        let identity = plan_identity(&config);
        let receipt = WhiteBalanceReceipt {
            source: config.source(),
            stage: config.stage(),
            multipliers: config.multipliers(),
            identity,
        };
        Ok(Self { config, receipt })
    }

    pub const fn config(&self) -> &TemperatureConfig {
        &self.config
    }

    pub const fn receipt(&self) -> WhiteBalanceReceipt {
        self.receipt
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<TemperatureExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<TemperatureExecution, OperationExecutionError> {
        let multipliers = self.config.multipliers();
        let mut pixels = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let values = [
                pixel.red().get() * multipliers.red().get(),
                pixel.green().get() * multipliers.green().get(),
                pixel.blue().get() * multipliers.blue().get(),
            ];
            let values = values
                .map(ProcessingFiniteF32::new)
                .into_iter()
                .enumerate()
                .map(|(channel, value)| {
                    value.map_err(|_| OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: match channel {
                            0 => crate::RgbChannel::Red,
                            1 => crate::RgbChannel::Green,
                            _ => crate::RgbChannel::Blue,
                        },
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            pixels.push(LinearRgb::new(values[0], values[1], values[2]));
        }
        Ok(TemperatureExecution {
            pixels,
            receipt: self.receipt,
        })
    }

    /// Applies the pre-demosaic path using typed CFA metadata and phase.
    pub fn execute_raw(
        &self,
        input: &crate::NormalizedRaw,
    ) -> Result<TemperatureRawExecution, OperationExecutionError> {
        self.execute_raw_with_cancel(input, || false)
    }

    pub fn execute_raw_with_cancel<F: Fn() -> bool>(
        &self,
        input: &crate::NormalizedRaw,
        cancelled: F,
    ) -> Result<TemperatureRawExecution, OperationExecutionError> {
        if self.config.stage() != WhiteBalanceStage::PreDemosaic {
            return Err(OperationExecutionError::NoReconstructionEvidence);
        }
        let width = usize::try_from(input.dimensions().width()).map_err(|_| {
            OperationExecutionError::DimensionsMismatch {
                expected: usize::MAX,
                actual: input.samples().len(),
            }
        })?;
        let cfa = input.cfa();
        let multipliers = self.config.multipliers();
        let mut samples = Vec::with_capacity(input.samples().len());
        for (index, sample) in input.samples().iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let x = u32::try_from(index % width).map_err(|_| {
                OperationExecutionError::DimensionsMismatch {
                    expected: input.samples().len(),
                    actual: input.samples().len(),
                }
            })?;
            let y = u32::try_from(index / width).map_err(|_| {
                OperationExecutionError::DimensionsMismatch {
                    expected: input.samples().len(),
                    actual: input.samples().len(),
                }
            })?;
            let multiplier = multipliers.for_cfa_color(cfa.pattern().color_at(x, y, cfa.phase()));
            let value =
                ProcessingFiniteF32::new(sample.get() * multiplier.get()).map_err(|_| {
                    OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: crate::RgbChannel::Green,
                    }
                })?;
            samples.push(value);
        }
        Ok(TemperatureRawExecution {
            samples,
            dimensions: input.dimensions(),
            cfa,
            receipt: self.receipt,
        })
    }
}

/// Converts a Darktable temperature/tint UI pair into canonical multipliers.
pub fn temperature_tint_to_multipliers(
    temperature_kelvin: f32,
    tint: f32,
) -> Result<ChannelMultipliers, TemperatureConversionError> {
    let pair = TemperatureTint::new(temperature_kelvin, tint)?;
    let xyz = temperature_tint_to_xyz(pair)?;
    xyz_to_multipliers(xyz)
}

/// Converts canonical multipliers back to the bounded Darktable UI pair.
pub fn multipliers_to_temperature_tint(
    multipliers: ChannelMultipliers,
) -> Result<TemperatureTint, TemperatureConversionError> {
    let camera = [
        1.0 / multipliers.red().get(),
        1.0 / multipliers.green().get(),
        1.0 / multipliers.blue().get(),
    ];
    let xyz = camera;
    if xyz.iter().any(|value| !value.is_finite()) || xyz[0] <= 0.0 || xyz[1] <= 0.0 || xyz[2] <= 0.0
    {
        return Err(TemperatureConversionError::InvalidMultipliers);
    }
    let target_ratio = xyz[2] / xyz[0];
    let mut low = LOWEST_TEMPERATURE_KELVIN;
    let mut high = HIGHEST_TEMPERATURE_KELVIN;
    for _ in 0..32 {
        let middle = low.midpoint(high);
        let ratio = temperature_to_xyz(middle)?[2] / temperature_to_xyz(middle)?[0];
        if ratio > target_ratio {
            high = middle;
        } else {
            low = middle;
        }
    }
    let temperature = low.midpoint(high);
    let base = temperature_to_xyz(temperature)?;
    let tint = (base[1] / base[0]) / (xyz[1] / xyz[0]);
    TemperatureTint::new(temperature, tint)
}

fn bounded<E>(value: f32, minimum: f32, maximum: f32, error: E) -> Result<ProcessingFiniteF32, E>
where
    E: Copy,
{
    let value = ProcessingFiniteF32::new(value).map_err(|_| error)?;
    if (minimum..=maximum).contains(&value.get()) {
        Ok(value)
    } else {
        Err(error)
    }
}

fn bounded_multiplier(value: f32) -> Result<ProcessingFiniteF32, MultiplierError> {
    let value = ProcessingFiniteF32::new(value).map_err(|_| MultiplierError::NonFinite)?;
    if value.get() <= 0.0 {
        return Err(MultiplierError::NotPositive);
    }
    if value.get() > MAXIMUM_MULTIPLIER {
        return Err(MultiplierError::AboveMaximum);
    }
    Ok(value)
}

fn temperature_tint_to_xyz(pair: TemperatureTint) -> Result<[f32; 3], TemperatureConversionError> {
    let mut xyz = temperature_to_xyz(pair.temperature_kelvin().get())?;
    xyz[1] /= pair.tint().get();
    if xyz.iter().any(|value| !value.is_finite()) {
        return Err(TemperatureConversionError::NonFinite);
    }
    Ok(xyz)
}

fn xyz_to_multipliers(xyz: [f32; 3]) -> Result<ChannelMultipliers, TemperatureConversionError> {
    if xyz.iter().any(|value| !value.is_finite() || *value <= 0.0) {
        return Err(TemperatureConversionError::InvalidChromaticity);
    }
    ChannelMultipliers::from_coefficients([1.0 / xyz[0], 1.0 / xyz[1], 1.0 / xyz[2], 1.0 / xyz[1]])
        .map_err(|_| TemperatureConversionError::InvalidChromaticity)
}

/// Uses the same blackbody/daylight split as Darktable, with the published
/// CIE approximation for the blackbody/daylight chromaticity locus.  Tint is
/// applied to Y exactly as Darktable's compatibility representation does.
fn temperature_to_xyz(temperature_kelvin: f32) -> Result<[f32; 3], TemperatureConversionError> {
    if !(LOWEST_TEMPERATURE_KELVIN..=HIGHEST_TEMPERATURE_KELVIN).contains(&temperature_kelvin)
        || !temperature_kelvin.is_finite()
    {
        return Err(TemperatureConversionError::TemperatureOutOfRange);
    }
    let inverse = 1.0 / temperature_kelvin;
    let x = if temperature_kelvin < 4_000.0 {
        -0.266_123_9e9 * inverse.powi(3) - 0.234_358_9e6 * inverse.powi(2)
            + 0.877_695_6e3 * inverse
            + 0.179_910
    } else {
        -3.025_846_9e9 * inverse.powi(3)
            + 2.107_037_9e6 * inverse.powi(2)
            + 0.222_634_7e3 * inverse
            + 0.240_390
    };
    let y = -1.106_381_4 * x.powi(3) - 1.348_110_2 * x.powi(2) + 2.185_558_32 * x - 0.202_196_83;
    let white =
        WhitePoint::custom(x, y).map_err(|_| TemperatureConversionError::InvalidChromaticity)?;
    let adaptation = Adaptation::between(WhitePoint::D65, white, AdaptationMethod::Bradford)
        .map_err(|_| TemperatureConversionError::InvalidMatrix)?;
    let [x, y, z] = adaptation.matrix().apply(WhitePoint::D65.xyz());
    let maximum = x.max(y).max(z);
    let xyz = [x / maximum, y / maximum, z / maximum];
    if xyz.iter().any(|value| !value.is_finite()) {
        Err(TemperatureConversionError::NonFinite)
    } else {
        Ok(xyz)
    }
}

fn plan_identity(config: &TemperatureConfig) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(TEMPERATURE_COMPATIBILITY_ID.as_bytes());
    hasher.update(TEMPERATURE_SCHEMA_VERSION.to_le_bytes());
    hasher.update(config.source().tag().as_bytes());
    hasher.update(config.stage().tag().as_bytes());
    for multiplier in config.multipliers().as_array() {
        hasher.update(multiplier.get().to_bits().to_le_bytes());
    }
    if let Some(pair) = config.temperature_tint() {
        hasher.update(pair.temperature_kelvin().get().to_bits().to_le_bytes());
        hasher.update(pair.tint().get().to_bits().to_le_bytes());
    }
    if let Some(provenance) = config.preset_provenance() {
        hasher.update(provenance.camera_alias().as_bytes());
        hasher.update(provenance.preset_identifier().as_bytes());
        hasher.update(provenance.tuning().to_le_bytes());
        hasher.update(provenance.source_table_revision().to_le_bytes());
    }
    hasher.finalize().into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureLegacyParametersV2 {
    pub temp_out: f32,
    pub coefficients: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureLegacyParametersV3 {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub various: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TemperatureLegacyParametersV4 {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub various: f32,
    pub preset: i32,
}

pub fn migrate_v2(value: TemperatureLegacyParametersV2) -> TemperatureLegacyParametersV4 {
    TemperatureLegacyParametersV4 {
        red: value.coefficients[0],
        green: value.coefficients[1],
        blue: value.coefficients[2],
        various: 1.0,
        preset: -1,
    }
}

pub fn migrate_v3(value: TemperatureLegacyParametersV3) -> TemperatureLegacyParametersV4 {
    TemperatureLegacyParametersV4 {
        red: value.red,
        green: value.green,
        blue: value.blue,
        various: if value.various.is_finite() {
            value.various
        } else {
            1.0
        },
        preset: -1,
    }
}

pub fn migrate_v4(value: TemperatureLegacyParametersV4) -> TemperatureLegacyParametersV4 {
    value
}
