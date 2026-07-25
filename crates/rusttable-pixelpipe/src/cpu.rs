#![allow(clippy::missing_errors_doc, clippy::match_same_arms)]

use rusttable_color::{
    AdaptationMethod, AlphaTransform, BlackPointCompensation, BuiltinColorTransformPlanner,
    BuiltinSpace, ColorRole, ColorTransformPlanner, ColorTransformRequest, ExtendedRange, Pcs,
    Precision, ProfileClass, ProfileId, ProfileModel, ProfileParserVersion, RenderingIntent,
    TransformPlan,
};
use rusttable_masks::MaskExecutionError;
use rusttable_processing::operations::colorin::{
    ColorInConfig, ColorInNormalization, ColorInPlan, ColorInProfile,
};
use rusttable_processing::{
    BasicAdjPlanSet, ColorContrastPixel, ColorContrastPlan, EvaluationError, FiniteF32, LinearRgb,
    OperationMaskSet, OperationMaskSetError, ProcessingOperation, RasterDimensions, SourceRgb,
    SourceRgbImage, SrgbChannel, WorkingRgbImage, convert_working_to_linear_srgb,
    encode_working_to_srgb, evaluate_graph_with_basicadj_plans_and_masks_with_cancellation,
    prepare_basicadj_plans, prepare_basicadj_plans_with_cancellation, to_linear_srgb,
};

use crate::frame::{execute_frame_image, has_frame_geometry};
use crate::{
    CancellationError, CancellationScope, CancellationStage, CpuNodeReceipt, CpuPipelineReceipt,
    CpuPixelpipeSnapshot, CpuTilePlan, CpuTilePlanError, PixelIdentity, RgbaF32Channel,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32ImageError, RgbaF32Pixel,
};

mod errors;
mod mask;
mod tile;

use mask::{crop_masks, resolve_masks};
use tile::{assemble_tile, checked_row_end, pixel_index, tile_pixel_count};

/// The typed presentation boundary requested from a CPU pixelpipe execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuPixelpipeOutputMode {
    /// Produce bounded transfer-encoded sRGB suitable for preview presentation.
    Preview,
    /// Retain linear sRGB for full-resolution file export.
    FullExport,
}

impl CpuPixelpipeOutputMode {
    pub(crate) const fn color_encoding(self) -> RgbaF32ColorEncoding {
        match self {
            Self::Preview => RgbaF32ColorEncoding::SrgbD65,
            Self::FullExport => RgbaF32ColorEncoding::LinearSrgbD65,
        }
    }
}

/// Immutable output from the registered scalar CPU executor.
#[derive(Debug, Clone, PartialEq)]
pub struct CpuPixelpipeResult {
    image: RgbaF32Image,
    receipt: CpuPipelineReceipt,
}

impl CpuPixelpipeResult {
    #[must_use]
    pub const fn image(&self) -> &RgbaF32Image {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> &CpuPipelineReceipt {
        &self.receipt
    }

    pub(crate) fn into_parts(self) -> (RgbaF32Image, CpuPipelineReceipt) {
        (self.image, self.receipt)
    }
}

/// Failure from the narrow scalar CPU pixelpipe executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuPixelpipeError {
    Cancelled(CancellationError),
    UnsupportedInputEncoding { actual: RgbaF32ColorEncoding },
    MissingSourceColor { actual: RgbaF32ColorEncoding },
    UnsupportedProfileTransform { profile: ProfileId },
    SourceColorPlan(String),
    InputBridge { source: RgbaF32ImageError },
    Evaluation { source: EvaluationError },
    OutputBoundary { source: RgbaF32ImageError },
    TilePlan { source: CpuTilePlanError },
    TileAssembly { source: CpuTileAssemblyError },
    MaskEvaluation { source: MaskExecutionError },
    MaskBinding { source: OperationMaskSetError },
}

/// Rejection reason while assembling scalar tile results into a full raster.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CpuTileAssemblyError {
    PixelIndexOverflow,
    PixelIndexExceedsPlatform { index: u64 },
    RowEndOverflow,
    SourceRowOutsideInput,
    DestinationRowOutsideOutput,
    TileUnavailable,
    TileOutputDimensionsMismatch,
}

/// The canonical scalar CPU executor for registered processing operations.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuPixelpipeExecutor;

impl CpuPixelpipeExecutor {
    /// Executes a prepared graph in authored order without interpreting operation names.
    ///
    /// The executor accepts normalized transfer-encoded sRGB, converts it once
    /// to linear sRGB, delegates registered nodes to `rusttable-processing`,
    /// then applies the requested typed output boundary. Straight alpha is
    /// preserved through each RGB-only boundary.
    ///
    /// # Errors
    ///
    /// Returns a typed failure before exposing a partial output image.
    pub fn execute(
        &self,
        request: &CpuPixelpipeSnapshot,
    ) -> Result<CpuPixelpipeResult, CpuPixelpipeError> {
        let masks = resolve_masks(request)?;
        if has_frame_geometry(request) {
            let (image, basicadj_identity, frame_plan_identity) =
                execute_frame_image(request, request.input(), None, masks.as_ref())?;
            return Ok(Self::result_for(
                request,
                image,
                basicadj_identity,
                frame_plan_identity,
            ));
        }
        let preserve_inert_lab = preserves_inert_lab_input(request, request.input());
        let plans = if preserve_inert_lab || is_colorcontrast_chain(request, request.input()) {
            BasicAdjPlanSet::default()
        } else {
            Self::prepare_plans(request)?
        };
        let image = Self::execute_image(
            request,
            request.input(),
            preserve_inert_lab,
            &plans,
            masks.as_ref(),
        )?;
        Ok(Self::result_for(request, image, plans.identity(), [0; 32]))
    }

    /// Executes with a generation-owned cancellation scope. The scope is
    /// checked before allocation, after evaluation, and before the result is
    /// constructed, so no partial image can escape.
    pub fn execute_with_cancellation(
        &self,
        request: &CpuPixelpipeSnapshot,
        scope: &CancellationScope,
    ) -> Result<CpuPixelpipeResult, CpuPixelpipeError> {
        scope
            .child(CancellationStage::Allocation)
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        let masks = resolve_masks(request)?;
        if has_frame_geometry(request) {
            let (image, basicadj_identity, frame_plan_identity) =
                execute_frame_image(request, request.input(), Some(scope), masks.as_ref())?;
            scope
                .child(CancellationStage::Publication)
                .check()
                .map_err(CpuPixelpipeError::Cancelled)?;
            return Ok(Self::result_for(
                request,
                image,
                basicadj_identity,
                frame_plan_identity,
            ));
        }
        let preserve_inert_lab = preserves_inert_lab_input(request, request.input());
        let plans = if preserve_inert_lab || is_colorcontrast_chain(request, request.input()) {
            BasicAdjPlanSet::default()
        } else {
            Self::prepare_plans_with_cancellation(request, scope)?
        };
        let image = Self::execute_image_with_cancellation(
            request,
            request.input(),
            preserve_inert_lab,
            &plans,
            masks.as_ref(),
            Some(scope),
        )?;
        scope
            .child(CancellationStage::Publication)
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        Ok(Self::result_for(request, image, plans.identity(), [0; 32]))
    }

    /// Executes a point-operation graph in deterministic, row-major tiles.
    ///
    /// # Errors
    ///
    /// Returns a typed error before exposing a partial image when the plan,
    /// source boundary, evaluation, or checked assembly fails.
    pub fn execute_tiled(
        &self,
        request: &CpuPixelpipeSnapshot,
        tile_plan: CpuTilePlan,
    ) -> Result<CpuPixelpipeResult, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        if requires_full_frame_execution(request) {
            // Both Darktable operations freeze full-image evidence before
            // replacement. Running them independently per tile changes their
            // result, so the legal tiled contract is a full-frame analysis
            // followed by one publication.
            return self.execute(request);
        }
        let preserve_inert_lab = preserves_inert_lab_input(request, request.input());
        let plans = if preserve_inert_lab || is_colorcontrast_chain(request, request.input()) {
            BasicAdjPlanSet::default()
        } else {
            Self::prepare_plans(request)?
        };
        let masks = resolve_masks(request)?;
        let grid = tile_plan
            .grid_for(request.input().descriptor().dimensions())
            .map_err(|source| CpuPixelpipeError::TilePlan { source })?;
        let mut assembled = request.input().pixels().to_vec();

        for tile_index in 0..grid.tile_count() {
            let tile = grid
                .tile_at(tile_index)
                .map_err(|source| CpuPixelpipeError::TilePlan { source })?
                .ok_or(CpuPixelpipeError::TileAssembly {
                    source: CpuTileAssemblyError::TileUnavailable,
                })?;
            let tile_input = tile_input(request.input(), tile)?;
            let tile_masks = masks
                .as_ref()
                .map(|set| crop_masks(set, tile))
                .transpose()?;
            let tile_output = Self::execute_image(
                request,
                &tile_input,
                preserve_inert_lab,
                &plans,
                tile_masks.as_ref(),
            )?;
            assemble_tile(
                &mut assembled,
                request.input().descriptor(),
                tile,
                &tile_output,
            )?;
        }

        let output_descriptor = output_descriptor(
            request.output_mode(),
            request.input().descriptor(),
            request.input().descriptor().dimensions(),
        );
        let image = RgbaF32Image::new(output_descriptor, assembled)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })?;
        Ok(Self::result_for(request, image, plans.identity(), [0; 32]))
    }

    /// Executes row-major tiles with a mandatory check before every tile and
    /// before final assembly/publication.
    pub fn execute_tiled_with_cancellation(
        &self,
        request: &CpuPixelpipeSnapshot,
        tile_plan: CpuTilePlan,
        scope: &CancellationScope,
    ) -> Result<CpuPixelpipeResult, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        scope
            .child(CancellationStage::Allocation)
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        if requires_full_frame_execution(request) {
            scope
                .child(CancellationStage::Tile)
                .check()
                .map_err(CpuPixelpipeError::Cancelled)?;
            let result = self.execute_with_cancellation(request, scope)?;
            scope
                .child(CancellationStage::Publication)
                .check()
                .map_err(CpuPixelpipeError::Cancelled)?;
            return Ok(result);
        }
        let preserve_inert_lab = preserves_inert_lab_input(request, request.input());
        let plans = if preserve_inert_lab || is_colorcontrast_chain(request, request.input()) {
            BasicAdjPlanSet::default()
        } else {
            Self::prepare_plans_with_cancellation(request, scope)?
        };
        let masks = resolve_masks(request)?;
        let grid = tile_plan
            .grid_for(request.input().descriptor().dimensions())
            .map_err(|source| CpuPixelpipeError::TilePlan { source })?;
        let mut assembled = request.input().pixels().to_vec();

        for tile_index in 0..grid.tile_count() {
            scope
                .child(CancellationStage::Tile)
                .check()
                .map_err(CpuPixelpipeError::Cancelled)?;
            let tile = grid
                .tile_at(tile_index)
                .map_err(|source| CpuPixelpipeError::TilePlan { source })?
                .ok_or(CpuPixelpipeError::TileAssembly {
                    source: CpuTileAssemblyError::TileUnavailable,
                })?;
            let tile_input = tile_input(request.input(), tile)?;
            let tile_masks = masks
                .as_ref()
                .map(|set| crop_masks(set, tile))
                .transpose()?;
            let tile_output = Self::execute_image_with_cancellation(
                request,
                &tile_input,
                preserve_inert_lab,
                &plans,
                tile_masks.as_ref(),
                Some(scope),
            )?;
            assemble_tile(
                &mut assembled,
                request.input().descriptor(),
                tile,
                &tile_output,
            )?;
        }
        scope
            .child(CancellationStage::Publication)
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        let output_descriptor = output_descriptor(
            request.output_mode(),
            request.input().descriptor(),
            request.input().descriptor().dimensions(),
        );
        let image = RgbaF32Image::new(output_descriptor, assembled)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })?;
        Ok(Self::result_for(request, image, plans.identity(), [0; 32]))
    }

    fn execute_image(
        request: &CpuPixelpipeSnapshot,
        input: &RgbaF32Image,
        preserve_inert_lab: bool,
        plans: &BasicAdjPlanSet,
        masks: Option<&OperationMaskSet>,
    ) -> Result<RgbaF32Image, CpuPixelpipeError> {
        Self::execute_image_with_cancellation(
            request,
            input,
            preserve_inert_lab,
            plans,
            masks,
            None,
        )
    }

    fn execute_image_with_cancellation(
        request: &CpuPixelpipeSnapshot,
        input: &RgbaF32Image,
        preserve_inert_lab: bool,
        plans: &BasicAdjPlanSet,
        masks: Option<&OperationMaskSet>,
        scope: Option<&CancellationScope>,
    ) -> Result<RgbaF32Image, CpuPixelpipeError> {
        validate_input_encoding(input)?;
        let node_scope = scope.map(|scope| scope.child(CancellationStage::Node));
        if let Some(scope) = &node_scope {
            scope.check().map_err(CpuPixelpipeError::Cancelled)?;
        }

        if preserve_inert_lab {
            return Ok(input.clone());
        }

        if is_colorcontrast_chain(request, input) {
            return execute_colorcontrast_chain(request, input, masks, node_scope.as_ref());
        }

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Censorize { .. }
            )
        }) && request.graph().nodes().count() == 1
            && masks.is_none()
        {
            return execute_censorize_image(request, input, node, node_scope.as_ref());
        }

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Clahe { .. }
            )
        }) && request.graph().nodes().count() == 1
            && masks.is_none()
        {
            return execute_clahe_image(request, input, node, node_scope.as_ref());
        }

        let linear_input = to_linear_working(input)?;
        let evaluated = evaluate_graph_with_basicadj_plans_and_masks_with_cancellation(
            request.graph(),
            &linear_input,
            Some(plans),
            masks,
            || {
                node_scope
                    .as_ref()
                    .is_some_and(|scope| scope.check().is_err())
            },
        )
        .map_err(|source| cancellable_evaluation_error(source, node_scope.as_ref()))?;
        if let Some(scope) = &node_scope {
            scope.check().map_err(CpuPixelpipeError::Cancelled)?;
        }
        output_from_working(request.output_mode(), input, &evaluated)
    }

    fn prepare_plans(request: &CpuPixelpipeSnapshot) -> Result<BasicAdjPlanSet, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        let linear = to_linear_working(request.input())?;
        prepare_basicadj_plans(request.graph(), &linear)
            .map_err(|source| CpuPixelpipeError::Evaluation { source })
    }

    fn prepare_plans_with_cancellation(
        request: &CpuPixelpipeSnapshot,
        scope: &CancellationScope,
    ) -> Result<BasicAdjPlanSet, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        let analysis_scope = scope.child(CancellationStage::Analysis);
        analysis_scope
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        let linear = to_linear_working(request.input())?;
        let plans = prepare_basicadj_plans_with_cancellation(request.graph(), &linear, || {
            analysis_scope.check().is_err()
        })
        .map_err(|source| cancellable_evaluation_error(source, Some(&analysis_scope)))?;
        analysis_scope
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        Ok(plans)
    }

    fn result_for(
        request: &CpuPixelpipeSnapshot,
        image: RgbaF32Image,
        basicadj_plan_identity: [u8; 32],
        frame_plan_identity: [u8; 32],
    ) -> CpuPixelpipeResult {
        let receipt = CpuPipelineReceipt::new(
            request.input().descriptor(),
            image.descriptor(),
            request.source_identity(),
            (pixel_identity(request.input()), pixel_identity(&image)),
            request.identity(),
            basicadj_plan_identity,
            frame_plan_identity,
            request.output_mode(),
            working_profile(request),
            request
                .graph()
                .nodes()
                .map(|node| {
                    CpuNodeReceipt::new(node.index().get(), node.operation().operation_id())
                })
                .collect(),
        );
        CpuPixelpipeResult { image, receipt }
    }
}

fn working_profile(request: &CpuPixelpipeSnapshot) -> rusttable_processing::WorkingFrameDescriptor {
    request
        .graph()
        .nodes()
        .filter_map(|node| match node.operation().kind() {
            rusttable_processing::ProcessingOperationKind::ColorIn { config } => {
                ColorInPlan::new(config.clone())
                    .ok()
                    .map(|plan| plan.output_frame())
            }
            _ => None,
        })
        .fold(None, |_, value| Some(value))
        .unwrap_or_else(rusttable_processing::WorkingFrameDescriptor::srgb)
}

pub(crate) fn operation_is_semantically_active(operation: &ProcessingOperation) -> bool {
    operation.is_enabled() && operation.opacity().get().to_bits() != 0.0_f32.to_bits()
}

fn preserves_inert_lab_input(request: &CpuPixelpipeSnapshot, input: &RgbaF32Image) -> bool {
    let input_encoding = input.descriptor().color_encoding();
    input_encoding == RgbaF32ColorEncoding::LabD50
        && output_encoding(request.output_mode(), input.descriptor()) == input_encoding
        && request
            .graph()
            .nodes()
            .all(|node| !operation_is_semantically_active(node.operation()))
}

fn is_colorcontrast_chain(request: &CpuPixelpipeSnapshot, input: &RgbaF32Image) -> bool {
    let input_supported = match input.descriptor().color_encoding() {
        RgbaF32ColorEncoding::SrgbD65
        | RgbaF32ColorEncoding::LinearSrgbD65
        | RgbaF32ColorEncoding::DisplayP3D65
        | RgbaF32ColorEncoding::LinearDisplayP3D65
        | RgbaF32ColorEncoding::LabD50 => true,
        RgbaF32ColorEncoding::External(_) => input
            .descriptor()
            .source_color()
            .is_some_and(|source_color| source_color.matrix().is_some()),
        _ => false,
    };
    if !input_supported {
        return false;
    }
    let mut has_active_colorcontrast = false;
    for node in request.graph().nodes() {
        let operation = node.operation();
        if !operation_is_semantically_active(operation) {
            continue;
        }
        if !matches!(
            operation.kind(),
            rusttable_processing::ProcessingOperationKind::ColorContrast { .. }
        ) {
            return false;
        }
        has_active_colorcontrast = true;
    }
    has_active_colorcontrast
}

fn execute_colorcontrast_chain(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    masks: Option<&OperationMaskSet>,
    scope: Option<&CancellationScope>,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let rgb_boundary;
    let mut output = if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        rgb_boundary = None;
        input
            .pixels()
            .iter()
            .map(|pixel| {
                ColorContrastPixel::new(pixel.red(), pixel.green(), pixel.blue(), pixel.alpha())
            })
            .collect::<Vec<_>>()
    } else {
        let working = to_linear_working(input)?;
        let to_lab = color_transform(
            working.frame().encoding(),
            rusttable_color::ColorEncoding::LabD50,
        )?;
        let from_lab = color_transform(
            rusttable_color::ColorEncoding::LabD50,
            working.frame().encoding(),
        )?;
        let lab = working
            .pixels()
            .zip(input.pixels())
            .enumerate()
            .map(|(pixel_index, (rgb, source))| {
                let channels = to_lab
                    .apply_rgb(
                        [rgb.red().get(), rgb.green().get(), rgb.blue().get()],
                        || scope.is_some_and(|scope| scope.check().is_err()),
                    )
                    .map_err(|error| {
                        colorcontrast_transform_error(
                            "RGB-to-Lab ingress",
                            pixel_index,
                            error,
                            scope,
                        )
                    })?;
                Ok(ColorContrastPixel::new(
                    channels[0],
                    channels[1],
                    channels[2],
                    source.alpha(),
                ))
            })
            .collect::<Result<Vec<_>, CpuPixelpipeError>>()?;
        rgb_boundary = Some((from_lab, working.frame()));
        lab
    };
    for node in request.graph().nodes() {
        if let Some(scope) = scope {
            scope.check().map_err(CpuPixelpipeError::Cancelled)?;
        }
        let operation = node.operation();
        let opacity = operation.opacity().get();
        if !operation_is_semantically_active(operation) {
            continue;
        }
        let config = match operation.kind() {
            rusttable_processing::ProcessingOperationKind::ColorContrast { config } => *config,
            _ => unreachable!("active nodes in a Lab Color Contrast chain are Color Contrast"),
        };
        let mask = masks.and_then(|set| set.mask_for(operation.operation_id()));
        let dimensions = input.descriptor().dimensions();
        if mask.is_some_and(|raster| {
            raster.width() != dimensions.width()
                || raster.height() != dimensions.height()
                || raster.values().len() != input.pixels().len()
        }) {
            return Err(CpuPixelpipeError::Evaluation {
                source: EvaluationError::OperationExecution {
                    step_index: node.pipeline_step_index(),
                    operation_id: operation.operation_id(),
                    reason: "Color Contrast mask sample count does not match the Lab raster"
                        .to_owned(),
                },
            });
        }
        let mask = mask.map(rusttable_masks::MaskRaster::values);
        let plan = ColorContrastPlan::new(config);
        output = if mask.is_none() && opacity.to_bits() == 1.0_f32.to_bits() {
            plan.execute_lab(&output)
        } else {
            plan.execute_lab_normal_blend(&output, mask, opacity)
        };
    }
    if let Some(scope) = scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    if let Some((from_lab, frame)) = rgb_boundary {
        let pixels = output
            .iter()
            .enumerate()
            .map(|(pixel_index, lab)| {
                let channels = lab.channels();
                let rgb = from_lab
                    .apply_rgb([channels[0], channels[1], channels[2]], || {
                        scope.is_some_and(|scope| scope.check().is_err())
                    })
                    .map_err(|error| {
                        colorcontrast_transform_error(
                            "Lab-to-RGB egress",
                            pixel_index,
                            error,
                            scope,
                        )
                    })?;
                Ok(LinearRgb::new(
                    FiniteF32::new(rgb[0]).map_err(|_| {
                        CpuPixelpipeError::SourceColorPlan(format!(
                            "Color Contrast Lab-to-RGB egress produced non-finite red at pixel {pixel_index}"
                        ))
                    })?,
                    FiniteF32::new(rgb[1]).map_err(|_| {
                        CpuPixelpipeError::SourceColorPlan(format!(
                            "Color Contrast Lab-to-RGB egress produced non-finite green at pixel {pixel_index}"
                        ))
                    })?,
                    FiniteF32::new(rgb[2]).map_err(|_| {
                        CpuPixelpipeError::SourceColorPlan(format!(
                            "Color Contrast Lab-to-RGB egress produced non-finite blue at pixel {pixel_index}"
                        ))
                    })?,
                ))
            })
            .collect::<Result<Vec<_>, CpuPixelpipeError>>()?;
        let evaluated =
            WorkingRgbImage::new_with_frame(input.descriptor().dimensions(), pixels, frame)
                .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
        return output_from_working(request.output_mode(), input, &evaluated);
    }
    let pixels = output
        .iter()
        .zip(input.pixels())
        .map(|(lab, source)| {
            let channels = lab.channels();
            RgbaF32Pixel::new(channels[0], channels[1], channels[2], source.alpha())
        })
        .collect();
    let descriptor = output_descriptor(
        request.output_mode(),
        input.descriptor(),
        input.descriptor().dimensions(),
    );
    RgbaF32Image::new(descriptor, pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

fn colorcontrast_transform_error(
    boundary: &str,
    pixel_index: usize,
    source: rusttable_color::TransformExecutionError,
    scope: Option<&CancellationScope>,
) -> CpuPixelpipeError {
    if matches!(source, rusttable_color::TransformExecutionError::Cancelled)
        && let Some(error) = scope.and_then(|scope| scope.check().err())
    {
        return CpuPixelpipeError::Cancelled(error);
    }
    CpuPixelpipeError::SourceColorPlan(format!(
        "Color Contrast {boundary} failed at pixel {pixel_index}: {source}"
    ))
}

fn execute_censorize_image(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    node: &rusttable_processing::OperationGraphNode,
    scope: Option<&CancellationScope>,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let linear = to_linear_working(input)?;
    let config = match node.operation().kind() {
        rusttable_processing::ProcessingOperationKind::Censorize { config } => *config,
        _ => unreachable!("censorize image bridge is only called for censorize"),
    };
    let rgba = linear
        .pixels()
        .zip(input.pixels())
        .map(|(rgb, source)| {
            rusttable_processing::CensorizePixel::new(
                rgb.red().get(),
                rgb.green().get(),
                rgb.blue().get(),
                source.alpha(),
            )
        })
        .collect::<Vec<_>>();
    let plan =
        rusttable_processing::CensorizePlan::new(config, input.descriptor().dimensions(), 1.0, 1.0)
            .map_err(|source| censorize_evaluation_error(node, &source, scope))?;
    let output = plan
        .execute_with_mask(&rgba, None, node.operation().opacity().get(), || {
            scope.is_some_and(|scope| scope.check().is_err())
        })
        .map_err(|source| censorize_evaluation_error(node, &source, scope))?;
    if let Some(scope) = scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    let rgb = output
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel_index, pixel)| {
            let channels = pixel.channels();
            Ok(rusttable_processing::LinearRgb::new(
                rusttable_processing::FiniteF32::new(channels[0])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Red))?,
                rusttable_processing::FiniteF32::new(channels[1])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Green))?,
                rusttable_processing::FiniteF32::new(channels[2])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Blue))?,
            ))
        })
        .collect::<Result<Vec<_>, CpuPixelpipeError>>()?;
    let working = rusttable_processing::WorkingRgbImage::new(input.descriptor().dimensions(), rgb)
        .map_err(|error| CpuPixelpipeError::Evaluation {
            source: EvaluationError::OperationExecution {
                step_index: node.pipeline_step_index(),
                operation_id: node.operation().operation_id(),
                reason: error.to_string(),
            },
        })?;
    let output_pixels = match request.output_mode() {
        CpuPixelpipeOutputMode::Preview => encode_working_to_srgb(&working)
            .image()
            .pixels()
            .zip(&output)
            .map(|(rgb, pixel)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    pixel.alpha(),
                )
            })
            .collect(),
        CpuPixelpipeOutputMode::FullExport => working
            .pixels()
            .zip(&output)
            .map(|(rgb, pixel)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    pixel.alpha(),
                )
            })
            .collect(),
    };
    let descriptor = output_descriptor(
        request.output_mode(),
        input.descriptor(),
        input.descriptor().dimensions(),
    );
    if let Some(scope) = scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    RgbaF32Image::new(descriptor, output_pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

fn censorize_evaluation_error(
    node: &rusttable_processing::OperationGraphNode,
    source: &rusttable_processing::CensorizeExecutionError,
    scope: Option<&CancellationScope>,
) -> CpuPixelpipeError {
    let source = match source {
        rusttable_processing::CensorizeExecutionError::Cancelled => EvaluationError::Cancelled {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
        },
        source => EvaluationError::OperationExecution {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
            reason: source.to_string(),
        },
    };
    cancellable_evaluation_error(source, scope)
}

fn execute_clahe_image(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    node: &rusttable_processing::OperationGraphNode,
    scope: Option<&CancellationScope>,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let linear = to_linear_working(input)?;
    let config = match node.operation().kind() {
        rusttable_processing::ProcessingOperationKind::Clahe { config } => *config,
        _ => unreachable!("clahe image bridge is only called for clahe"),
    };
    let pixels = linear
        .pixels()
        .zip(input.pixels())
        .map(|(rgb, source)| {
            rusttable_processing::ClahePixel::new(
                rgb.red().get(),
                rgb.green().get(),
                rgb.blue().get(),
                source.alpha(),
            )
        })
        .collect::<Vec<_>>();
    let plan =
        rusttable_processing::ClahePlan::new(config, input.descriptor().dimensions(), 1.0, 1.0)
            .map_err(|source| clahe_evaluation_error(node, &source, scope))?;
    let output = plan
        .execute_with_mask(&pixels, None, node.operation().opacity().get(), || {
            scope.is_some_and(|scope| scope.check().is_err())
        })
        .map_err(|source| clahe_evaluation_error(node, &source, scope))?;
    if let Some(scope) = scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    let rgb = output
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel_index, pixel)| {
            let channels = pixel.channels();
            Ok(rusttable_processing::LinearRgb::new(
                rusttable_processing::FiniteF32::new(channels[0])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Red))?,
                rusttable_processing::FiniteF32::new(channels[1])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Green))?,
                rusttable_processing::FiniteF32::new(channels[2])
                    .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Blue))?,
            ))
        })
        .collect::<Result<Vec<_>, CpuPixelpipeError>>()?;
    let working = rusttable_processing::WorkingRgbImage::new(input.descriptor().dimensions(), rgb)
        .map_err(|error| CpuPixelpipeError::Evaluation {
            source: EvaluationError::OperationExecution {
                step_index: node.pipeline_step_index(),
                operation_id: node.operation().operation_id(),
                reason: error.to_string(),
            },
        })?;
    let output_pixels = match request.output_mode() {
        CpuPixelpipeOutputMode::Preview => encode_working_to_srgb(&working)
            .image()
            .pixels()
            .zip(&output)
            .map(|(rgb, pixel)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    pixel.channels()[3],
                )
            })
            .collect(),
        CpuPixelpipeOutputMode::FullExport => working
            .pixels()
            .zip(&output)
            .map(|(rgb, pixel)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    pixel.channels()[3],
                )
            })
            .collect(),
    };
    let descriptor = output_descriptor(
        request.output_mode(),
        input.descriptor(),
        input.descriptor().dimensions(),
    );
    if let Some(scope) = scope {
        scope.check().map_err(CpuPixelpipeError::Cancelled)?;
    }
    RgbaF32Image::new(descriptor, output_pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

pub(crate) fn requires_full_frame_execution(request: &CpuPixelpipeSnapshot) -> bool {
    request.graph().nodes().any(|node| {
        node.operation().requires_full_image_analysis()
            || matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Highlights { .. }
                    | rusttable_processing::ProcessingOperationKind::ColorReconstruction { .. }
                    | rusttable_processing::ProcessingOperationKind::Bloom { .. }
                    | rusttable_processing::ProcessingOperationKind::Soften { .. }
                    | rusttable_processing::ProcessingOperationKind::Crop { .. }
                    | rusttable_processing::ProcessingOperationKind::Flip { .. }
                    | rusttable_processing::ProcessingOperationKind::RotatePixels { .. }
                    | rusttable_processing::ProcessingOperationKind::ScalePixels { .. }
                    | rusttable_processing::ProcessingOperationKind::FinalScale { .. }
                    | rusttable_processing::ProcessingOperationKind::EnlargeCanvas { .. }
                    | rusttable_processing::ProcessingOperationKind::Perspective { .. }
                    | rusttable_processing::ProcessingOperationKind::Clipping { .. }
                    | rusttable_processing::ProcessingOperationKind::LensCorrection { .. }
                    | rusttable_processing::ProcessingOperationKind::Grain { .. }
                    | rusttable_processing::ProcessingOperationKind::Censorize { .. }
                    | rusttable_processing::ProcessingOperationKind::Clahe { .. }
            )
    })
}

fn cancellable_evaluation_error(
    source: EvaluationError,
    scope: Option<&CancellationScope>,
) -> CpuPixelpipeError {
    if source.is_cancelled()
        && let Some(error) = scope.and_then(|scope| scope.check().err())
    {
        return CpuPixelpipeError::Cancelled(error);
    }
    CpuPixelpipeError::Evaluation { source }
}

fn clahe_evaluation_error(
    node: &rusttable_processing::OperationGraphNode,
    source: &rusttable_processing::ClaheExecutionError,
    scope: Option<&CancellationScope>,
) -> CpuPixelpipeError {
    let source = match source {
        rusttable_processing::ClaheExecutionError::Cancelled => EvaluationError::Cancelled {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
        },
        source => EvaluationError::OperationExecution {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
            reason: source.to_string(),
        },
    };
    cancellable_evaluation_error(source, scope)
}

pub(crate) fn validate_input_encoding(input: &RgbaF32Image) -> Result<(), CpuPixelpipeError> {
    let actual = input.descriptor().color_encoding();
    if matches!(
        actual,
        RgbaF32ColorEncoding::SrgbD65
            | RgbaF32ColorEncoding::LinearSrgbD65
            | RgbaF32ColorEncoding::DisplayP3D65
            | RgbaF32ColorEncoding::LinearDisplayP3D65
            | RgbaF32ColorEncoding::External(_)
            | RgbaF32ColorEncoding::LabD50
    ) {
        Ok(())
    } else {
        Err(CpuPixelpipeError::UnsupportedInputEncoding { actual })
    }
}

fn tile_input(
    input: &RgbaF32Image,
    tile: crate::CpuPixelpipeTile,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let mut pixels = Vec::with_capacity(tile_pixel_count(tile)?);
    for local_y in 0..tile.dimensions().height() {
        let source_y =
            tile.origin_y()
                .checked_add(local_y)
                .ok_or(CpuPixelpipeError::TileAssembly {
                    source: CpuTileAssemblyError::PixelIndexOverflow,
                })?;
        let source_start = pixel_index(input.descriptor(), tile.origin_x(), source_y)?;
        let source_end = checked_row_end(source_start, tile.dimensions().width())?;
        let source_row = input.pixels().get(source_start..source_end).ok_or(
            CpuPixelpipeError::TileAssembly {
                source: CpuTileAssemblyError::SourceRowOutsideInput,
            },
        )?;
        pixels.extend_from_slice(source_row);
    }
    RgbaF32Image::new(
        input.descriptor().with_dimensions_and_color_encoding(
            tile.dimensions(),
            input.descriptor().color_encoding(),
        ),
        pixels,
    )
    .map_err(|source| CpuPixelpipeError::InputBridge { source })
}

fn output_pixels(
    mode: CpuPixelpipeOutputMode,
    evaluated: &rusttable_processing::WorkingRgbImage,
    input: &RgbaF32Image,
) -> Result<Vec<RgbaF32Pixel>, CpuPixelpipeError> {
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        let to_lab = color_transform(
            evaluated.frame().encoding(),
            rusttable_color::ColorEncoding::LabD50,
        )?;
        return evaluated
            .pixels()
            .zip(input.pixels())
            .enumerate()
            .map(|(pixel_index, (rgb, source))| {
                let lab = to_lab
                    .apply_rgb(
                        [rgb.red().get(), rgb.green().get(), rgb.blue().get()],
                        || false,
                    )
                    .map_err(|error| {
                        CpuPixelpipeError::SourceColorPlan(format!(
                            "Lab output transform failed at pixel {pixel_index}: {error}"
                        ))
                    })?;
                Ok(RgbaF32Pixel::new(lab[0], lab[1], lab[2], source.alpha()))
            })
            .collect();
    }
    match mode {
        CpuPixelpipeOutputMode::Preview => Ok(encode_working_to_srgb(evaluated)
            .image()
            .pixels()
            .zip(input.pixels())
            .map(|(rgb, source)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    source.alpha(),
                )
            })
            .collect()),
        CpuPixelpipeOutputMode::FullExport => Ok(convert_working_to_linear_srgb(evaluated)
            .pixels()
            .zip(input.pixels())
            .map(|(rgb, source)| {
                RgbaF32Pixel::new(
                    rgb.red().get(),
                    rgb.green().get(),
                    rgb.blue().get(),
                    source.alpha(),
                )
            })
            .collect()),
    }
}

fn output_encoding(mode: CpuPixelpipeOutputMode, input: RgbaF32Descriptor) -> RgbaF32ColorEncoding {
    if input.color_encoding() == RgbaF32ColorEncoding::LabD50 {
        RgbaF32ColorEncoding::LabD50
    } else {
        mode.color_encoding()
    }
}

fn color_transform(
    source: rusttable_color::ColorEncoding,
    target: rusttable_color::ColorEncoding,
) -> Result<TransformPlan, CpuPixelpipeError> {
    let request = ColorTransformRequest::new(
        source,
        target,
        ColorRole::Working,
        RenderingIntent::Relative,
        BlackPointCompensation::Disabled,
        AdaptationMethod::Bradford,
        Precision::F32,
        AlphaTransform::Preserve,
        ExtendedRange::Extended,
        1,
    )
    .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
    BuiltinColorTransformPlanner
        .plan(&request)
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))
}

fn to_processing_source(input: &RgbaF32Image) -> Result<SourceRgbImage, CpuPixelpipeError> {
    let pixels = input
        .pixels()
        .iter()
        .copied()
        .enumerate()
        .map(|(pixel_index, pixel)| {
            let red = SrgbChannel::new(pixel.red())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Red))?;
            let green = SrgbChannel::new(pixel.green())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Green))?;
            let blue = SrgbChannel::new(pixel.blue())
                .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Blue))?;
            Ok(SourceRgb::new(red, green, blue))
        })
        .collect::<Result<Vec<_>, _>>()?;
    SourceRgbImage::new(input.descriptor().dimensions(), pixels).map_err(|_| {
        CpuPixelpipeError::InputBridge {
            source: RgbaF32ImageError::PixelCountMismatch {
                expected: input.descriptor().dimensions().pixel_count(),
                actual: input.pixels().len(),
            },
        }
    })
}

pub(crate) fn to_linear_working(
    input: &RgbaF32Image,
) -> Result<WorkingRgbImage, CpuPixelpipeError> {
    validate_input_encoding(input)?;
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        let to_rgb = color_transform(
            rusttable_color::ColorEncoding::LabD50,
            rusttable_color::ColorEncoding::LinearSrgbD65,
        )?;
        let pixels = input
            .pixels()
            .iter()
            .copied()
            .enumerate()
            .map(|(pixel_index, pixel)| {
                let rgb = to_rgb
                    .apply_rgb([pixel.red(), pixel.green(), pixel.blue()], || false)
                    .map_err(|error| {
                        CpuPixelpipeError::SourceColorPlan(format!(
                            "Lab input transform failed at pixel {pixel_index}: {error}"
                        ))
                    })?;
                Ok(LinearRgb::new(
                    FiniteF32::new(rgb[0])
                        .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Red))?,
                    FiniteF32::new(rgb[1])
                        .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Green))?,
                    FiniteF32::new(rgb[2])
                        .map_err(|_| input_component_error(pixel_index, RgbaF32Channel::Blue))?,
                ))
            })
            .collect::<Result<Vec<_>, CpuPixelpipeError>>()?;
        return WorkingRgbImage::new_with_frame(
            input.descriptor().dimensions(),
            pixels,
            rusttable_processing::WorkingFrameDescriptor::srgb(),
        )
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()));
    }
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LinearSrgbD65 {
        let pixels = input
            .pixels()
            .iter()
            .copied()
            .map(|pixel| {
                LinearRgb::new(
                    FiniteF32::new(pixel.red()).expect("validated finite red"),
                    FiniteF32::new(pixel.green()).expect("validated finite green"),
                    FiniteF32::new(pixel.blue()).expect("validated finite blue"),
                )
            })
            .collect();
        return WorkingRgbImage::new(input.descriptor().dimensions(), pixels).map_err(|_| {
            CpuPixelpipeError::InputBridge {
                source: RgbaF32ImageError::PixelCountMismatch {
                    expected: input.descriptor().dimensions().pixel_count(),
                    actual: input.pixels().len(),
                },
            }
        });
    }
    match input.descriptor().color_encoding() {
        RgbaF32ColorEncoding::DisplayP3D65 | RgbaF32ColorEncoding::LinearDisplayP3D65 => {
            let encoding = match input.descriptor().color_encoding() {
                RgbaF32ColorEncoding::DisplayP3D65 => rusttable_color::ColorEncoding::DisplayP3D65,
                RgbaF32ColorEncoding::LinearDisplayP3D65 => {
                    rusttable_color::ColorEncoding::LinearDisplayP3D65
                }
                _ => unreachable!("matched Display P3 ingress"),
            };
            let source_color = rusttable_image::SourceColor::declared(encoding)
                .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
            return to_colorin_working(input, source_color);
        }
        RgbaF32ColorEncoding::External(_) => {
            let source_color =
                input
                    .descriptor()
                    .source_color()
                    .ok_or(CpuPixelpipeError::MissingSourceColor {
                        actual: input.descriptor().color_encoding(),
                    })?;
            return to_colorin_working(input, source_color);
        }
        _ => {}
    }
    let source = to_processing_source(input)?;
    Ok(to_linear_srgb(&source))
}

pub(crate) fn output_from_working(
    mode: CpuPixelpipeOutputMode,
    input: &RgbaF32Image,
    evaluated: &WorkingRgbImage,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let output_descriptor =
        output_descriptor(mode, input.descriptor(), input.descriptor().dimensions());
    let pixels = output_pixels(mode, evaluated, input)?;
    RgbaF32Image::new(output_descriptor, pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

pub(crate) fn output_descriptor(
    mode: CpuPixelpipeOutputMode,
    source: RgbaF32Descriptor,
    dimensions: RasterDimensions,
) -> RgbaF32Descriptor {
    source.with_dimensions_and_color_encoding(dimensions, output_encoding(mode, source))
}

fn to_colorin_working(
    input: &RgbaF32Image,
    source_color: rusttable_image::SourceColor,
) -> Result<WorkingRgbImage, CpuPixelpipeError> {
    let Some((primaries, transfer)) = source_color.matrix() else {
        return Err(CpuPixelpipeError::UnsupportedProfileTransform {
            profile: source_color
                .profile()
                .expect("profile-authoritative source has an identity"),
        });
    };
    let id = source_color
        .profile()
        .map_or_else(|| synthetic_profile(source_color.encoding(), transfer), Ok)?;
    let input_profile = ColorInProfile::Matrix {
        id,
        primaries,
        transfer,
    };
    let config = ColorInConfig::new(
        input_profile,
        ColorInProfile::Builtin(BuiltinSpace::SrgbD65),
        RenderingIntent::Relative,
        ColorInNormalization::Off,
        false,
    )
    .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
    let plan = ColorInPlan::new(config)
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
    let pixels = input
        .pixels()
        .iter()
        .copied()
        .map(|pixel| {
            LinearRgb::new(
                FiniteF32::new(pixel.red()).expect("validated finite red"),
                FiniteF32::new(pixel.green()).expect("validated finite green"),
                FiniteF32::new(pixel.blue()).expect("validated finite blue"),
            )
        })
        .collect::<Vec<_>>();
    let execution = plan
        .execute(&pixels)
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
    WorkingRgbImage::new(input.descriptor().dimensions(), execution.pixels().to_vec())
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))
}

fn synthetic_profile(
    encoding: rusttable_color::ColorEncoding,
    transfer: rusttable_color::TransferFunction,
) -> Result<ProfileId, CpuPixelpipeError> {
    let bytes = postcard::to_allocvec(&(encoding, transfer))
        .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?;
    ProfileId::from_content(
        &bytes,
        ProfileClass::Input,
        ProfileModel::Matrix,
        Pcs::XyzD50,
        ProfileParserVersion::new(1)
            .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))?,
    )
    .map_err(|error| CpuPixelpipeError::SourceColorPlan(error.to_string()))
}

const fn input_component_error(pixel_index: usize, channel: RgbaF32Channel) -> CpuPixelpipeError {
    CpuPixelpipeError::InputBridge {
        source: RgbaF32ImageError::ComponentOutsideUnitInterval {
            pixel_index,
            channel,
        },
    }
}

fn pixel_identity(image: &RgbaF32Image) -> PixelIdentity {
    PixelIdentity::from_components(
        image
            .pixels()
            .iter()
            .flat_map(|pixel| [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]),
    )
}

#[cfg(test)]
mod tests {
    use rusttable_color::{Primaries, TransferFunction};
    use rusttable_core::{
        Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity,
        ParameterName, ParameterValue, PhotoId, Revision,
    };
    use rusttable_image::{SourceColor, SourceColorEvidence};
    use rusttable_processing::PipelineStepIndex;
    use rusttable_processing::operations::colorcontrast::ColorContrastConfig;

    use super::*;
    use crate::{CancellationReason, PipelineGeneration};

    #[test]
    fn inert_colorcontrast_nodes_do_not_add_a_lab_round_trip() {
        let input = linear_colorcontrast_input();
        let baseline = CpuPixelpipeExecutor
            .execute(&CpuPixelpipeSnapshot::new(
                input.clone(),
                operation_graph(Vec::new()),
                CpuPixelpipeOutputMode::FullExport,
            ))
            .expect("empty graph")
            .image()
            .clone();

        for operation in [
            colorcontrast_operation(
                0xcc01,
                false,
                OperationOpacity::ONE,
                [1.75, 12.0, 0.45, -9.0],
                1,
            ),
            colorcontrast_operation(
                0xcc02,
                true,
                OperationOpacity::ZERO,
                [0.4, -11.0, 1.8, 8.0],
                0,
            ),
        ] {
            let snapshot = CpuPixelpipeSnapshot::new(
                input.clone(),
                operation_graph(vec![operation]),
                CpuPixelpipeOutputMode::FullExport,
            );

            assert!(!is_colorcontrast_chain(&snapshot, snapshot.input()));
            assert_eq!(
                CpuPixelpipeExecutor
                    .execute(&snapshot)
                    .expect("inert Color Contrast graph")
                    .image(),
                &baseline
            );
        }
    }

    #[test]
    fn disabled_only_extreme_finite_lab_graph_preserves_exact_input_bits() {
        assert_inert_lab_identity(&colorcontrast_operation(
            0xcc03,
            false,
            OperationOpacity::ONE,
            [1.75, 12.0, 0.45, -9.0],
            1,
        ));
    }

    #[test]
    fn opacity_zero_only_extreme_finite_lab_graph_preserves_exact_input_bits() {
        assert_inert_lab_identity(&colorcontrast_operation(
            0xcc04,
            true,
            OperationOpacity::ZERO,
            [0.4, -11.0, 1.8, 8.0],
            0,
        ));
    }

    #[test]
    fn inert_non_colorcontrast_nodes_do_not_break_the_active_lab_chain() {
        let input = linear_colorcontrast_input();
        let first = colorcontrast_operation(
            0xcc11,
            true,
            OperationOpacity::ONE,
            [1.25, 3.0, 0.8, -2.0],
            1,
        );
        let second = colorcontrast_operation(
            0xcc12,
            true,
            OperationOpacity::ONE,
            [0.7, -4.0, 1.4, 5.0],
            0,
        );
        let routed = CpuPixelpipeSnapshot::new(
            input.clone(),
            operation_graph(vec![
                first.clone(),
                scalar_operation(
                    0xe001,
                    "rusttable.exposure",
                    false,
                    OperationOpacity::ONE,
                    &[("stops", 3.0)],
                ),
                scalar_operation(
                    0xe002,
                    "rusttable.linear_offset",
                    true,
                    OperationOpacity::ZERO,
                    &[("value", 0.25)],
                ),
                second.clone(),
            ]),
            CpuPixelpipeOutputMode::FullExport,
        );
        let direct = CpuPixelpipeSnapshot::new(
            input,
            operation_graph(vec![first, second]),
            CpuPixelpipeOutputMode::FullExport,
        );

        assert!(is_colorcontrast_chain(&routed, routed.input()));
        assert_eq!(
            CpuPixelpipeExecutor
                .execute(&routed)
                .expect("active chain with inert RGB nodes")
                .image(),
            CpuPixelpipeExecutor
                .execute(&direct)
                .expect("direct active chain")
                .image()
        );
    }

    #[test]
    fn external_matrix_colorcontrast_chain_matches_one_lab_boundary() {
        let profile = ProfileId::from_content(
            b"CPU Color Contrast continuous Lab matrix profile",
            ProfileClass::Input,
            ProfileModel::Matrix,
            Pcs::XyzD50,
            ProfileParserVersion::new(1).expect("parser version"),
        )
        .expect("profile identity");
        let source_color = SourceColor::external(
            profile,
            Primaries::display_p3(),
            TransferFunction::Srgb,
            SourceColorEvidence::EmbeddedChromaticities,
        )
        .expect("external matrix source");
        let dimensions = RasterDimensions::new(3, 1).expect("dimensions");
        let input = RgbaF32Image::new(
            RgbaF32Descriptor::new(dimensions, RgbaF32ColorEncoding::External(profile))
                .with_source_color(source_color),
            vec![
                RgbaF32Pixel::new(0.82, 0.15, 0.07, 0.2),
                RgbaF32Pixel::new(0.12, 0.68, 0.31, 0.6),
                RgbaF32Pixel::new(0.21, 0.24, 0.91, 0.9),
            ],
        )
        .expect("external matrix input");
        let configs = [
            ColorContrastConfig::new(1.35, 4.0, 0.75, -3.0, 1).expect("first config"),
            ColorContrastConfig::new(0.65, -2.0, 1.45, 5.0, 1).expect("second config"),
        ];
        let snapshot = CpuPixelpipeSnapshot::new(
            input.clone(),
            operation_graph(vec![
                colorcontrast_operation(
                    0xcc21,
                    true,
                    OperationOpacity::ONE,
                    [1.35, 4.0, 0.75, -3.0],
                    1,
                ),
                colorcontrast_operation(
                    0xcc22,
                    true,
                    OperationOpacity::ONE,
                    [0.65, -2.0, 1.45, 5.0],
                    1,
                ),
            ]),
            CpuPixelpipeOutputMode::FullExport,
        );

        assert!(is_colorcontrast_chain(&snapshot, snapshot.input()));
        assert_eq!(
            CpuPixelpipeExecutor
                .execute(&snapshot)
                .expect("external matrix Color Contrast chain")
                .image(),
            &one_boundary_colorcontrast_reference(
                &input,
                &configs,
                CpuPixelpipeOutputMode::FullExport,
            )
        );
    }

    #[test]
    fn typed_graph_cancellation_maps_to_the_node_scope() {
        let scope =
            CancellationScope::root(PipelineGeneration::new(7).expect("nonzero generation"))
                .child(CancellationStage::Node);
        scope.cancel(CancellationReason::EditChanged);

        let error = cancellable_evaluation_error(
            EvaluationError::Cancelled {
                step_index: PipelineStepIndex::new(0),
                operation_id: OperationId::new(1).expect("operation ID"),
            },
            Some(&scope),
        );

        let CpuPixelpipeError::Cancelled(error) = error else {
            panic!("typed graph cancellation must remain typed at the pixelpipe boundary");
        };
        assert_eq!(error.reason(), CancellationReason::EditChanged);
        assert_eq!(error.stage(), Some(CancellationStage::Node));
    }

    #[test]
    fn typed_preparation_cancellation_maps_to_the_analysis_scope() {
        let scope =
            CancellationScope::root(PipelineGeneration::new(8).expect("nonzero generation"))
                .child(CancellationStage::Analysis);
        scope.cancel(CancellationReason::UserRequested);

        let error = cancellable_evaluation_error(
            EvaluationError::Cancelled {
                step_index: PipelineStepIndex::new(1),
                operation_id: OperationId::new(2).expect("operation ID"),
            },
            Some(&scope),
        );

        let CpuPixelpipeError::Cancelled(error) = error else {
            panic!("automatic preparation cancellation must be terminal at the CPU boundary");
        };
        assert_eq!(error.reason(), CancellationReason::UserRequested);
        assert_eq!(error.stage(), Some(CancellationStage::Analysis));
    }

    #[test]
    fn singleton_filter_cancellation_maps_to_the_node_scope() {
        let censorize = singleton_graph(
            3,
            "rusttable.censorize",
            &[
                ("radius_1", 0.0),
                ("pixelate", 1.0),
                ("radius_2", 0.0),
                ("noise", 0.0),
            ],
        );
        let clahe = singleton_graph(4, "rusttable.clahe", &[("radius", 2.0), ("slope", 2.0)]);
        let scope =
            CancellationScope::root(PipelineGeneration::new(9).expect("nonzero generation"))
                .child(CancellationStage::Node);
        scope.cancel(CancellationReason::EditChanged);

        for error in [
            censorize_evaluation_error(
                censorize.nodes().next().expect("censorize node"),
                &rusttable_processing::CensorizeExecutionError::Cancelled,
                Some(&scope),
            ),
            clahe_evaluation_error(
                clahe.nodes().next().expect("CLAHE node"),
                &rusttable_processing::ClaheExecutionError::Cancelled,
                Some(&scope),
            ),
        ] {
            let CpuPixelpipeError::Cancelled(error) = error else {
                panic!("singleton cancellation must remain terminal at the CPU boundary");
            };
            assert_eq!(error.reason(), CancellationReason::EditChanged);
            assert_eq!(error.stage(), Some(CancellationStage::Node));
        }
    }

    fn singleton_graph(
        operation_id: u128,
        key: &str,
        parameters: &[(&str, f64)],
    ) -> rusttable_processing::CompiledOperationGraph {
        let operation =
            scalar_operation(operation_id, key, true, OperationOpacity::ONE, parameters);
        operation_graph(vec![operation])
    }

    fn operation_graph(operations: Vec<Operation>) -> rusttable_processing::CompiledOperationGraph {
        let edit = Edit::from_parts(
            EditId::new(1).expect("edit ID"),
            PhotoId::new(2).expect("photo ID"),
            Revision::ZERO,
            Revision::from_u64(1),
            operations,
        )
        .expect("edit");
        rusttable_processing::CompiledOperationGraph::compile(&edit).expect("compiled graph")
    }

    fn scalar_operation(
        operation_id: u128,
        key: &str,
        enabled: bool,
        opacity: OperationOpacity,
        parameters: &[(&str, f64)],
    ) -> Operation {
        Operation::new_with_opacity(
            OperationId::new(operation_id).expect("operation ID"),
            OperationKey::new(key).expect("operation key"),
            enabled,
            opacity,
            parameters.iter().map(|(name, value)| {
                (
                    ParameterName::new(*name).expect("parameter name"),
                    ParameterValue::Scalar(FiniteF64::new(*value).expect("finite parameter")),
                )
            }),
        )
        .expect("operation")
    }

    fn colorcontrast_operation(
        operation_id: u128,
        enabled: bool,
        opacity: OperationOpacity,
        parameters: [f64; 4],
        unbound: i64,
    ) -> Operation {
        let [a_steepness, a_offset, b_steepness, b_offset] = parameters;
        let scalar = |value| {
            ParameterValue::Scalar(FiniteF64::new(value).expect("finite Color Contrast parameter"))
        };
        Operation::new_with_opacity(
            OperationId::new(operation_id).expect("operation ID"),
            OperationKey::new("rusttable.colorcontrast").expect("operation key"),
            enabled,
            opacity,
            [
                ("a_steepness", scalar(a_steepness)),
                ("a_offset", scalar(a_offset)),
                ("b_steepness", scalar(b_steepness)),
                ("b_offset", scalar(b_offset)),
                ("unbound", ParameterValue::Integer(unbound)),
            ]
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
        )
        .expect("Color Contrast operation")
    }

    fn linear_colorcontrast_input() -> RgbaF32Image {
        RgbaF32Image::new(
            RgbaF32Descriptor::new(
                RasterDimensions::new(3, 1).expect("dimensions"),
                RgbaF32ColorEncoding::LinearSrgbD65,
            ),
            vec![
                RgbaF32Pixel::new(0.62, 0.17, 0.08, 0.25),
                RgbaF32Pixel::new(0.09, 0.54, 0.23, 0.5),
                RgbaF32Pixel::new(0.14, 0.19, 0.73, 0.75),
            ],
        )
        .expect("linear Color Contrast input")
    }

    fn assert_inert_lab_identity(operation: &Operation) {
        let input = extreme_lab_colorcontrast_input();
        let expected_bits = rgba_bits(&input);
        for mode in [
            CpuPixelpipeOutputMode::Preview,
            CpuPixelpipeOutputMode::FullExport,
        ] {
            let snapshot = CpuPixelpipeSnapshot::new(
                input.clone(),
                operation_graph(vec![operation.clone()]),
                mode,
            );
            let output = CpuPixelpipeExecutor
                .execute(&snapshot)
                .expect("inert Lab graph")
                .image()
                .clone();

            assert_eq!(output.descriptor(), input.descriptor());
            assert_eq!(rgba_bits(&output), expected_bits);
        }
    }

    fn extreme_lab_colorcontrast_input() -> RgbaF32Image {
        RgbaF32Image::new(
            RgbaF32Descriptor::new(
                RasterDimensions::new(2, 1).expect("dimensions"),
                RgbaF32ColorEncoding::LabD50,
            ),
            vec![
                RgbaF32Pixel::new(f32::MAX, f32::MAX, -f32::MAX, 0.333_333_34),
                RgbaF32Pixel::new(72.765_43, 31.234_56, -42.345_67, 0.777_777_8),
            ],
        )
        .expect("extreme finite Lab Color Contrast input")
    }

    fn rgba_bits(image: &RgbaF32Image) -> Vec<[u32; 4]> {
        image
            .pixels()
            .iter()
            .map(|pixel| {
                [
                    pixel.red().to_bits(),
                    pixel.green().to_bits(),
                    pixel.blue().to_bits(),
                    pixel.alpha().to_bits(),
                ]
            })
            .collect()
    }

    fn one_boundary_colorcontrast_reference(
        input: &RgbaF32Image,
        configs: &[ColorContrastConfig],
        mode: CpuPixelpipeOutputMode,
    ) -> RgbaF32Image {
        let working = to_linear_working(input).expect("matrix source ingress");
        let to_lab = color_transform(
            working.frame().encoding(),
            rusttable_color::ColorEncoding::LabD50,
        )
        .expect("RGB-to-Lab plan");
        let from_lab = color_transform(
            rusttable_color::ColorEncoding::LabD50,
            working.frame().encoding(),
        )
        .expect("Lab-to-RGB plan");
        let mut lab = working
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                let channels = to_lab
                    .apply_rgb(
                        [pixel.red().get(), pixel.green().get(), pixel.blue().get()],
                        || false,
                    )
                    .expect("RGB-to-Lab reference");
                ColorContrastPixel::new(channels[0], channels[1], channels[2], source.alpha())
            })
            .collect::<Vec<_>>();
        for config in configs {
            lab = ColorContrastPlan::new(*config).execute_lab(&lab);
        }
        let pixels = lab
            .iter()
            .map(|pixel| {
                let channels = pixel.channels();
                let rgb = from_lab
                    .apply_rgb([channels[0], channels[1], channels[2]], || false)
                    .expect("Lab-to-RGB reference");
                LinearRgb::new(
                    FiniteF32::new(rgb[0]).expect("finite reference red"),
                    FiniteF32::new(rgb[1]).expect("finite reference green"),
                    FiniteF32::new(rgb[2]).expect("finite reference blue"),
                )
            })
            .collect::<Vec<_>>();
        let evaluated = WorkingRgbImage::new_with_frame(
            input.descriptor().dimensions(),
            pixels,
            working.frame(),
        )
        .expect("reference working image");
        output_from_working(mode, input, &evaluated).expect("reference output")
    }
}
