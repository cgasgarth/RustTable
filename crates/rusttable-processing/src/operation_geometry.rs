use rusttable_core::Operation;

use super::{
    OperationCompileError, ProcessingOperation, ProcessingOperationKind, compile_opacity,
    invalid_parameters, parameter_f32, parameter_integer, parameter_u32, reject_unexpected,
};
use crate::operations::{
    enlargecanvas::{CanvasColor, EnlargeCanvasConfig},
    finalscale::{
        FinalScaleConfig, FinalScaleKernel, RenderQuality, RenderQualityKind, RenderSizeRequest,
    },
    lenscorrection::{LensCorrectionConfig, LensCorrectionParametersV1},
    perspective::{PerspectiveConfig, PerspectiveParametersV5},
};

const FINALSCALE_PARAMETERS: [&str; 12] = [
    "mode",
    "width",
    "height",
    "edge",
    "megapixels",
    "width_mm",
    "height_mm",
    "dpi",
    "pipeline_scale",
    "allow_upscale",
    "kernel",
    "quality",
];
const ENLARGECANVAS_PARAMETERS: [&str; 5] = [
    "percent_left",
    "percent_right",
    "percent_top",
    "percent_bottom",
    "color",
];
const PERSPECTIVE_PARAMETERS: [&str; 14] = [
    "rotation",
    "lensshift_v",
    "lensshift_h",
    "shear",
    "focal_length",
    "crop_factor",
    "orthocorr",
    "aspect",
    "mode",
    "crop_mode",
    "crop_left",
    "crop_right",
    "crop_top",
    "crop_bottom",
];
const LENSCORRECTION_PARAMETERS: [&str; 8] = [
    "method",
    "modify_flags",
    "mode",
    "scale",
    "crop_factor",
    "focal_length",
    "aperture",
    "distance",
];

pub(crate) fn compile_finalscale(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &FINALSCALE_PARAMETERS)?;
    let mode = parameter_integer(operation, "mode", 0.0)?;
    let request = match mode {
        0 => RenderSizeRequest::Original,
        1 => RenderSizeRequest::exact(
            parameter_u32(operation, "width", 1)?,
            parameter_u32(operation, "height", 1)?,
        ),
        2 => RenderSizeRequest::fit_within(
            parameter_u32(operation, "width", 1)?,
            parameter_u32(operation, "height", 1)?,
        ),
        3 => RenderSizeRequest::long_edge(parameter_u32(operation, "edge", 1)?),
        4 => RenderSizeRequest::short_edge(parameter_u32(operation, "edge", 1)?),
        5 => RenderSizeRequest::megapixels(parameter_f32(operation, "megapixels", 1.0)?)
            .map_err(|error| invalid_parameters(operation, error))?,
        6 => RenderSizeRequest::print(
            parameter_f32(operation, "width_mm", 210.0)?,
            parameter_f32(operation, "height_mm", 297.0)?,
            parameter_f32(operation, "dpi", 300.0)?,
        )
        .map_err(|error| invalid_parameters(operation, error))?,
        7 => RenderSizeRequest::pipeline_scale(parameter_f32(operation, "pipeline_scale", 1.0)?)
            .map_err(|error| invalid_parameters(operation, error))?,
        _ => return Err(invalid_parameters(operation, "finalscale mode is invalid")),
    };
    let kernel = match parameter_integer(operation, "kernel", 1.0)? {
        0 => FinalScaleKernel::Nearest,
        1 => FinalScaleKernel::Bilinear,
        2 => FinalScaleKernel::Bicubic,
        3 => FinalScaleKernel::Lanczos,
        _ => {
            return Err(invalid_parameters(
                operation,
                "finalscale kernel is invalid",
            ));
        }
    };
    let quality_kind = match parameter_integer(operation, "quality", 2.0)? {
        0 => RenderQualityKind::Preview,
        1 => RenderQualityKind::Thumbnail,
        2 => RenderQualityKind::ImageFinal,
        3 => RenderQualityKind::Export,
        4 => RenderQualityKind::Print,
        _ => {
            return Err(invalid_parameters(
                operation,
                "finalscale quality is invalid",
            ));
        }
    };
    let config = FinalScaleConfig::new(request)
        .with_quality(RenderQuality::new(quality_kind, kernel))
        .with_upscale(parameter_integer(operation, "allow_upscale", 0.0)? != 0);
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::FinalScale { config },
    })
}

pub(crate) fn compile_enlargecanvas(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &ENLARGECANVAS_PARAMETERS)?;
    let color = parameter_u32(operation, "color", 0)?;
    let config = EnlargeCanvasConfig::new(
        parameter_f32(operation, "percent_left", 0.0)?,
        parameter_f32(operation, "percent_right", 0.0)?,
        parameter_f32(operation, "percent_top", 0.0)?,
        parameter_f32(operation, "percent_bottom", 0.0)?,
        CanvasColor::try_from(color).map_err(|error| invalid_parameters(operation, error))?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::EnlargeCanvas { config },
    })
}

pub(crate) fn compile_perspective(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &PERSPECTIVE_PARAMETERS)?;
    let parameters = PerspectiveParametersV5 {
        rotation: parameter_f32(operation, "rotation", 0.0)?,
        lensshift_v: parameter_f32(operation, "lensshift_v", 0.0)?,
        lensshift_h: parameter_f32(operation, "lensshift_h", 0.0)?,
        shear: parameter_f32(operation, "shear", 0.0)?,
        focal_length: parameter_f32(operation, "focal_length", 50.0)?,
        crop_factor: parameter_f32(operation, "crop_factor", 1.0)?,
        orthocorr: parameter_f32(operation, "orthocorr", 0.0)?,
        aspect: parameter_f32(operation, "aspect", 1.0)?,
        mode: parameter_integer(operation, "mode", 0.0)?,
        crop_mode: parameter_integer(operation, "crop_mode", 0.0)?,
        crop_left: parameter_f32(operation, "crop_left", 0.0)?,
        crop_right: parameter_f32(operation, "crop_right", 1.0)?,
        crop_top: parameter_f32(operation, "crop_top", 0.0)?,
        crop_bottom: parameter_f32(operation, "crop_bottom", 1.0)?,
        ..Default::default()
    };
    let config = PerspectiveConfig::from_parameters(parameters)
        .map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::Perspective { config },
    })
}

pub(crate) fn compile_lenscorrection(
    operation: &Operation,
) -> Result<ProcessingOperation, OperationCompileError> {
    reject_unexpected(operation, &LENSCORRECTION_PARAMETERS)?;
    let mut parameters = LensCorrectionParametersV1::new(
        "",
        "",
        parameter_f32(operation, "focal_length", 50.0)?,
        parameter_f32(operation, "aperture", 8.0)?,
    )
    .map_err(|error| invalid_parameters(operation, error))?;
    parameters.method = match parameter_integer(operation, "method", 0.0)? {
        0 => crate::operations::lenscorrection::LensCorrectionMethod::Lensfun,
        1 => crate::operations::lenscorrection::LensCorrectionMethod::OnlyVignetting,
        value => {
            return Err(invalid_parameters(
                operation,
                format!("lens correction method {value} is invalid"),
            ));
        }
    };
    parameters.modify_flags = match parameter_integer(operation, "modify_flags", 7.0)? {
        0 => crate::operations::lenscorrection::CorrectionFlags::empty(),
        1 => crate::operations::lenscorrection::CorrectionFlags::DISTORTION,
        2 => crate::operations::lenscorrection::CorrectionFlags::TCA,
        3 => crate::operations::lenscorrection::CorrectionFlags::ALL
            .without(crate::operations::lenscorrection::CorrectionFlags::VIGNETTING),
        4 => crate::operations::lenscorrection::CorrectionFlags::VIGNETTING,
        5 => crate::operations::lenscorrection::CorrectionFlags::ALL
            .without(crate::operations::lenscorrection::CorrectionFlags::TCA),
        6 => crate::operations::lenscorrection::CorrectionFlags::ALL
            .without(crate::operations::lenscorrection::CorrectionFlags::DISTORTION),
        7 => crate::operations::lenscorrection::CorrectionFlags::ALL,
        value => {
            return Err(invalid_parameters(
                operation,
                format!("lens correction flags {value} are invalid"),
            ));
        }
    };
    parameters.mode = match parameter_integer(operation, "mode", 0.0)? {
        0 => crate::operations::lenscorrection::LensCorrectionMode::Correct,
        1 => crate::operations::lenscorrection::LensCorrectionMode::Distort,
        value => {
            return Err(invalid_parameters(
                operation,
                format!("lens correction mode {value} is invalid"),
            ));
        }
    };
    parameters.scale = parameter_f32(operation, "scale", 1.0)?;
    parameters.crop_factor = parameter_f32(operation, "crop_factor", 1.0)?;
    parameters.distance = parameter_f32(operation, "distance", 1000.0)?;
    let config = LensCorrectionConfig::new(parameters)
        .map_err(|error| invalid_parameters(operation, error))?;
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        opacity: compile_opacity(operation)?,
        kind: ProcessingOperationKind::LensCorrection { config },
    })
}
