use crate::{
    Adaptation, AdaptationMethod, AlphaTransform, BuiltinSpace, ColorEncoding, ColorRole,
    ColorTransformRequest, ExtendedRange, MatrixErrorAdapter, Precision, RenderingIntent,
    TransformPlan, TransformPlanError, TransformStep,
};
use std::fmt;

pub trait ColorTransformPlanner {
    fn plan(&self, request: &ColorTransformRequest) -> Result<TransformPlan, PlannerError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct BuiltinColorTransformPlanner;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerError {
    UnknownProfile,
    UnsupportedColorSpace,
    Adaptation(MatrixErrorAdapter),
    Plan(TransformPlanError),
    Request(crate::ColorTransformRequestError),
    ContractVector,
}

impl ColorTransformPlanner for BuiltinColorTransformPlanner {
    fn plan(&self, request: &ColorTransformRequest) -> Result<TransformPlan, PlannerError> {
        if request.source() == request.target() {
            return TransformPlan::new(request.clone(), vec![TransformStep::Identity])
                .map_err(PlannerError::Plan);
        }
        let source_space = request
            .source()
            .builtin()
            .ok_or_else(|| match request.source() {
                ColorEncoding::External(_) => PlannerError::UnknownProfile,
                _ => PlannerError::UnsupportedColorSpace,
            })?;
        let target_space = request
            .target()
            .builtin()
            .ok_or_else(|| match request.target() {
                ColorEncoding::External(_) => PlannerError::UnknownProfile,
                _ => PlannerError::UnsupportedColorSpace,
            })?;
        if !source_space.is_matrix_space() || !target_space.is_matrix_space() {
            return Err(PlannerError::UnsupportedColorSpace);
        }

        let mut steps = Vec::with_capacity(5);
        if !request.source().is_linear() {
            steps.push(TransformStep::Transfer {
                function: request
                    .source()
                    .transfer()
                    .ok_or(PlannerError::UnsupportedColorSpace)?,
                decode: true,
            });
        }
        if source_space != BuiltinSpace::XyzD50 && source_space != BuiltinSpace::XyzD65 {
            steps.push(TransformStep::Matrix(
                source_space
                    .to_xyz_matrix()
                    .ok_or(PlannerError::UnsupportedColorSpace)?,
            ));
        }
        let source_white = source_space.white_point();
        let target_white = target_space.white_point();
        if source_white != target_white {
            steps.push(TransformStep::Adaptation(
                Adaptation::between(source_white, target_white, AdaptationMethod::Bradford)
                    .map_err(PlannerError::Adaptation)?,
            ));
        }
        if target_space != BuiltinSpace::XyzD50 && target_space != BuiltinSpace::XyzD65 {
            steps.push(TransformStep::Matrix(
                target_space
                    .to_xyz_matrix()
                    .ok_or(PlannerError::UnsupportedColorSpace)?
                    .inverse()
                    .map_err(|_| PlannerError::UnsupportedColorSpace)?,
            ));
        }
        if !request.target().is_linear() {
            steps.push(TransformStep::Transfer {
                function: request
                    .target()
                    .transfer()
                    .ok_or(PlannerError::UnsupportedColorSpace)?,
                decode: false,
            });
        }
        if steps.is_empty() {
            steps.push(TransformStep::Identity);
        }
        TransformPlan::new(request.clone(), steps).map_err(PlannerError::Plan)
    }
}

pub fn verify_builtin_contracts(
    verify_roundtrip: bool,
    verify_identities: bool,
) -> Result<crate::ContractReceipt, PlannerError> {
    let encodings = [
        ColorEncoding::SrgbD65,
        ColorEncoding::LinearSrgbD65,
        ColorEncoding::DisplayP3D65,
        ColorEncoding::LinearDisplayP3D65,
        ColorEncoding::Rec2020D65,
        ColorEncoding::LinearRec2020D65,
        ColorEncoding::AcesCgD60,
        ColorEncoding::XyzD50,
        ColorEncoding::XyzD65,
        ColorEncoding::LabD50,
        ColorEncoding::LchD50,
    ];
    let planner = BuiltinColorTransformPlanner;
    let mut identity_plans = 0;
    if verify_identities {
        for encoding in encodings {
            let request = request(encoding, encoding).map_err(PlannerError::Request)?;
            let plan = planner.plan(&request)?;
            if !plan.is_identity() {
                return Err(PlannerError::ContractVector);
            }
            identity_plans += 1;
        }
    }
    let mut roundtrips = 0;
    if verify_roundtrip {
        for encoding in [
            ColorEncoding::SrgbD65,
            ColorEncoding::DisplayP3D65,
            ColorEncoding::Rec2020D65,
        ] {
            let function = encoding.transfer().ok_or(PlannerError::ContractVector)?;
            for value in [0.0_f32, 0.001, 0.18, 1.0, 4.0, -0.25] {
                let decoded = function
                    .decode(value)
                    .map_err(|_| PlannerError::ContractVector)?;
                let encoded = function
                    .encode(decoded)
                    .map_err(|_| PlannerError::ContractVector)?;
                if (encoded - value).abs() > 0.000_01_f32.max(value.abs() * 0.000_01) {
                    return Err(PlannerError::ContractVector);
                }
                roundtrips += 1;
            }
        }
        let d50 = crate::WhitePoint::D50;
        let d65 = crate::WhitePoint::D65;
        let adaptation = Adaptation::between(d50, d65, AdaptationMethod::Bradford)
            .map_err(PlannerError::Adaptation)?;
        let restored = Adaptation::between(d65, d50, AdaptationMethod::Bradford)
            .map_err(PlannerError::Adaptation)?;
        let white = restored
            .matrix()
            .apply(adaptation.matrix().apply(d50.xyz()));
        if white
            .iter()
            .zip(d50.xyz())
            .any(|(actual, expected)| (actual - expected).abs() > 0.000_2)
        {
            return Err(PlannerError::ContractVector);
        }
        roundtrips += 1;
        for space in [
            BuiltinSpace::SrgbD65,
            BuiltinSpace::DisplayP3D65,
            BuiltinSpace::Rec2020D65,
            BuiltinSpace::AcesCgD60,
            BuiltinSpace::XyzD50,
            BuiltinSpace::XyzD65,
        ] {
            let matrix = space.to_xyz_matrix().ok_or(PlannerError::ContractVector)?;
            let inverse = matrix.inverse().map_err(|_| PlannerError::ContractVector)?;
            let vector = [0.18, 0.42, 1.7];
            let restored = inverse.apply(matrix.apply(vector));
            if restored
                .into_iter()
                .zip(vector)
                .any(|(actual, expected)| (actual - expected).abs() > 0.000_02)
            {
                return Err(PlannerError::ContractVector);
            }
            roundtrips += 1;
        }
    }
    Ok(crate::ContractReceipt {
        schema_version: crate::COLOR_SCHEMA_VERSION,
        builtins: encodings.len(),
        identity_plans,
        roundtrips,
    })
}

fn request(
    source: ColorEncoding,
    target: ColorEncoding,
) -> Result<ColorTransformRequest, crate::ColorTransformRequestError> {
    ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        crate::BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
}

impl fmt::Display for PlannerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "color planner error: {self:?}")
    }
}

impl std::error::Error for PlannerError {}
