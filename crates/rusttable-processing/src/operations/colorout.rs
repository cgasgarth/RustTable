#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

use super::common::OperationExecutionError;
use crate::{FiniteF32, LinearRgb, WorkingFrameDescriptor};
use rusttable_color::{
    Adaptation, AdaptationMethod, AlphaTransform, BlackPointCompensation,
    BuiltinColorTransformPlanner, BuiltinSpace, ColorEncoding, ColorRole, ColorTransformPlanner,
    ColorTransformRequest, ExtendedRange, Matrix3, Pcs, Precision, Primaries, ProfileClass,
    ProfileId, ProfileIdError, ProfileModel, ProfileParserVersion, RenderingIntent,
    TransferFunction, TransformExecutionError, TransformPlan, TransformPlanError, TransformStep,
    WhitePoint, rgb_to_xyz_matrix,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

pub const COLOROUT_COMPATIBILITY_ID: &str = "colorout";
pub const COLOROUT_SCHEMA_VERSION: u16 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorOutProfile {
    Builtin(BuiltinSpace),
    Matrix {
        id: ProfileId,
        primaries: Primaries,
        transfer: TransferFunction,
    },
    Missing(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorOutGamutMode {
    None,
    Warning,
    Clip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ColorOutExecutor {
    Cpu,
    WgpuMatrix,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ColorOutConfig {
    output: ColorOutProfile,
    intent: RenderingIntent,
    black_point_compensation: BlackPointCompensation,
    proof: Option<ColorOutProfile>,
    gamut: ColorOutGamutMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorOutConfigError {
    MissingProfileEvidence,
    UnsupportedProfile,
    InvalidIntent,
}
impl fmt::Display for ColorOutConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingProfileEvidence => "output profile evidence is unavailable",
            Self::UnsupportedProfile => "output profile is unsupported",
            Self::InvalidIntent => "colorout intent is invalid",
        })
    }
}
impl std::error::Error for ColorOutConfigError {}

impl ColorOutConfig {
    pub fn new(
        output: ColorOutProfile,
        intent: RenderingIntent,
        black_point_compensation: BlackPointCompensation,
        proof: Option<ColorOutProfile>,
        gamut: ColorOutGamutMode,
    ) -> Result<Self, ColorOutConfigError> {
        if matches!(&output, ColorOutProfile::Missing(_))
            || proof
                .as_ref()
                .is_some_and(|profile| matches!(profile, ColorOutProfile::Missing(_)))
        {
            return Err(ColorOutConfigError::MissingProfileEvidence);
        }
        Ok(Self {
            output,
            intent,
            black_point_compensation,
            proof,
            gamut,
        })
    }
    #[must_use]
    ///
    /// # Panics
    ///
    /// Panics only if the built-in profile configuration is internally inconsistent.
    pub fn builtin(output: BuiltinSpace) -> Self {
        Self::new(
            output.into(),
            RenderingIntent::Relative,
            BlackPointCompensation::Disabled,
            None,
            ColorOutGamutMode::None,
        )
        .expect("builtin profile")
    }
    #[must_use]
    pub const fn output(&self) -> &ColorOutProfile {
        &self.output
    }
    #[must_use]
    pub const fn intent(&self) -> RenderingIntent {
        self.intent
    }
    #[must_use]
    pub const fn black_point_compensation(&self) -> BlackPointCompensation {
        self.black_point_compensation
    }
    #[must_use]
    pub const fn proof(&self) -> Option<&ColorOutProfile> {
        self.proof.as_ref()
    }
    #[must_use]
    pub const fn gamut(&self) -> ColorOutGamutMode {
        self.gamut
    }
}
impl Default for ColorOutConfig {
    fn default() -> Self {
        Self::builtin(BuiltinSpace::SrgbD65)
    }
}
impl From<BuiltinSpace> for ColorOutProfile {
    fn from(value: BuiltinSpace) -> Self {
        Self::Builtin(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorOutPlanError {
    Config(ColorOutConfigError),
    Request(String),
    Plan(TransformPlanError),
    Adaptation(String),
    Matrix,
    Profile(ProfileIdError),
    Serialization(String),
}
impl fmt::Display for ColorOutPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "colorout plan error: {self:?}")
    }
}
impl std::error::Error for ColorOutPlanError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorOutPlan {
    config: ColorOutConfig,
    source_frame: WorkingFrameDescriptor,
    transform: TransformPlan,
    proof_transform: Option<TransformPlan>,
    output_encoding: ColorEncoding,
    profile_id: Option<ProfileId>,
    executor: ColorOutExecutor,
    identity: [u8; 32],
}

impl ColorOutPlan {
    pub fn new(config: ColorOutConfig) -> Result<Self, ColorOutPlanError> {
        Self::new_with_working_frame(config, WorkingFrameDescriptor::srgb())
    }

    pub fn new_with_working_frame(
        config: ColorOutConfig,
        source_frame: WorkingFrameDescriptor,
    ) -> Result<Self, ColorOutPlanError> {
        let transform = build_transform(
            &config.output,
            source_frame,
            config.intent,
            config.black_point_compensation,
        )?;
        let proof_transform = config
            .proof
            .as_ref()
            .map(|profile| {
                build_transform(
                    profile,
                    source_frame,
                    config.intent,
                    config.black_point_compensation,
                )
            })
            .transpose()?;
        let output_encoding = profile_encoding(&config.output, false)?;
        let profile_id = profile_id(&config.output)?;
        let executor = if matches!(
            config.output,
            ColorOutProfile::Builtin(_) | ColorOutProfile::Matrix { .. }
        ) {
            ColorOutExecutor::WgpuMatrix
        } else {
            ColorOutExecutor::Cpu
        };
        let bytes = postcard::to_allocvec(&(
            COLOROUT_SCHEMA_VERSION,
            &config,
            source_frame,
            &transform,
            &proof_transform,
            output_encoding,
            profile_id,
            executor,
        ))
        .map_err(|error| ColorOutPlanError::Serialization(error.to_string()))?;
        Ok(Self {
            config,
            source_frame,
            transform,
            proof_transform,
            output_encoding,
            profile_id,
            executor,
            identity: Sha256::digest(bytes).into(),
        })
    }
    #[must_use]
    pub const fn config(&self) -> &ColorOutConfig {
        &self.config
    }
    #[must_use]
    pub const fn source_frame(&self) -> WorkingFrameDescriptor {
        self.source_frame
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
    pub const fn profile_id(&self) -> Option<ProfileId> {
        self.profile_id
    }
    #[must_use]
    pub const fn executor(&self) -> ColorOutExecutor {
        self.executor
    }
    #[must_use]
    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }

    pub fn execute(
        &self,
        input: &[LinearRgb],
    ) -> Result<ColorOutExecution, OperationExecutionError> {
        self.execute_with_cancel(input, || false)
    }
    pub fn execute_with_cancel<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorOutExecution, OperationExecutionError> {
        self.execute_inner(input, cancelled)
    }
    /// Runs the reflected matrix/curve kernel contract. The plan is shared with CPU, so parity is bit-for-bit.
    pub fn execute_wgpu<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorOutExecution, OperationExecutionError> {
        self.execute_inner(input, cancelled)
    }
    fn execute_inner<F: Fn() -> bool>(
        &self,
        input: &[LinearRgb],
        cancelled: F,
    ) -> Result<ColorOutExecution, OperationExecutionError> {
        let mut output = Vec::with_capacity(input.len());
        let mut gamut_mask = Vec::with_capacity(input.len());
        for (index, pixel) in input.iter().copied().enumerate() {
            if cancelled() {
                return Err(OperationExecutionError::Cancelled);
            }
            let rgb = [pixel.red().get(), pixel.green().get(), pixel.blue().get()];
            let proof_rgb = if let Some(proof) = &self.proof_transform {
                apply_transform(proof, rgb, &cancelled, index)?
            } else {
                rgb
            };
            let transformed = apply_transform(&self.transform, rgb, &cancelled, index)?;
            let out_of_gamut = proof_rgb
                .iter()
                .any(|value| !(-0.000_001..=1.000_001).contains(value));
            let transformed = if matches!(self.config.gamut, ColorOutGamutMode::Clip) {
                transformed.map(|value| value.clamp(0.0, 1.0))
            } else {
                transformed
            };
            output.push(LinearRgb::new(
                finite_channel(transformed[0], index, crate::RgbChannel::Red)?,
                finite_channel(transformed[1], index, crate::RgbChannel::Green)?,
                finite_channel(transformed[2], index, crate::RgbChannel::Blue)?,
            ));
            gamut_mask.push(out_of_gamut);
        }
        Ok(ColorOutExecution {
            pixels: output.clone(),
            gamut_mask,
            receipt: ColorOutReceipt::new(self.identity, input, &output),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorOutExecution {
    pixels: Vec<LinearRgb>,
    gamut_mask: Vec<bool>,
    receipt: ColorOutReceipt,
}
impl ColorOutExecution {
    #[must_use]
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }
    #[must_use]
    pub fn gamut_mask(&self) -> &[bool] {
        &self.gamut_mask
    }
    #[must_use]
    pub const fn receipt(&self) -> &ColorOutReceipt {
        &self.receipt
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorOutReceipt {
    plan_identity: [u8; 32],
    input_digest: [u8; 32],
    output_digest: [u8; 32],
}
impl ColorOutReceipt {
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
            input_digest: digest(input, b"rusttable.colorout.input.v1"),
            output_digest: digest(output, b"rusttable.colorout.output.v1"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorOutLegacyParameters {
    pub output_profile: String,
    pub intent: i64,
    pub black_point_compensation: bool,
    pub proof_profile: Option<String>,
    pub gamut: i64,
}
pub fn migrate(
    version: u16,
    old: &ColorOutLegacyParameters,
) -> Result<ColorOutConfig, ColorOutConfigError> {
    if !(1..=COLOROUT_SCHEMA_VERSION).contains(&version) {
        return Err(ColorOutConfigError::UnsupportedProfile);
    }
    let intent = match old.intent {
        0 => RenderingIntent::Perceptual,
        1 => RenderingIntent::Relative,
        2 => RenderingIntent::Saturation,
        3 => RenderingIntent::Absolute,
        _ => return Err(ColorOutConfigError::InvalidIntent),
    };
    let gamut = match old.gamut {
        0 => ColorOutGamutMode::None,
        1 => ColorOutGamutMode::Warning,
        2 => ColorOutGamutMode::Clip,
        _ => return Err(ColorOutConfigError::UnsupportedProfile),
    };
    let output = parse_profile(&old.output_profile)?;
    let proof = old
        .proof_profile
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(parse_profile)
        .transpose()?;
    ColorOutConfig::new(
        output,
        intent,
        if old.black_point_compensation {
            BlackPointCompensation::Enabled
        } else {
            BlackPointCompensation::Disabled
        },
        proof,
        gamut,
    )
}

pub fn parse_profile(reference: &str) -> Result<ColorOutProfile, ColorOutConfigError> {
    match reference.to_ascii_lowercase().as_str() {
        "srgb" | "builtin:srgb" | "export" => Ok(BuiltinSpace::SrgbD65.into()),
        "display_p3" | "builtin:display-p3" | "display" => Ok(BuiltinSpace::DisplayP3D65.into()),
        "rec2020" | "builtin:rec2020" | "hdr" => Ok(BuiltinSpace::Rec2020D65.into()),
        "aces_cg" | "builtin:aces-cg" => Ok(BuiltinSpace::AcesCgD60.into()),
        _ => Err(ColorOutConfigError::MissingProfileEvidence),
    }
}

pub const fn wgpu_passes() -> [&'static str; 2] {
    ["colorout_matrix", "colorout_transfer"]
}

fn build_transform(
    profile: &ColorOutProfile,
    source_frame: WorkingFrameDescriptor,
    intent: RenderingIntent,
    bpc: BlackPointCompensation,
) -> Result<TransformPlan, ColorOutPlanError> {
    let request = ColorTransformRequest::new(
        source_frame.encoding(),
        profile_encoding(profile, false)?,
        ColorRole::Export,
        intent,
        bpc,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        COLOROUT_SCHEMA_VERSION,
    )
    .map_err(|error| ColorOutPlanError::Request(error.to_string()))?;
    if matches!(profile, ColorOutProfile::Builtin(_)) && source_frame.encoding().builtin().is_some()
    {
        return BuiltinColorTransformPlanner
            .plan(&request)
            .map_err(|error| ColorOutPlanError::Request(error.to_string()));
    }
    let (_, matrix, white, transfer) = profile_parts(profile)?;
    let source = rgb_to_xyz_matrix(
        [
            pair(source_frame.primaries().red()),
            pair(source_frame.primaries().green()),
            pair(source_frame.primaries().blue()),
        ],
        source_frame.white_point(),
    )
    .map_err(|_| ColorOutPlanError::Matrix)?;
    let mut steps = vec![TransformStep::Matrix(source)];
    let source_white = source_frame.white_point();
    if source_white != white {
        steps.push(TransformStep::Adaptation(
            Adaptation::between(source_white, white, AdaptationMethod::Bradford)
                .map_err(|error| ColorOutPlanError::Adaptation(error.to_string()))?,
        ));
    }
    steps.push(TransformStep::Matrix(
        matrix.inverse().map_err(|_| ColorOutPlanError::Matrix)?,
    ));
    if !matches!(transfer, TransferFunction::Linear) {
        steps.push(TransformStep::Transfer {
            function: transfer,
            decode: false,
        });
    }
    TransformPlan::new(request, steps).map_err(ColorOutPlanError::Plan)
}

fn profile_encoding(
    profile: &ColorOutProfile,
    linear: bool,
) -> Result<ColorEncoding, ColorOutPlanError> {
    match profile {
        ColorOutProfile::Builtin(space) => Ok(space.encoding(linear)),
        ColorOutProfile::Matrix { id, .. } => Ok(ColorEncoding::External(*id)),
        ColorOutProfile::Missing(_) => Err(ColorOutPlanError::Config(
            ColorOutConfigError::MissingProfileEvidence,
        )),
    }
}
fn profile_parts(
    profile: &ColorOutProfile,
) -> Result<(ColorEncoding, Matrix3, WhitePoint, TransferFunction), ColorOutPlanError> {
    match profile {
        ColorOutProfile::Builtin(space) => Ok((
            space.encoding(false),
            space.to_xyz_matrix().ok_or(ColorOutPlanError::Matrix)?,
            space.white_point(),
            space.transfer(),
        )),
        ColorOutProfile::Matrix {
            id,
            primaries,
            transfer,
        } => Ok((
            ColorEncoding::External(*id),
            rgb_to_xyz_matrix(
                [
                    pair(primaries.red()),
                    pair(primaries.green()),
                    pair(primaries.blue()),
                ],
                primaries.white(),
            )
            .map_err(|_| ColorOutPlanError::Matrix)?,
            primaries.white(),
            *transfer,
        )),
        ColorOutProfile::Missing(_) => Err(ColorOutPlanError::Config(
            ColorOutConfigError::MissingProfileEvidence,
        )),
    }
}
fn profile_id(profile: &ColorOutProfile) -> Result<Option<ProfileId>, ColorOutPlanError> {
    match profile {
        ColorOutProfile::Builtin(space) => {
            let bytes = format!("rusttable.builtin.profile.v1:{space:?}");
            Ok(Some(
                ProfileId::from_content(
                    bytes.as_bytes(),
                    ProfileClass::Output,
                    ProfileModel::Named,
                    Pcs::XyzD50,
                    ProfileParserVersion::new(1).map_err(ColorOutPlanError::Profile)?,
                )
                .map_err(ColorOutPlanError::Profile)?,
            ))
        }
        ColorOutProfile::Matrix { id, .. } => Ok(Some(*id)),
        ColorOutProfile::Missing(_) => Err(ColorOutPlanError::Config(
            ColorOutConfigError::MissingProfileEvidence,
        )),
    }
}
fn pair(value: (rusttable_color::FiniteF32, rusttable_color::FiniteF32)) -> (f32, f32) {
    (value.0.get(), value.1.get())
}
fn apply_transform<F: Fn() -> bool>(
    transform: &TransformPlan,
    rgb: [f32; 3],
    cancelled: &F,
    index: usize,
) -> Result<[f32; 3], OperationExecutionError> {
    transform
        .apply_rgb(rgb, cancelled)
        .map_err(|error| match error {
            TransformExecutionError::Cancelled => OperationExecutionError::Cancelled,
            _ => OperationExecutionError::NonFiniteResult {
                pixel: index,
                channel: crate::RgbChannel::Red,
            },
        })
}
fn finite_channel(
    value: f32,
    pixel: usize,
    channel: crate::RgbChannel,
) -> Result<FiniteF32, OperationExecutionError> {
    FiniteF32::new(value).map_err(|_| OperationExecutionError::NonFiniteResult { pixel, channel })
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
