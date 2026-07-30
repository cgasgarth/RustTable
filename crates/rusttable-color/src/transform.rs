use crate::conversions::{ColorMathError, lab_to_xyz, xyz_to_lab};
use crate::{
    AdaptationMethod, ColorEncoding, ColorRole, FiniteF32, FiniteF32Error, Matrix3,
    TransferFunction, TransferFunctionError, WhitePoint,
};
use serde::{Deserialize, Serialize};
use std::fmt;

const MAX_STEPS: usize = 64;
const MAX_COMPOSITE_STEPS: usize = 64;
const MAX_LUT_1D_SAMPLES: usize = 65_536;
const MAX_LUT_3D_EDGE: usize = 65;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Precision {
    F32,
    F64,
    U16,
    U32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RenderingIntent {
    Perceptual,
    Relative,
    Saturation,
    Absolute,
}

pub type Intent = RenderingIntent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum BlackPointCompensation {
    Disabled,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum AlphaTransform {
    Preserve,
    Premultiply,
    Unpremultiply,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adaptation {
    method: AdaptationMethod,
    source: WhitePoint,
    target: WhitePoint,
    matrix: Matrix3,
}

impl Adaptation {
    /// Builds an explicit chromatic adaptation matrix. Bradford is the product default.
    pub fn between(
        source: WhitePoint,
        target: WhitePoint,
        method: AdaptationMethod,
    ) -> Result<Self, MatrixErrorAdapter> {
        let matrix = match method {
            AdaptationMethod::Identity if source == target => Matrix3::identity(),
            AdaptationMethod::Identity => return Err(MatrixErrorAdapter::InvalidMatrix),
            AdaptationMethod::Bradford => bradford_matrix(source, target)?,
        };
        Ok(Self {
            method,
            source,
            target,
            matrix,
        })
    }

    #[must_use]
    pub const fn method(self) -> AdaptationMethod {
        self.method
    }

    #[must_use]
    pub const fn source(self) -> WhitePoint {
        self.source
    }

    #[must_use]
    pub const fn target(self) -> WhitePoint {
        self.target
    }

    #[must_use]
    pub const fn matrix(self) -> Matrix3 {
        self.matrix
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatrixErrorAdapter {
    InvalidMatrix,
}

/// Linear Bradford CAT as specified by ICC.1 and reproduced in CSS Color 4.
///
/// <https://www.w3.org/TR/css-color-4/#bradford>
fn bradford_matrix(source: WhitePoint, target: WhitePoint) -> Result<Matrix3, MatrixErrorAdapter> {
    if source == target {
        return Ok(Matrix3::identity());
    }
    let bradford = Matrix3::new([
        0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
    ])
    .map_err(|_| MatrixErrorAdapter::InvalidMatrix)?;
    let inverse = bradford
        .inverse()
        .map_err(|_| MatrixErrorAdapter::InvalidMatrix)?;
    let source_cone = bradford
        .apply_checked(source.xyz())
        .map_err(|_| MatrixErrorAdapter::InvalidMatrix)?;
    let target_cone = bradford
        .apply_checked(target.xyz())
        .map_err(|_| MatrixErrorAdapter::InvalidMatrix)?;
    if source_cone.into_iter().any(|value| value == 0.0) {
        return Err(MatrixErrorAdapter::InvalidMatrix);
    }
    let diagonal = Matrix3::new([
        target_cone[0] / source_cone[0],
        0.0,
        0.0,
        0.0,
        target_cone[1] / source_cone[1],
        0.0,
        0.0,
        0.0,
        target_cone[2] / source_cone[2],
    ])
    .map_err(|_| MatrixErrorAdapter::InvalidMatrix)?;
    inverse
        .multiply(diagonal)
        .and_then(|matrix| matrix.multiply(bradford))
        .map_err(|_| MatrixErrorAdapter::InvalidMatrix)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColorTransformRequest {
    schema_version: u16,
    source: ColorEncoding,
    target: ColorEncoding,
    role: ColorRole,
    intent: RenderingIntent,
    black_point_compensation: BlackPointCompensation,
    adaptation: AdaptationMethod,
    precision: Precision,
    alpha: AlphaTransform,
    extended_range: crate::ExtendedRange,
    planner_version: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorTransformRequestError {
    UnspecifiedEndpoint,
    InvalidPlannerVersion,
    SchemaVersionMismatch,
    Serialization(String),
    InvalidAdaptation,
}

impl ColorTransformRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source: ColorEncoding,
        target: ColorEncoding,
        role: ColorRole,
        intent: RenderingIntent,
        black_point_compensation: BlackPointCompensation,
        adaptation: AdaptationMethod,
        precision: Precision,
        alpha: AlphaTransform,
        extended_range: crate::ExtendedRange,
        planner_version: u16,
    ) -> Result<Self, ColorTransformRequestError> {
        if !source.is_explicit() || !target.is_explicit() {
            return Err(ColorTransformRequestError::UnspecifiedEndpoint);
        }
        if planner_version == 0 {
            return Err(ColorTransformRequestError::InvalidPlannerVersion);
        }
        Ok(Self {
            schema_version: crate::COLOR_SCHEMA_VERSION,
            source,
            target,
            role,
            intent,
            black_point_compensation,
            adaptation,
            precision,
            alpha,
            extended_range,
            planner_version,
        })
    }

    #[must_use]
    pub const fn source(&self) -> ColorEncoding {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> ColorEncoding {
        self.target
    }

    #[must_use]
    pub const fn role(&self) -> ColorRole {
        self.role
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
    pub const fn adaptation(&self) -> AdaptationMethod {
        self.adaptation
    }

    #[must_use]
    pub const fn precision(&self) -> Precision {
        self.precision
    }

    #[must_use]
    pub const fn alpha(&self) -> AlphaTransform {
        self.alpha
    }

    #[must_use]
    pub const fn extended_range(&self) -> crate::ExtendedRange {
        self.extended_range
    }

    #[must_use]
    pub const fn planner_version(&self) -> u16 {
        self.planner_version
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ColorTransformRequestError> {
        postcard::to_allocvec(self)
            .map_err(|error| ColorTransformRequestError::Serialization(error.to_string()))
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, ColorTransformRequestError> {
        let request: Self = postcard::from_bytes(bytes)
            .map_err(|error| ColorTransformRequestError::Serialization(error.to_string()))?;
        if request.schema_version != crate::COLOR_SCHEMA_VERSION {
            return Err(ColorTransformRequestError::SchemaVersionMismatch);
        }
        Self::new(
            request.source,
            request.target,
            request.role,
            request.intent,
            request.black_point_compensation,
            request.adaptation,
            request.precision,
            request.alpha,
            request.extended_range,
            request.planner_version,
        )
    }

    pub fn identity(&self) -> Result<[u8; 32], ColorTransformRequestError> {
        let bytes = self.canonical_bytes()?;
        Ok(sha256(&bytes))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LutInterpolation {
    Linear,
    Tetrahedral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LutPacking {
    RgbInterleaved,
    PlanarRgb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lut1D {
    samples: Vec<[FiniteF32; 3]>,
    interpolation: LutInterpolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lut1DError {
    TooFewSamples,
    TooManySamples,
    NonFinite,
}

impl Lut1D {
    pub fn new(
        samples: Vec<[f32; 3]>,
        interpolation: LutInterpolation,
    ) -> Result<Self, Lut1DError> {
        if samples.len() < 2 {
            return Err(Lut1DError::TooFewSamples);
        }
        if samples.len() > MAX_LUT_1D_SAMPLES {
            return Err(Lut1DError::TooManySamples);
        }
        let samples = samples
            .into_iter()
            .map(|sample| {
                sample
                    .map(|value| {
                        FiniteF32::new(value).map_err(|_: FiniteF32Error| Lut1DError::NonFinite)
                    })
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|values| values.try_into().map_err(|_| Lut1DError::NonFinite))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            samples,
            interpolation,
        })
    }

    #[must_use]
    pub fn samples(&self) -> &[[FiniteF32; 3]] {
        &self.samples
    }

    #[must_use]
    pub const fn interpolation(&self) -> LutInterpolation {
        self.interpolation
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lut3D {
    edge: u8,
    values: Vec<[FiniteF32; 3]>,
    packing: LutPacking,
    interpolation: LutInterpolation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lut3DError {
    EdgeTooSmall,
    EdgeTooLarge,
    SampleCountOverflow,
    SampleCountMismatch { expected: usize, actual: usize },
    NonFinite,
}

impl Lut3D {
    pub fn new(
        edge: u8,
        values: Vec<[f32; 3]>,
        packing: LutPacking,
        interpolation: LutInterpolation,
    ) -> Result<Self, Lut3DError> {
        let edge_usize = usize::from(edge);
        if edge_usize < 2 {
            return Err(Lut3DError::EdgeTooSmall);
        }
        if edge_usize > MAX_LUT_3D_EDGE {
            return Err(Lut3DError::EdgeTooLarge);
        }
        let expected = edge_usize
            .checked_mul(edge_usize)
            .and_then(|value| value.checked_mul(edge_usize))
            .ok_or(Lut3DError::SampleCountOverflow)?;
        if values.len() != expected {
            return Err(Lut3DError::SampleCountMismatch {
                expected,
                actual: values.len(),
            });
        }
        let values = values
            .into_iter()
            .map(|sample| {
                sample
                    .map(|value| {
                        FiniteF32::new(value).map_err(|_: FiniteF32Error| Lut3DError::NonFinite)
                    })
                    .into_iter()
                    .collect::<Result<Vec<_>, _>>()
                    .and_then(|values| values.try_into().map_err(|_| Lut3DError::NonFinite))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            edge,
            values,
            packing,
            interpolation,
        })
    }

    #[must_use]
    pub const fn edge(&self) -> u8 {
        self.edge
    }

    #[must_use]
    pub fn values(&self) -> &[[FiniteF32; 3]] {
        &self.values
    }

    #[must_use]
    pub const fn packing(&self) -> LutPacking {
        self.packing
    }

    #[must_use]
    pub const fn interpolation(&self) -> LutInterpolation {
        self.interpolation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransformStep {
    Identity,
    Transfer {
        function: TransferFunction,
        decode: bool,
    },
    Matrix(Matrix3),
    Adaptation(Adaptation),
    XyzToLab {
        white_point: WhitePoint,
    },
    LabToXyz {
        white_point: WhitePoint,
    },
    Lut1D(Lut1D),
    Lut3D(Lut3D),
    Composite(CompositeStep),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompositeStep {
    steps: Vec<TransformStep>,
}

impl CompositeStep {
    pub fn new(steps: Vec<TransformStep>) -> Result<Self, TransformStepError> {
        if steps.is_empty() || steps.len() > MAX_COMPOSITE_STEPS {
            return Err(TransformStepError::InvalidCompositeLength);
        }
        for step in &steps {
            step.validate()?;
        }
        Ok(Self { steps })
    }

    #[must_use]
    pub fn steps(&self) -> &[TransformStep] {
        &self.steps
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformStepError {
    InvalidCompositeLength,
    InvalidMatrix,
    InvalidAdaptation,
    InvalidLut,
}

impl TransformStep {
    fn validate(&self) -> Result<(), TransformStepError> {
        match self {
            Self::Identity | Self::XyzToLab { .. } | Self::LabToXyz { .. } => Ok(()),
            Self::Transfer { function, .. } => match function {
                TransferFunction::Gamma(gamma) if gamma.get() <= 0.0 => {
                    Err(TransformStepError::InvalidLut)
                }
                _ => Ok(()),
            },
            Self::Matrix(matrix) => matrix
                .inverse()
                .map(|_| ())
                .map_err(|_| TransformStepError::InvalidMatrix),
            Self::Adaptation(adaptation) => adaptation
                .matrix
                .inverse()
                .map(|_| ())
                .map_err(|_| TransformStepError::InvalidAdaptation),
            Self::Lut1D(lut) => (2..=MAX_LUT_1D_SAMPLES)
                .contains(&lut.samples.len())
                .then_some(())
                .ok_or(TransformStepError::InvalidLut),
            Self::Lut3D(lut) => {
                let edge = usize::from(lut.edge);
                let expected = edge.checked_mul(edge).and_then(|x| x.checked_mul(edge));
                (matches!(expected, Some(value) if value == lut.values.len())
                    && (2..=MAX_LUT_3D_EDGE).contains(&edge))
                .then_some(())
                .ok_or(TransformStepError::InvalidLut)
            }
            Self::Composite(composite) => {
                if composite.steps.is_empty() || composite.steps.len() > MAX_COMPOSITE_STEPS {
                    return Err(TransformStepError::InvalidCompositeLength);
                }
                composite.steps.iter().try_for_each(Self::validate)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformPlan {
    schema_version: u16,
    request: ColorTransformRequest,
    steps: Vec<TransformStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformExecutionError {
    Cancelled,
    NonFinite,
    Transfer(TransferFunctionError),
    ColorMath(ColorMathError),
    InvalidLut,
}

impl fmt::Display for TransformExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Cancelled => "color transform was cancelled",
            Self::NonFinite => "color transform produced a non-finite value",
            Self::Transfer(error) => return error.fmt(formatter),
            Self::ColorMath(error) => return error.fmt(formatter),
            Self::InvalidLut => "color transform LUT interpolation is invalid",
        })
    }
}

impl std::error::Error for TransformExecutionError {}

impl From<TransferFunctionError> for TransformExecutionError {
    fn from(error: TransferFunctionError) -> Self {
        Self::Transfer(error)
    }
}

impl From<ColorMathError> for TransformExecutionError {
    fn from(error: ColorMathError) -> Self {
        Self::ColorMath(error)
    }
}

/// A bounded durable projection containing no profile bytes or native handles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransformReceipt {
    schema_version: u16,
    request_hash: [u8; 32],
    plan_hash: [u8; 32],
    source: ColorEncoding,
    target: ColorEncoding,
    step_count: u16,
    resource_estimate: u64,
}

impl TransformReceipt {
    #[must_use]
    pub const fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }

    #[must_use]
    pub const fn plan_hash(&self) -> [u8; 32] {
        self.plan_hash
    }

    #[must_use]
    pub const fn source(&self) -> ColorEncoding {
        self.source
    }

    #[must_use]
    pub const fn target(&self) -> ColorEncoding {
        self.target
    }

    #[must_use]
    pub const fn step_count(&self) -> u16 {
        self.step_count
    }

    #[must_use]
    pub const fn resource_estimate(&self) -> u64 {
        self.resource_estimate
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransformPlanError {
    Request(ColorTransformRequestError),
    EmptyPlan,
    TooManySteps,
    InvalidStep(TransformStepError),
    SchemaVersionMismatch,
    Serialization(String),
}

impl TransformPlan {
    pub fn new(
        request: ColorTransformRequest,
        steps: Vec<TransformStep>,
    ) -> Result<Self, TransformPlanError> {
        if steps.len() > MAX_STEPS {
            return Err(TransformPlanError::TooManySteps);
        }
        if steps.is_empty() {
            return Err(TransformPlanError::EmptyPlan);
        }
        for step in &steps {
            step.validate().map_err(TransformPlanError::InvalidStep)?;
        }
        Ok(Self {
            schema_version: crate::COLOR_SCHEMA_VERSION,
            request,
            steps,
        })
    }

    #[must_use]
    pub const fn request(&self) -> &ColorTransformRequest {
        &self.request
    }

    #[must_use]
    pub fn steps(&self) -> &[TransformStep] {
        &self.steps
    }

    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.steps
            .iter()
            .all(|step| matches!(step, TransformStep::Identity))
    }

    #[must_use]
    pub fn resource_estimate(&self) -> u64 {
        self.steps.iter().map(step_resource_estimate).sum()
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, TransformPlanError> {
        postcard::to_allocvec(self)
            .map_err(|error| TransformPlanError::Serialization(error.to_string()))
    }

    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self, TransformPlanError> {
        let plan: Self = postcard::from_bytes(bytes)
            .map_err(|error| TransformPlanError::Serialization(error.to_string()))?;
        if plan.schema_version != crate::COLOR_SCHEMA_VERSION {
            return Err(TransformPlanError::SchemaVersionMismatch);
        }
        Self::new(plan.request, plan.steps)
    }

    pub fn identity(&self) -> Result<[u8; 32], TransformPlanError> {
        let bytes = self.canonical_bytes()?;
        Ok(sha256(&bytes))
    }

    pub fn receipt(&self) -> Result<TransformReceipt, TransformPlanError> {
        let request_hash = self
            .request
            .identity()
            .map_err(TransformPlanError::Request)?;
        let plan_hash = self.identity()?;
        Ok(TransformReceipt {
            schema_version: crate::COLOR_SCHEMA_VERSION,
            request_hash,
            plan_hash,
            source: self.request.source(),
            target: self.request.target(),
            step_count: u16::try_from(self.steps.len())
                .map_err(|_| TransformPlanError::TooManySteps)?,
            resource_estimate: self.resource_estimate(),
        })
    }

    /// Applies the checked plan to one RGB triplet without clipping extended
    /// or negative values. The callback is polled between plan steps.
    pub fn apply_rgb<F>(
        &self,
        rgb: [f32; 3],
        cancelled: F,
    ) -> Result<[f32; 3], TransformExecutionError>
    where
        F: Fn() -> bool,
    {
        let mut value = rgb;
        for step in &self.steps {
            if cancelled() {
                return Err(TransformExecutionError::Cancelled);
            }
            apply_step(step, &mut value)?;
        }
        if value.iter().all(|channel| channel.is_finite()) {
            Ok(value)
        } else {
            Err(TransformExecutionError::NonFinite)
        }
    }
}

fn apply_step(step: &TransformStep, value: &mut [f32; 3]) -> Result<(), TransformExecutionError> {
    match step {
        TransformStep::Identity => {}
        TransformStep::Transfer { function, decode } => {
            for channel in value.iter_mut() {
                *channel = if *decode {
                    function.decode(*channel)?
                } else {
                    function.encode(*channel)?
                };
            }
        }
        TransformStep::Matrix(matrix) | TransformStep::Adaptation(Adaptation { matrix, .. }) => {
            *value = matrix
                .apply_checked(*value)
                .map_err(|_| TransformExecutionError::NonFinite)?;
        }
        TransformStep::XyzToLab { white_point } => {
            *value = xyz_to_lab(*value, *white_point)?;
        }
        TransformStep::LabToXyz { white_point } => {
            *value = lab_to_xyz(*value, *white_point)?;
        }
        TransformStep::Lut1D(lut) => {
            for (channel_index, channel) in value.iter_mut().enumerate() {
                *channel = interpolate_lut_channel(lut.samples(), channel_index, *channel)?;
            }
        }
        TransformStep::Lut3D(lut) => {
            *value = interpolate_lut_3d(lut, *value)?;
        }
        TransformStep::Composite(composite) => {
            for child in composite.steps() {
                apply_step(child, value)?;
            }
        }
    }
    if value.iter().all(|channel| channel.is_finite()) {
        Ok(())
    } else {
        Err(TransformExecutionError::NonFinite)
    }
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn interpolate_lut_channel(
    samples: &[[FiniteF32; 3]],
    channel_index: usize,
    value: f32,
) -> Result<f32, TransformExecutionError> {
    if samples.len() < 2 || channel_index >= 3 || !value.is_finite() {
        return Err(TransformExecutionError::InvalidLut);
    }
    let position = value.clamp(0.0, 1.0) * (samples.len() - 1) as f32;
    let lower = (position.floor() as usize).min(samples.len() - 2);
    let fraction = position - lower as f32;
    let output = samples[lower][channel_index].get() * (1.0 - fraction)
        + samples[lower + 1][channel_index].get() * fraction;
    output
        .is_finite()
        .then_some(output)
        .ok_or(TransformExecutionError::NonFinite)
}

#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn interpolate_lut_3d(lut: &Lut3D, value: [f32; 3]) -> Result<[f32; 3], TransformExecutionError> {
    let edge = usize::from(lut.edge());
    if edge < 2 || lut.values().len() != edge * edge * edge {
        return Err(TransformExecutionError::InvalidLut);
    }
    let scaled = value.map(|channel| channel.clamp(0.0, 1.0) * (edge - 1) as f32);
    let base = scaled.map(|channel| (channel.floor() as usize).min(edge - 2));
    let fraction = scaled.map(|channel| channel - channel.floor());
    let mut output = [0.0; 3];
    for dz in 0..=1 {
        for dy in 0..=1 {
            for dx in 0..=1 {
                let x = base[0] + dx;
                let y = base[1] + dy;
                let z = base[2] + dz;
                let index = z * edge * edge + y * edge + x;
                let weight = if dx == 0 {
                    1.0 - fraction[0]
                } else {
                    fraction[0]
                } * if dy == 0 {
                    1.0 - fraction[1]
                } else {
                    fraction[1]
                } * if dz == 0 {
                    1.0 - fraction[2]
                } else {
                    fraction[2]
                };
                for (channel, output_channel) in output.iter_mut().enumerate() {
                    *output_channel += lut.values()[index][channel].get() * weight;
                }
            }
        }
    }
    output
        .iter()
        .all(|channel| channel.is_finite())
        .then_some(output)
        .ok_or(TransformExecutionError::NonFinite)
}

fn step_resource_estimate(step: &TransformStep) -> u64 {
    match step {
        TransformStep::Identity
        | TransformStep::Transfer { .. }
        | TransformStep::Matrix(_)
        | TransformStep::Adaptation(_)
        | TransformStep::XyzToLab { .. }
        | TransformStep::LabToXyz { .. } => 1,
        TransformStep::Lut1D(lut) => u64::try_from(lut.samples.len()).unwrap_or(u64::MAX),
        TransformStep::Lut3D(lut) => u64::try_from(lut.values.len()).unwrap_or(u64::MAX),
        TransformStep::Composite(composite) => {
            composite.steps.iter().map(step_resource_estimate).sum()
        }
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

use sha2::{Digest, Sha256};

impl fmt::Display for MatrixErrorAdapter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("chromatic adaptation matrix is invalid")
    }
}

impl std::error::Error for MatrixErrorAdapter {}

impl fmt::Display for ColorTransformRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnspecifiedEndpoint => "transform endpoints must be explicit",
            Self::InvalidPlannerVersion => "planner version must be nonzero",
            Self::SchemaVersionMismatch => "unsupported color request schema version",
            Self::Serialization(error) => return write!(formatter, "color request codec: {error}"),
            Self::InvalidAdaptation => "adaptation method is unsupported",
        })
    }
}

impl std::error::Error for ColorTransformRequestError {}

impl fmt::Display for Lut1DError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooFewSamples => "1D LUT needs at least two samples",
            Self::TooManySamples => "1D LUT exceeds 65536 samples",
            Self::NonFinite => "1D LUT contains a non-finite sample",
        })
    }
}

impl std::error::Error for Lut1DError {}

impl fmt::Display for Lut3DError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid 3D LUT: {self:?}")
    }
}

impl std::error::Error for Lut3DError {}

impl fmt::Display for TransformStepError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid transform step: {self:?}")
    }
}

impl std::error::Error for TransformStepError {}

impl fmt::Display for TransformPlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid transform plan: {self:?}")
    }
}

impl std::error::Error for TransformPlanError {}
