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

pub(super) fn compile_finalscale(
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

pub(super) fn compile_enlargecanvas(
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
