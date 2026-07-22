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
    BasicAdjPlanSet, EvaluationError, FiniteF32, LinearRgb, OperationMaskSet,
    OperationMaskSetError, SourceRgb, SourceRgbImage, SrgbChannel, WorkingRgbImage,
    convert_working_to_linear_srgb, encode_working_to_srgb,
    evaluate_graph_with_basicadj_plans_and_masks, prepare_basicadj_plans, to_linear_srgb,
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
}

/// Failure from the narrow scalar CPU pixelpipe executor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CpuPixelpipeError {
    Cancelled(CancellationError),
    UnsupportedInputEncoding { actual: RgbaF32ColorEncoding },
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
        let plans = Self::prepare_plans(request)?;
        let image = Self::execute_image(request, request.input(), &plans, masks.as_ref())?;
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
        let plans = Self::prepare_plans(request)?;
        let image = Self::execute_image(request, request.input(), &plans, masks.as_ref())?;
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
        if request.graph().nodes().any(|node| {
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
        }) {
            // Both Darktable operations freeze full-image evidence before
            // replacement. Running them independently per tile changes their
            // result, so the legal tiled contract is a full-frame analysis
            // followed by one publication.
            return self.execute(request);
        }
        let plans = Self::prepare_plans(request)?;
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
            let tile_output =
                Self::execute_image(request, &tile_input, &plans, tile_masks.as_ref())?;
            assemble_tile(
                &mut assembled,
                request.input().descriptor(),
                tile,
                &tile_output,
            )?;
        }

        let output_descriptor = RgbaF32Descriptor::with_source_representation(
            request.input().descriptor().dimensions(),
            request.output_mode().color_encoding(),
            request.input().descriptor().source_representation(),
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
        if request.graph().nodes().any(|node| {
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
        }) {
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
        let plans = Self::prepare_plans(request)?;
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
            let tile_output =
                Self::execute_image(request, &tile_input, &plans, tile_masks.as_ref())?;
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
        let output_descriptor = RgbaF32Descriptor::with_source_representation(
            request.input().descriptor().dimensions(),
            request.output_mode().color_encoding(),
            request.input().descriptor().source_representation(),
        );
        let image = RgbaF32Image::new(output_descriptor, assembled)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })?;
        Ok(Self::result_for(request, image, plans.identity(), [0; 32]))
    }

    fn execute_image(
        request: &CpuPixelpipeSnapshot,
        input: &RgbaF32Image,
        plans: &BasicAdjPlanSet,
        masks: Option<&OperationMaskSet>,
    ) -> Result<RgbaF32Image, CpuPixelpipeError> {
        validate_input_encoding(input)?;

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Censorize { .. }
            )
        }) && request.graph().nodes().count() == 1
            && masks.is_none()
        {
            return execute_censorize_image(request, input, node);
        }

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Clahe { .. }
            )
        }) && request.graph().nodes().count() == 1
            && masks.is_none()
        {
            return execute_clahe_image(request, input, node);
        }

        let linear_input = to_linear_working(input)?;
        let evaluated = evaluate_graph_with_basicadj_plans_and_masks(
            request.graph(),
            &linear_input,
            Some(plans),
            masks,
        )
        .map_err(|source| CpuPixelpipeError::Evaluation { source })?;
        let output_encoding = output_encoding(request, input);
        let output_descriptor = RgbaF32Descriptor::with_source_representation(
            input.descriptor().dimensions(),
            output_encoding,
            input.descriptor().source_representation(),
        );
        let output_pixels = output_pixels(request.output_mode(), &evaluated, input)?;
        RgbaF32Image::new(output_descriptor, output_pixels)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
    }

    fn prepare_plans(request: &CpuPixelpipeSnapshot) -> Result<BasicAdjPlanSet, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        let linear = to_linear_working(request.input())?;
        prepare_basicadj_plans(request.graph(), &linear)
            .map_err(|source| CpuPixelpipeError::Evaluation { source })
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

fn execute_censorize_image(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    node: &rusttable_processing::OperationGraphNode,
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
            .map_err(|source| censorize_evaluation_error(node, &source))?;
    let output = plan
        .execute_with_mask(&rgba, None, node.operation().opacity().get(), || false)
        .map_err(|source| censorize_evaluation_error(node, &source))?;
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
    let descriptor = RgbaF32Descriptor::new(
        input.descriptor().dimensions(),
        request.output_mode().color_encoding(),
    );
    RgbaF32Image::new(descriptor, output_pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

fn censorize_evaluation_error(
    node: &rusttable_processing::OperationGraphNode,
    source: &rusttable_processing::CensorizeExecutionError,
) -> CpuPixelpipeError {
    CpuPixelpipeError::Evaluation {
        source: EvaluationError::OperationExecution {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
            reason: source.to_string(),
        },
    }
}

fn execute_clahe_image(
    request: &CpuPixelpipeSnapshot,
    input: &RgbaF32Image,
    node: &rusttable_processing::OperationGraphNode,
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
            .map_err(|source| clahe_evaluation_error(node, &source))?;
    let output = plan
        .execute_with_mask(&pixels, None, node.operation().opacity().get(), || false)
        .map_err(|source| clahe_evaluation_error(node, &source))?;
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
    let descriptor = RgbaF32Descriptor::new(
        input.descriptor().dimensions(),
        request.output_mode().color_encoding(),
    );
    RgbaF32Image::new(descriptor, output_pixels)
        .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
}

fn clahe_evaluation_error(
    node: &rusttable_processing::OperationGraphNode,
    source: &rusttable_processing::ClaheExecutionError,
) -> CpuPixelpipeError {
    CpuPixelpipeError::Evaluation {
        source: EvaluationError::OperationExecution {
            step_index: node.pipeline_step_index(),
            operation_id: node.operation().operation_id(),
            reason: source.to_string(),
        },
    }
}

fn validate_input_encoding(input: &RgbaF32Image) -> Result<(), CpuPixelpipeError> {
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
        RgbaF32Descriptor::with_source_representation(
            tile.dimensions(),
            input.descriptor().color_encoding(),
            input.descriptor().source_representation(),
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

fn output_encoding(request: &CpuPixelpipeSnapshot, input: &RgbaF32Image) -> RgbaF32ColorEncoding {
    if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
        RgbaF32ColorEncoding::LabD50
    } else {
        request.output_mode().color_encoding()
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
    if let Some(source_color) = input.descriptor().source_color() {
        return to_colorin_working(input, source_color);
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
    let source = to_processing_source(input)?;
    Ok(to_linear_srgb(&source))
}

fn to_colorin_working(
    input: &RgbaF32Image,
    source_color: rusttable_image::SourceColor,
) -> Result<WorkingRgbImage, CpuPixelpipeError> {
    let id = source_color
        .profile()
        .map_or_else(|| synthetic_profile(source_color), Ok)?;
    let input_profile = ColorInProfile::Matrix {
        id,
        primaries: source_color.primaries(),
        transfer: source_color.transfer(),
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
    source_color: rusttable_image::SourceColor,
) -> Result<ProfileId, CpuPixelpipeError> {
    let bytes = postcard::to_allocvec(&(source_color.encoding(), source_color.transfer()))
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
