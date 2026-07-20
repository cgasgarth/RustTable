#![allow(
    clippy::match_same_arms,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::needless_pass_by_value
)]

use super::common::OperationExecutionError;
use crate::{FiniteF32 as ProcessingFiniteF32, LinearRgb};
use rusttable_color::{
    Adaptation, AdaptationMethod, AlphaTransform, BlackPointCompensation,
    BuiltinColorTransformPlanner, BuiltinSpace, ColorEncoding, ColorRole, ColorTransformPlanner,
    ColorTransformRequest, ExtendedRange, Matrix3, Precision, ProfileId, RenderingIntent,
    TransferFunction, TransformExecutionError, TransformPlan, TransformStep, WhitePoint,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const COLORIN_COMPATIBILITY_ID: &str = "colorin";
pub const COLORIN_SCHEMA_VERSION: u16 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorInProfile {
    Builtin(BuiltinSpace),
    Matrix {
        id: ProfileId,
        primaries: rusttable_color::Primaries,
        transfer: TransferFunction,
    },
    Missing(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorInNormalization {
    Off,
    Srgb,
    AdobeRgb,
    LinearRec709,
    LinearRec2020,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColorInConfig {
    input: ColorInProfile,
    working: ColorInProfile,
    intent: RenderingIntent,
    normalization: ColorInNormalization,
    blue_mapping: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorInConfigError {
    MissingProfileEvidence,
    UnsupportedProfile,
    InvalidIntent,
    InvalidNormalization,
}

impl fmt::Display for ColorInConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingProfileEvidence => "selected color profile evidence is unavailable",
            Self::UnsupportedProfile => "selected color profile is unsupported",
            Self::InvalidIntent => "colorin intent is invalid",
            Self::InvalidNormalization => "colorin normalization mode is invalid",
        })
    }
}
impl std::error::Error for ColorInConfigError {}

impl ColorInConfig {
    pub fn new(
        input: ColorInProfile,
        working: ColorInProfile,
        intent: RenderingIntent,
        normalization: ColorInNormalization,
        blue_mapping: bool,
    ) -> Result<Self, ColorInConfigError> {
        if matches!(&input, ColorInProfile::Missing(_))
            || matches!(&working, ColorInProfile::Missing(_))
        {
            return Err(ColorInConfigError::MissingProfileEvidence);
        }
        Ok(Self {
            input,
            working,
            intent,
            normalization,
            blue_mapping,
        })
    }

    #[must_use]
    pub fn builtin(input: BuiltinSpace, working: BuiltinSpace) -> Self {
        Self::new(
            input.into(),
            working.into(),
            RenderingIntent::Perceptual,
            ColorInNormalization::Off,
            true,
        )
        .unwrap_or_else(|_| unreachable!())
    }
    #[must_use]
    pub const fn input(&self) -> &ColorInProfile {
        &self.input
    }
    #[must_use]
    pub const fn working(&self) -> &ColorInProfile {
        &self.working
    }
    #[must_use]
    pub const fn intent(&self) -> RenderingIntent {
        self.intent
    }
    #[must_use]
    pub const fn normalization(&self) -> ColorInNormalization {
        self.normalization
    }
    #[must_use]
    pub const fn blue_mapping(&self) -> bool {
        self.blue_mapping
    }
}

impl Default for ColorInConfig {
    fn default() -> Self {
        Self::builtin(BuiltinSpace::SrgbD65, BuiltinSpace::Rec2020D65)
    }
}

impl From<BuiltinSpace> for ColorInProfile {
    fn from(value: BuiltinSpace) -> Self {
        Self::Builtin(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorInPlanError {
    Config(ColorInConfigError),
    Request(rusttable_color::ColorTransformRequestError),
    Adaptation(rusttable_color::MatrixErrorAdapter),
    Plan(rusttable_color::TransformPlanError),
    Matrix,
    Serialization(String),
}
impl fmt::Display for ColorInPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "colorin plan error: {self:?}")
    }
}
impl std::error::Error for ColorInPlanError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorInPlan {
    config: ColorInConfig,
    transform: TransformPlan,
    output_encoding: ColorEncoding,
    output_primaries: Option<rusttable_color::Primaries>,
    identity: [u8; 32],
}

impl ColorInPlan {
    pub fn new(config: ColorInConfig) -> Result<Self, ColorInPlanError> {
        let (source_encoding, source_matrix, source_white, source_transfer) =
            profile_parts(&config.input, false)?;
        // The working-space side of colorin is an internal linear space. Keep
        // its encoding linear so the planner does not append a display
        // transfer encode to the processing graph.
        let (target_encoding, target_matrix, target_white, _) =
            profile_parts(&config.working, true)?;
        let request = ColorTransformRequest::new(
            source_encoding,
            target_encoding,
            ColorRole::Input,
            config.intent,
            BlackPointCompensation::Disabled,
            AdaptationMethod::Bradford,
            Precision::F32,
            AlphaTransform::Preserve,
            ExtendedRange::Extended,
            COLORIN_SCHEMA_VERSION,
        )
        .map_err(ColorInPlanError::Request)?;
        let transform = if matches!(
            (&config.input, &config.working),
            (ColorInProfile::Builtin(_), ColorInProfile::Builtin(_))
        ) {
            BuiltinColorTransformPlanner
                .plan(&request)
                .map_err(|error| match error {
                    rusttable_color::PlannerError::Adaptation(error) => {
                        ColorInPlanError::Adaptation(error)
                    }
                    rusttable_color::PlannerError::Plan(error) => ColorInPlanError::Plan(error),
                    rusttable_color::PlannerError::Request(error) => {
                        ColorInPlanError::Request(error)
                    }
                    _ => ColorInPlanError::Matrix,
                })?
        } else {
            let mut steps = Vec::with_capacity(4);
            if !matches!(source_transfer, TransferFunction::Linear) {
                steps.push(TransformStep::Transfer {
                    function: source_transfer,
                    decode: true,
                });
            }
            steps.push(TransformStep::Matrix(source_matrix));
            if source_white != target_white {
                steps.push(TransformStep::Adaptation(
                    Adaptation::between(source_white, target_white, AdaptationMethod::Bradford)
                        .map_err(ColorInPlanError::Adaptation)?,
                ));
            }
            steps.push(TransformStep::Matrix(
                target_matrix
                    .inverse()
                    .map_err(|_| ColorInPlanError::Matrix)?,
            ));
            TransformPlan::new(request, steps).map_err(ColorInPlanError::Plan)?
        };
        let identity = plan_identity(&config, &transform)?;
        Ok(Self {
            output_encoding: target_encoding,
            output_primaries: profile_primaries(&config.working),
            config,
            transform,
            identity,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &ColorInConfig {
        &self.config
    }
    #[must_use]
    pub const fn transform(&self) -> &TransformPlan {
        &self.transform
    }
    #[must_use]
    pub const fn output_encoding(&self) -> ColorEncoding {
        self.output_encoding
    }
    #[must_use]
    pub const fn output_primaries(&self) -> Option<rusttable_color::Primaries> {
        self.output_primaries
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ColorInExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }

    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorInExecution, OperationExecutionError> {
        let mut output = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let transformed = self
                .transform
                .apply_rgb(
                    [pixel.red().get(), pixel.green().get(), pixel.blue().get()],
                    &cancelled,
                )
                .map_err(|error| match error {
                    TransformExecutionError::Cancelled => OperationExecutionError::Cancelled,
                    _ => OperationExecutionError::NonFiniteResult {
                        pixel: index,
                        channel: crate::RgbChannel::Red,
                    },
                })?;
            let transformed = apply_blue_mapping(transformed, self.config.blue_mapping);
            let transformed = normalize(transformed, self.config.normalization);
            let red = ProcessingFiniteF32::new(transformed[0]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Red,
                }
            })?;
            let green = ProcessingFiniteF32::new(transformed[1]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Green,
                }
            })?;
            let blue = ProcessingFiniteF32::new(transformed[2]).map_err(|_| {
                OperationExecutionError::NonFiniteResult {
                    pixel: index,
                    channel: crate::RgbChannel::Blue,
                }
            })?;
            output.push(LinearRgb::new(red, green, blue));
        }
        let receipt = ExecutionReceipt::new(self.identity, input, &output);
        Ok(ColorInExecution {
            pixels: output,
            receipt,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorInExecution {
    pixels: Vec<LinearRgb>,
    receipt: ExecutionReceipt,
}
impl ColorInExecution {
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
    fn new(plan_identity: [u8; 32], input: &[LinearRgb], output: &[LinearRgb]) -> Self {
        Self {
            plan_identity,
            input_digest: digest(input, b"rusttable.colorin.input.v1"),
            output_digest: digest(output, b"rusttable.colorin.output.v1"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorInLegacyParameters {
    pub input_profile: String,
    pub working_profile: Option<String>,
    pub intent: i64,
    pub normalization: i64,
    pub blue_mapping: Option<bool>,
}

pub fn migrate(
    version: u16,
    old: ColorInLegacyParameters,
) -> Result<ColorInConfig, ColorInConfigError> {
    if !(1..=COLORIN_SCHEMA_VERSION).contains(&version) {
        return Err(ColorInConfigError::UnsupportedProfile);
    }
    let input = parse_profile(&old.input_profile)?;
    let working = parse_profile(
        old.working_profile
            .as_deref()
            .unwrap_or("linear_rec2020_rgb"),
    )?;
    let intent = match old.intent {
        0 => RenderingIntent::Perceptual,
        1 => RenderingIntent::Relative,
        2 => RenderingIntent::Saturation,
        3 => RenderingIntent::Absolute,
        _ => return Err(ColorInConfigError::InvalidIntent),
    };
    let normalization = match old.normalization {
        0 => ColorInNormalization::Off,
        1 => ColorInNormalization::Srgb,
        2 => ColorInNormalization::AdobeRgb,
        3 => ColorInNormalization::LinearRec709,
        4 => ColorInNormalization::LinearRec2020,
        _ => return Err(ColorInConfigError::InvalidNormalization),
    };
    ColorInConfig::new(
        input,
        working,
        intent,
        normalization,
        old.blue_mapping.unwrap_or(true),
    )
}

pub fn parse_profile(reference: &str) -> Result<ColorInProfile, ColorInConfigError> {
    let normalized = reference.to_ascii_lowercase();
    let space = match normalized.as_str() {
        "srgb" | "builtin:srgb" | "sprofile" => BuiltinSpace::SrgbD65,
        "linear_rgb" | "linear_rec709_rgb" | "builtin:linear-srgb" => BuiltinSpace::SrgbD65,
        "linear_rec2020_rgb" | "builtin:linear-rec2020" => BuiltinSpace::Rec2020D65,
        "display_p3" | "builtin:display-p3" => BuiltinSpace::DisplayP3D65,
        "aces_cg" | "builtin:aces-cg" => BuiltinSpace::AcesCgD60,
        "eprofile" | "ematrix" | "cmatrix" | "darktable" | "vendor" | "alternate" => {
            return Err(ColorInConfigError::MissingProfileEvidence);
        }
        _ => return Err(ColorInConfigError::MissingProfileEvidence),
    };
    Ok(ColorInProfile::Builtin(space))
}

fn profile_parts(
    profile: &ColorInProfile,
    linear: bool,
) -> Result<(ColorEncoding, Matrix3, WhitePoint, TransferFunction), ColorInPlanError> {
    match profile {
        ColorInProfile::Builtin(space) => Ok((
            space.encoding(linear),
            space.to_xyz_matrix().ok_or(ColorInPlanError::Matrix)?,
            space.white_point(),
            space.transfer(),
        )),
        ColorInProfile::Matrix {
            id,
            primaries,
            transfer,
        } => Ok((
            ColorEncoding::External(*id),
            rusttable_color::rgb_to_xyz_matrix(
                [
                    pair(primaries.red()),
                    pair(primaries.green()),
                    pair(primaries.blue()),
                ],
                primaries.white(),
            )
            .map_err(|_| ColorInPlanError::Matrix)?,
            primaries.white(),
            *transfer,
        )),
        ColorInProfile::Missing(_) => Err(ColorInPlanError::Config(
            ColorInConfigError::MissingProfileEvidence,
        )),
    }
}

fn profile_primaries(profile: &ColorInProfile) -> Option<rusttable_color::Primaries> {
    match profile {
        ColorInProfile::Builtin(space) => space.primaries(),
        ColorInProfile::Matrix { primaries, .. } => Some(*primaries),
        ColorInProfile::Missing(_) => None,
    }
}

fn apply_blue_mapping(mut value: [f32; 3], enabled: bool) -> [f32; 3] {
    if enabled {
        let sum = value[0] + value[1] + value[2];
        if sum > 0.0 {
            let blue_fraction = value[2] / sum;
            if blue_fraction > 0.5 {
                let amount = 0.11 * ((blue_fraction - 0.5) / 0.5) * (sum / 0.5).min(1.0);
                value[1] += amount;
                value[2] -= amount;
            }
        }
    }
    value
}

fn normalize(mut value: [f32; 3], mode: ColorInNormalization) -> [f32; 3] {
    if !matches!(mode, ColorInNormalization::Off) {
        value = value.map(|channel| channel.clamp(0.0, 1.0));
    }
    value
}

fn pair(value: (rusttable_color::FiniteF32, rusttable_color::FiniteF32)) -> (f32, f32) {
    (value.0.get(), value.1.get())
}
fn plan_identity(
    config: &ColorInConfig,
    transform: &TransformPlan,
) -> Result<[u8; 32], ColorInPlanError> {
    let bytes = postcard::to_allocvec(&(COLORIN_SCHEMA_VERSION, config, transform))
        .map_err(|error| ColorInPlanError::Serialization(error.to_string()))?;
    Ok(Sha256::digest(bytes).into())
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
pub const fn wgpu_passes() -> [&'static str; 2] {
    ["colorin_matrix", "colorin_transfer"]
}
