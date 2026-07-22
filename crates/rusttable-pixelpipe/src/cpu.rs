#![allow(clippy::missing_errors_doc, clippy::match_same_arms)]

use std::fmt;

use rusttable_processing::{
    BasicAdjPlanSet, EvaluationError, FiniteF32, LinearRgb, SourceRgb, SourceRgbImage, SrgbChannel,
    WorkingRgbImage, encode_linear_srgb, evaluate_graph_with_basicadj_plans,
    prepare_basicadj_plans, to_linear_srgb,
};

use crate::frame::{execute_frame_image, has_discrete_geometry};
use crate::{
    CancellationError, CancellationScope, CancellationStage, CpuNodeReceipt, CpuPipelineReceipt,
    CpuPixelpipeSnapshot, CpuTilePlan, CpuTilePlanError, PixelIdentity, RgbaF32Channel,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32ImageError, RgbaF32Pixel,
};

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
    InputBridge { source: RgbaF32ImageError },
    Evaluation { source: EvaluationError },
    OutputBoundary { source: RgbaF32ImageError },
    TilePlan { source: CpuTilePlanError },
    TileAssembly { source: CpuTileAssemblyError },
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
        if has_discrete_geometry(request) {
            let (image, plan_identity) = execute_frame_image(request, request.input(), None)?;
            return Ok(Self::result_for(request, image, plan_identity));
        }
        let plans = Self::prepare_plans(request)?;
        let image = Self::execute_image(request, request.input(), &plans)?;
        Ok(Self::result_for(request, image, plans.identity()))
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
        if has_discrete_geometry(request) {
            let (image, plan_identity) =
                execute_frame_image(request, request.input(), Some(scope))?;
            scope
                .child(CancellationStage::Publication)
                .check()
                .map_err(CpuPixelpipeError::Cancelled)?;
            return Ok(Self::result_for(request, image, plan_identity));
        }
        let plans = Self::prepare_plans(request)?;
        let image = Self::execute_image(request, request.input(), &plans)?;
        scope
            .child(CancellationStage::Publication)
            .check()
            .map_err(CpuPixelpipeError::Cancelled)?;
        Ok(Self::result_for(request, image, plans.identity()))
    }

    /// Executes a point-operation graph in deterministic, row-major tiles.
    ///
    /// Each tile is evaluated through the same scalar operation path as a
    /// full-frame request. Tile results are copied back into their original
    /// row-major positions, so this path produces the exact full-frame image
    /// and receipt identity for the currently supported point operations.
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
            matches!(
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
                    | rusttable_processing::ProcessingOperationKind::LensCorrection { .. }
                    | rusttable_processing::ProcessingOperationKind::Grain { .. }
                    | rusttable_processing::ProcessingOperationKind::Censorize { .. }
                    | rusttable_processing::ProcessingOperationKind::Defringe { .. }
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
            let tile_output = Self::execute_image(request, &tile_input, &plans)?;
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
        Ok(Self::result_for(request, image, plans.identity()))
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
            matches!(
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
                    | rusttable_processing::ProcessingOperationKind::LensCorrection { .. }
                    | rusttable_processing::ProcessingOperationKind::Grain { .. }
                    | rusttable_processing::ProcessingOperationKind::Censorize { .. }
                    | rusttable_processing::ProcessingOperationKind::Defringe { .. }
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
            let tile_output = Self::execute_image(request, &tile_input, &plans)?;
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
        Ok(Self::result_for(request, image, plans.identity()))
    }

    fn execute_image(
        request: &CpuPixelpipeSnapshot,
        input: &RgbaF32Image,
        plans: &BasicAdjPlanSet,
    ) -> Result<RgbaF32Image, CpuPixelpipeError> {
        validate_input_encoding(input)?;

        if input.descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
            if let Some(node) = request.graph().nodes().find(|node| {
                matches!(
                    node.operation().kind(),
                    rusttable_processing::ProcessingOperationKind::Defringe { .. }
                )
            }) && request.graph().nodes().count() == 1
            {
                return execute_defringe_image(input, node);
            }
            return Err(CpuPixelpipeError::UnsupportedInputEncoding {
                actual: RgbaF32ColorEncoding::LabD50,
            });
        }

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Censorize { .. }
            )
        }) && request.graph().nodes().count() == 1
        {
            return execute_censorize_image(request, input, node);
        }

        if let Some(node) = request.graph().nodes().find(|node| {
            matches!(
                node.operation().kind(),
                rusttable_processing::ProcessingOperationKind::Clahe { .. }
            )
        }) && request.graph().nodes().count() == 1
        {
            return execute_clahe_image(request, input, node);
        }

        let linear_input = to_linear_working(input)?;
        let evaluated =
            evaluate_graph_with_basicadj_plans(request.graph(), &linear_input, Some(plans))
                .map_err(|source| CpuPixelpipeError::Evaluation { source })?;
        let output_descriptor = RgbaF32Descriptor::with_source_representation(
            input.descriptor().dimensions(),
            request.output_mode().color_encoding(),
            input.descriptor().source_representation(),
        );
        let output_pixels = output_pixels(request.output_mode(), &evaluated, input);
        RgbaF32Image::new(output_descriptor, output_pixels)
            .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
    }

    fn prepare_plans(request: &CpuPixelpipeSnapshot) -> Result<BasicAdjPlanSet, CpuPixelpipeError> {
        validate_input_encoding(request.input())?;
        if request.input().descriptor().color_encoding() == RgbaF32ColorEncoding::LabD50 {
            return Ok(BasicAdjPlanSet::default());
        }
        let linear = to_linear_working(request.input())?;
        prepare_basicadj_plans(request.graph(), &linear)
            .map_err(|source| CpuPixelpipeError::Evaluation { source })
    }

    fn result_for(
        request: &CpuPixelpipeSnapshot,
        image: RgbaF32Image,
        basicadj_plan_identity: [u8; 32],
    ) -> CpuPixelpipeResult {
        let receipt = CpuPipelineReceipt::new(
            request.input().descriptor(),
            image.descriptor(),
            request.source_identity(),
            (pixel_identity(request.input()), pixel_identity(&image)),
            request.identity(),
            basicadj_plan_identity,
            request.output_mode(),
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
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(&working)
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

fn execute_defringe_image(
    input: &RgbaF32Image,
    node: &rusttable_processing::OperationGraphNode,
) -> Result<RgbaF32Image, CpuPixelpipeError> {
    let config = match node.operation().kind() {
        rusttable_processing::ProcessingOperationKind::Defringe { config } => *config,
        _ => unreachable!("defringe image bridge is only called for defringe"),
    };
    let pixels = input
        .pixels()
        .iter()
        .map(|pixel| {
            rusttable_processing::DefringePixel::new(
                pixel.red(),
                pixel.green(),
                pixel.blue(),
                pixel.alpha(),
            )
        })
        .collect::<Vec<_>>();
    let plan =
        rusttable_processing::DefringePlan::new(config, input.descriptor().dimensions(), 1.0, 1.0)
            .map_err(|source| defringe_evaluation_error(node, &source))?;
    let output = plan
        .execute_with_mask(&pixels, None, node.operation().opacity().get(), || false)
        .map_err(|source| defringe_evaluation_error(node, &source))?;
    let output_pixels = output
        .into_iter()
        .map(|pixel| {
            let channels = pixel.channels();
            RgbaF32Pixel::new(channels[0], channels[1], channels[2], channels[3])
        })
        .collect();
    RgbaF32Image::new(
        RgbaF32Descriptor::new(
            input.descriptor().dimensions(),
            RgbaF32ColorEncoding::LabD50,
        ),
        output_pixels,
    )
    .map_err(|source| CpuPixelpipeError::OutputBoundary { source })
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
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(&working)
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

fn defringe_evaluation_error(
    node: &rusttable_processing::OperationGraphNode,
    source: &rusttable_processing::DefringeExecutionError,
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

fn assemble_tile(
    assembled: &mut [RgbaF32Pixel],
    output_descriptor: RgbaF32Descriptor,
    tile: crate::CpuPixelpipeTile,
    tile_output: &RgbaF32Image,
) -> Result<(), CpuPixelpipeError> {
    if tile_output.descriptor().dimensions() != tile.dimensions() {
        return Err(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::TileOutputDimensionsMismatch,
        });
    }
    for local_y in 0..tile.dimensions().height() {
        let output_y =
            tile.origin_y()
                .checked_add(local_y)
                .ok_or(CpuPixelpipeError::TileAssembly {
                    source: CpuTileAssemblyError::PixelIndexOverflow,
                })?;
        let destination_start = pixel_index(output_descriptor, tile.origin_x(), output_y)?;
        let destination_end = checked_row_end(destination_start, tile.dimensions().width())?;
        let destination = assembled
            .get_mut(destination_start..destination_end)
            .ok_or(CpuPixelpipeError::TileAssembly {
                source: CpuTileAssemblyError::DestinationRowOutsideOutput,
            })?;
        let source_start = pixel_index(tile_output.descriptor(), 0, local_y)?;
        let source_end = checked_row_end(source_start, tile.dimensions().width())?;
        let source = tile_output.pixels().get(source_start..source_end).ok_or(
            CpuPixelpipeError::TileAssembly {
                source: CpuTileAssemblyError::SourceRowOutsideInput,
            },
        )?;
        destination.copy_from_slice(source);
    }
    Ok(())
}

fn tile_pixel_count(tile: crate::CpuPixelpipeTile) -> Result<usize, CpuPixelpipeError> {
    usize::try_from(tile.dimensions().pixel_count()).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform {
            index: tile.dimensions().pixel_count(),
        },
    })
}

fn pixel_index(descriptor: RgbaF32Descriptor, x: u32, y: u32) -> Result<usize, CpuPixelpipeError> {
    let offset = u64::from(y)
        .checked_mul(u64::from(descriptor.dimensions().width()))
        .and_then(|row_offset| row_offset.checked_add(u64::from(x)))
        .ok_or(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::PixelIndexOverflow,
        })?;
    usize::try_from(offset).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform { index: offset },
    })
}

fn checked_row_end(start: usize, width: u32) -> Result<usize, CpuPixelpipeError> {
    let width = usize::try_from(width).map_err(|_| CpuPixelpipeError::TileAssembly {
        source: CpuTileAssemblyError::PixelIndexExceedsPlatform {
            index: u64::from(width),
        },
    })?;
    start
        .checked_add(width)
        .ok_or(CpuPixelpipeError::TileAssembly {
            source: CpuTileAssemblyError::RowEndOverflow,
        })
}

fn output_pixels(
    mode: CpuPixelpipeOutputMode,
    evaluated: &rusttable_processing::WorkingRgbImage,
    input: &RgbaF32Image,
) -> Vec<RgbaF32Pixel> {
    match mode {
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(evaluated)
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
            .collect(),
        CpuPixelpipeOutputMode::FullExport => evaluated
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
            .collect(),
    }
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

impl fmt::Display for CpuPixelpipeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(error) => error.fmt(formatter),
            Self::UnsupportedInputEncoding { actual } => {
                write!(formatter, "CPU pixelpipe does not accept {actual:?} input")
            }
            Self::InputBridge { source } => write!(formatter, "invalid CPU input bridge: {source}"),
            Self::Evaluation { source } => {
                write!(formatter, "CPU operation evaluation failed: {source}")
            }
            Self::OutputBoundary { source } => {
                write!(formatter, "invalid CPU output boundary: {source}")
            }
            Self::TilePlan { source } => write!(formatter, "invalid CPU tile plan: {source}"),
            Self::TileAssembly { source } => {
                write!(formatter, "invalid CPU tile assembly: {source}")
            }
        }
    }
}

impl std::error::Error for CpuPixelpipeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled(_) => None,
            Self::UnsupportedInputEncoding { .. } => None,
            Self::InputBridge { source } | Self::OutputBoundary { source } => Some(source),
            Self::Evaluation { source } => Some(source),
            Self::TilePlan { source } => Some(source),
            Self::TileAssembly { source } => Some(source),
        }
    }
}

impl fmt::Display for CpuTileAssemblyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PixelIndexOverflow => formatter.write_str("CPU tile pixel index overflowed"),
            Self::PixelIndexExceedsPlatform { index } => {
                write!(
                    formatter,
                    "CPU tile pixel index {index} exceeds this platform"
                )
            }
            Self::RowEndOverflow => formatter.write_str("CPU tile row end overflowed"),
            Self::SourceRowOutsideInput => {
                formatter.write_str("CPU tile source row is out of bounds")
            }
            Self::DestinationRowOutsideOutput => {
                formatter.write_str("CPU tile destination row is out of bounds")
            }
            Self::TileUnavailable => formatter.write_str("CPU tile grid omitted a planned tile"),
            Self::TileOutputDimensionsMismatch => {
                formatter.write_str("CPU tile output dimensions do not match its input tile")
            }
        }
    }
}

impl std::error::Error for CpuTileAssemblyError {}
