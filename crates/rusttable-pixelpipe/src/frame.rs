use rusttable_processing::{
    FrameBoundaryMode, FrameBoundaryOptions, OperationMaskSet, WorkingRgbImage,
    convert_working_to_linear_srgb, encode_working_to_srgb,
    evaluate_graph_at_frame_boundaries_with_masks, graph_has_frame_geometry,
};

use crate::{
    CancellationScope, CancellationStage, CpuPixelpipeError, CpuPixelpipeOutputMode,
    CpuPixelpipeSnapshot, RgbaF32ColorEncoding, RgbaF32Image, RgbaF32Pixel,
};

pub(crate) fn has_frame_geometry(request: &CpuPixelpipeSnapshot) -> bool {
    graph_has_frame_geometry(request.graph())
}

pub(crate) fn execute_frame_image(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    scope: Option<&CancellationScope>,
    masks: Option<&OperationMaskSet>,
) -> Result<(RgbaF32Image, [u8; 32], [u8; 32]), CpuPixelpipeError> {
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        return Err(CpuPixelpipeError::UnsupportedInputEncoding {
            actual: RgbaF32ColorEncoding::LabD50,
        });
    }
    let linear = crate::cpu::to_linear_working(input)?;
    let alpha = input
        .pixels()
        .iter()
        .map(|pixel| pixel.alpha())
        .collect::<Vec<_>>();
    let mode = match request.output_mode() {
        CpuPixelpipeOutputMode::Preview => FrameBoundaryMode::Preview,
        CpuPixelpipeOutputMode::FullExport => FrameBoundaryMode::Export,
    };
    let node_scope = scope.map(|scope| scope.child(CancellationStage::Node));
    let evaluated = evaluate_graph_at_frame_boundaries_with_masks(
        request.graph(),
        &linear,
        &alpha,
        FrameBoundaryOptions::new(mode)
            .with_source_orientation(input.descriptor().source_orientation()),
        masks,
        || {
            node_scope
                .as_ref()
                .is_some_and(|scope| scope.check().is_err())
        },
    )
    .map_err(|source| {
        node_scope
            .as_ref()
            .and_then(|scope| scope.check().err())
            .map_or(
                CpuPixelpipeError::Evaluation { source },
                CpuPixelpipeError::Cancelled,
            )
    })?;
    if let Some(scope) = &node_scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    let descriptor = crate::cpu::output_descriptor(
        request.output_mode(),
        input.descriptor(),
        evaluated.image().dimensions(),
    )
    .with_source_orientation(evaluated.output_source_orientation());
    let pixels = output_pixels(request.output_mode(), evaluated.image(), evaluated.alpha());
    let image = RgbaF32Image::new(descriptor, pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })?;
    Ok((
        image,
        evaluated.basicadj_plans().identity(),
        evaluated.frame_plan_identity(),
    ))
}

fn output_pixels(
    mode: CpuPixelpipeOutputMode,
    evaluated: &WorkingRgbImage,
    alpha: &[f32],
) -> Vec<RgbaF32Pixel> {
    match mode {
        CpuPixelpipeOutputMode::Preview => encode_working_to_srgb(evaluated)
            .image()
            .pixels()
            .zip(alpha)
            .map(|(rgb, alpha)| {
                RgbaF32Pixel::new(rgb.red().get(), rgb.green().get(), rgb.blue().get(), *alpha)
            })
            .collect(),
        CpuPixelpipeOutputMode::FullExport => convert_working_to_linear_srgb(evaluated)
            .pixels()
            .zip(alpha)
            .map(|(rgb, alpha)| {
                RgbaF32Pixel::new(rgb.red().get(), rgb.green().get(), rgb.blue().get(), *alpha)
            })
            .collect(),
    }
}
