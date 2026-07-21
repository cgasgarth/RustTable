use rusttable_gpu::{
    BasicPointError, BasicPointOperation, BasicPointRequest, GpuRuntime, GrainPointError,
    GrainPointRequest,
};
use rusttable_processing::{
    FiniteF32, GrainPlan, LinearRgb, RasterDimensions, SourceRgb, SourceRgbImage, SrgbChannel,
    WorkingRgbImage, encode_linear_srgb, to_linear_srgb,
};
use sha2::{Digest, Sha256};

use crate::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeSnapshot,
    RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image, RgbaF32ImageError, RgbaF32Pixel,
};

/// The backend that published one pixelpipe result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelpipeBackend {
    CpuCanonical,
    CpuTiledFallback,
    WgpuBasic,
    WgpuTiled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum GpuExecutionError {
    Basic(BasicPointError),
    Grain(GrainPointError),
}

impl From<BasicPointError> for GpuExecutionError {
    fn from(error: BasicPointError) -> Self {
        Self::Basic(error)
    }
}

impl From<GrainPointError> for GpuExecutionError {
    fn from(error: GrainPointError) -> Self {
        Self::Grain(error)
    }
}

impl GpuExecutionError {
    fn into_receipt_error(self) -> BasicPointError {
        match self {
            Self::Basic(error) => error,
            Self::Grain(error) => BasicPointError::Readback(error.to_string()),
        }
    }
}

/// Bounded provenance for one tiled execution and its recovery attempts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelpipeTilingReceipt {
    plan_identity: [u8; 32],
    tile_count: u64,
    attempts: u8,
}

impl PixelpipeTilingReceipt {
    #[must_use]
    pub const fn plan_identity(&self) -> [u8; 32] {
        self.plan_identity
    }

    #[must_use]
    pub const fn tile_count(&self) -> u64 {
        self.tile_count
    }

    #[must_use]
    pub const fn attempts(&self) -> u8 {
        self.attempts
    }
}

/// Bounded provenance for one service execution attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PixelpipeExecutionReceipt {
    snapshot_identity: crate::CpuPixelpipeSnapshotIdentity,
    backend: PixelpipeBackend,
    gpu_fallback: Option<BasicPointError>,
    dispatches: u32,
    tiling: Option<PixelpipeTilingReceipt>,
}

impl PixelpipeExecutionReceipt {
    #[must_use]
    pub const fn snapshot_identity(&self) -> crate::CpuPixelpipeSnapshotIdentity {
        self.snapshot_identity
    }

    #[must_use]
    pub const fn backend(&self) -> PixelpipeBackend {
        self.backend
    }

    #[must_use]
    pub const fn gpu_fallback(&self) -> Option<&BasicPointError> {
        self.gpu_fallback.as_ref()
    }

    #[must_use]
    pub const fn dispatches(&self) -> u32 {
        self.dispatches
    }

    #[must_use]
    pub const fn tiling(&self) -> Option<&PixelpipeTilingReceipt> {
        self.tiling.as_ref()
    }
}

/// An image and the backend receipt that authorized its publication.
#[derive(Debug, Clone, PartialEq)]
pub struct PixelpipeExecutionResult {
    image: RgbaF32Image,
    receipt: PixelpipeExecutionReceipt,
}

impl PixelpipeExecutionResult {
    #[must_use]
    pub const fn image(&self) -> &RgbaF32Image {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> &PixelpipeExecutionReceipt {
        &self.receipt
    }
}

/// Application-facing basic pixelpipe coordinator.
///
/// GPU eligibility is derived from the immutable snapshot. The coordinator
/// never skips an enabled unsupported node and always retains the canonical
/// CPU executor as the publication path when GPU preparation or execution
/// fails.
#[derive(Debug)]
pub struct PixelpipeExecutionService {
    cpu: CpuPixelpipeExecutor,
    gpu: Option<GpuRuntime>,
}

impl PixelpipeExecutionService {
    #[must_use]
    pub const fn cpu_only() -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: None,
        }
    }

    #[must_use]
    pub fn with_gpu(gpu: GpuRuntime) -> Self {
        Self {
            cpu: CpuPixelpipeExecutor,
            gpu: Some(gpu),
        }
    }

    /// Executes the snapshot, selecting WGPU only for the currently qualified
    /// basic point range and otherwise publishing the canonical CPU result.
    ///
    /// # Errors
    ///
    /// Returns the canonical pixelpipe error when CPU publication fails.
    pub fn execute(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let Some(plan) = gpu_plan(snapshot) else {
            return self.cpu_result(snapshot, None);
        };
        let Some(gpu) = self.gpu.as_ref() else {
            return self.cpu_result(snapshot, None);
        };
        if !gpu.health_check() {
            return self.cpu_result(snapshot, Some(BasicPointError::Unhealthy));
        }

        match execute_gpu(gpu, snapshot, &plan) {
            Ok((image, dispatches)) => Ok(PixelpipeExecutionResult {
                image,
                receipt: PixelpipeExecutionReceipt {
                    snapshot_identity: snapshot.identity(),
                    backend: PixelpipeBackend::WgpuBasic,
                    gpu_fallback: None,
                    dispatches,
                    tiling: None,
                },
            }),
            Err(error) => self.cpu_result(snapshot, Some(error.into_receipt_error())),
        }
    }

    fn cpu_result(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        fallback: Option<BasicPointError>,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let cpu_result = self.cpu.execute(snapshot)?;
        let image = cpu_result.image().clone();
        Ok(PixelpipeExecutionResult {
            image,
            receipt: PixelpipeExecutionReceipt {
                snapshot_identity: snapshot.identity(),
                backend: PixelpipeBackend::CpuCanonical,
                gpu_fallback: fallback,
                dispatches: 0,
                tiling: None,
            },
        })
    }

    /// Executes eligible point operations in row-major tiles with bounded
    /// smaller-tile recovery before publishing the canonical CPU fallback.
    ///
    /// Each GPU attempt uses a fresh tile assembly. A failed attempt cannot
    /// publish partial pixels, and at most three tile plans are tried.
    ///
    /// # Errors
    ///
    /// Returns the canonical CPU pixelpipe error if every bounded GPU attempt
    /// and its CPU fallback fail.
    pub fn execute_tiled(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        tile_plan: crate::CpuTilePlan,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let Some(plan) = gpu_plan(snapshot) else {
            return self.cpu_tiled_result(snapshot, tile_plan, None, 0);
        };
        let Some(gpu) = self.gpu.as_ref() else {
            return self.cpu_tiled_result(snapshot, tile_plan, None, 0);
        };
        if !gpu.health_check() {
            return self.cpu_tiled_result(snapshot, tile_plan, Some(BasicPointError::Unhealthy), 0);
        }

        let plans = recovery_plans(tile_plan);
        let mut last_error = None;
        for (index, candidate) in plans.iter().copied().enumerate() {
            match execute_gpu_tiled(gpu, snapshot, &plan, candidate) {
                Ok((image, dispatches, tile_count)) => {
                    return Ok(PixelpipeExecutionResult {
                        image,
                        receipt: PixelpipeExecutionReceipt {
                            snapshot_identity: snapshot.identity(),
                            backend: PixelpipeBackend::WgpuTiled,
                            gpu_fallback: None,
                            dispatches,
                            tiling: Some(tiling_receipt(
                                snapshot,
                                candidate,
                                tile_count,
                                index + 1,
                            )),
                        },
                    });
                }
                Err(error) => last_error = Some(error.into_receipt_error()),
            }
        }
        self.cpu_tiled_result(
            snapshot,
            tile_plan,
            last_error,
            u8::try_from(plans.len()).unwrap_or(u8::MAX),
        )
    }

    fn cpu_tiled_result(
        &self,
        snapshot: &CpuPixelpipeSnapshot,
        plan: crate::CpuTilePlan,
        fallback: Option<BasicPointError>,
        attempts: u8,
    ) -> Result<PixelpipeExecutionResult, CpuPixelpipeError> {
        let result = self.cpu.execute_tiled(snapshot, plan)?;
        let grid = plan
            .grid_for(snapshot.input().descriptor().dimensions())
            .map_err(|source| CpuPixelpipeError::TilePlan { source })?;
        Ok(PixelpipeExecutionResult {
            image: result.image().clone(),
            receipt: PixelpipeExecutionReceipt {
                snapshot_identity: snapshot.identity(),
                backend: PixelpipeBackend::CpuTiledFallback,
                gpu_fallback: fallback,
                dispatches: 0,
                tiling: Some(tiling_receipt(
                    snapshot,
                    plan,
                    grid.tile_count(),
                    usize::from(attempts),
                )),
            },
        })
    }
}

fn recovery_plans(initial: crate::CpuTilePlan) -> Vec<crate::CpuTilePlan> {
    let mut plans = vec![initial];
    let mut width = initial.tile_width();
    let mut height = initial.tile_height();
    for _ in 0..2 {
        width = width.div_ceil(2);
        height = height.div_ceil(2);
        let Ok(plan) = crate::CpuTilePlan::new(width, height) else {
            break;
        };
        if plans.last().is_some_and(|previous| *previous == plan) {
            break;
        }
        plans.push(plan);
    }
    plans
}

fn tiling_receipt(
    snapshot: &CpuPixelpipeSnapshot,
    plan: crate::CpuTilePlan,
    tile_count: u64,
    attempts: usize,
) -> PixelpipeTilingReceipt {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.pixelpipe.tiling.v1");
    hasher.update(snapshot.identity().as_bytes());
    hasher.update(plan.tile_width().to_le_bytes());
    hasher.update(plan.tile_height().to_le_bytes());
    PixelpipeTilingReceipt {
        plan_identity: hasher.finalize().into(),
        tile_count,
        attempts: u8::try_from(attempts.min(usize::from(u8::MAX))).unwrap_or(u8::MAX),
    }
}

#[derive(Debug, Clone)]
enum GpuPlan {
    Basic(Vec<BasicPointOperation>),
    Grain(rusttable_processing::operations::grain::GrainConfig),
}

fn gpu_plan(snapshot: &CpuPixelpipeSnapshot) -> Option<GpuPlan> {
    let mut operations = Vec::new();
    let mut grain = None;
    for node in snapshot.graph().nodes() {
        let operation = node.operation();
        if !operation.is_enabled() {
            continue;
        }
        if operation.opacity().get().to_bits() != 1.0_f32.to_bits() {
            return None;
        }
        let gpu_operation = match operation.kind() {
            rusttable_processing::ProcessingOperationKind::Exposure { stops } => {
                BasicPointOperation::Exposure { stops: stops.get() }
            }
            rusttable_processing::ProcessingOperationKind::LinearOffset { value } => {
                BasicPointOperation::LinearOffset { value: value.get() }
            }
            rusttable_processing::ProcessingOperationKind::RgbGain { red, green, blue } => {
                BasicPointOperation::RgbGain {
                    red: red.get(),
                    green: green.get(),
                    blue: blue.get(),
                }
            }
            rusttable_processing::ProcessingOperationKind::Grain { config } => {
                if grain.is_some() || !operations.is_empty() {
                    return None;
                }
                grain = Some(*config);
                continue;
            }
            _ => return None,
        };
        if grain.is_some() {
            return None;
        }
        operations.push(gpu_operation);
    }
    if let Some(config) = grain {
        Some(GpuPlan::Grain(config))
    } else {
        Some(GpuPlan::Basic(operations))
    }
}

fn execute_gpu(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    plan: &GpuPlan,
) -> Result<(RgbaF32Image, u32), GpuExecutionError> {
    match plan {
        GpuPlan::Basic(operations) => {
            execute_gpu_image(gpu, snapshot.input(), snapshot.output_mode(), operations)
        }
        GpuPlan::Grain(config) => execute_gpu_grain_image(
            gpu,
            snapshot.input(),
            snapshot.output_mode(),
            *config,
            snapshot.input().descriptor().dimensions(),
            (0, 0),
        ),
    }
}

fn execute_gpu_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    operations: &[BasicPointOperation],
) -> Result<(RgbaF32Image, u32), GpuExecutionError> {
    let dimensions = input.descriptor().dimensions();
    let source_pixels = input
        .pixels()
        .iter()
        .copied()
        .map(|pixel| {
            Ok(SourceRgb::new(
                SrgbChannel::new(pixel.red())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 0 })?,
                SrgbChannel::new(pixel.green())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 1 })?,
                SrgbChannel::new(pixel.blue())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 2 })?,
            ))
        })
        .collect::<Result<Vec<_>, BasicPointError>>()?;
    let source = SourceRgbImage::new(dimensions, source_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let linear = to_linear_srgb(&source);
    let mut packed = Vec::with_capacity(input.pixels().len() * 4);
    for (working, source) in linear.pixels().zip(input.pixels()) {
        packed.extend([
            working.red().get(),
            working.green().get(),
            working.blue().get(),
            source.alpha(),
        ]);
    }
    let result = gpu.execute_basic_point(BasicPointRequest {
        pixels: &packed,
        operations,
    })?;
    let (packed_pixels, remainder) = result.pixels().as_chunks::<4>();
    debug_assert!(remainder.is_empty(), "GPU output must contain RGBA pixels");
    let mut working_pixels = Vec::with_capacity(input.pixels().len());
    for (index, pixel) in packed_pixels.iter().enumerate() {
        working_pixels.push(LinearRgb::new(
            FiniteF32::new(pixel[0]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4,
            })?,
            FiniteF32::new(pixel[1]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 1,
            })?,
            FiniteF32::new(pixel[2]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 2,
            })?,
        ));
    }
    let working = WorkingRgbImage::new(dimensions, working_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let output_pixels = match output_mode {
        CpuPixelpipeOutputMode::FullExport => working
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(&working)
            .image()
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
    };
    let encoding = match output_mode {
        CpuPixelpipeOutputMode::Preview => RgbaF32ColorEncoding::SrgbD65,
        CpuPixelpipeOutputMode::FullExport => RgbaF32ColorEncoding::LinearSrgbD65,
    };
    let descriptor = RgbaF32Descriptor::new(dimensions, encoding);
    let image = RgbaF32Image::new(descriptor, output_pixels).map_err(|source| match source {
        RgbaF32ImageError::NonFiniteComponent { .. }
        | RgbaF32ImageError::ComponentOutsideUnitInterval { .. }
        | RgbaF32ImageError::PixelCountMismatch { .. }
        | RgbaF32ImageError::SourceIdentityMismatch { .. } => {
            BasicPointError::Readback("GPU output failed the typed image boundary".to_owned())
        }
    })?;
    Ok((image, result.dispatches()))
}

fn execute_gpu_grain_image(
    gpu: &GpuRuntime,
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    config: rusttable_processing::operations::grain::GrainConfig,
    full_dimensions: RasterDimensions,
    origin: (u32, u32),
) -> Result<(RgbaF32Image, u32), GpuExecutionError> {
    let dimensions = input.descriptor().dimensions();
    let source_pixels = input
        .pixels()
        .iter()
        .copied()
        .map(|pixel| {
            Ok(SourceRgb::new(
                SrgbChannel::new(pixel.red())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 0 })?,
                SrgbChannel::new(pixel.green())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 1 })?,
                SrgbChannel::new(pixel.blue())
                    .map_err(|_| BasicPointError::NonFiniteInput { component: 2 })?,
            ))
        })
        .collect::<Result<Vec<_>, BasicPointError>>()?;
    let source = SourceRgbImage::new(dimensions, source_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let linear = to_linear_srgb(&source);
    let mut packed = Vec::with_capacity(input.pixels().len() * 4);
    for (working, source) in linear.pixels().zip(input.pixels()) {
        packed.extend([
            working.red().get(),
            working.green().get(),
            working.blue().get(),
            source.alpha(),
        ]);
    }
    let plan = GrainPlan::new(config, full_dimensions)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let parameters = plan.gpu_parameters();
    let result = gpu.execute_grain_point(GrainPointRequest {
        pixels: &packed,
        width: dimensions.width(),
        height: dimensions.height(),
        full_width: full_dimensions.width(),
        full_height: full_dimensions.height(),
        origin_x: origin.0,
        origin_y: origin.1,
        channel: parameters.channel.id(),
        seed: parameters.seed,
        zoom: parameters.zoom,
        strength: parameters.strength,
        lut: plan.gpu_lut(),
    })?;
    image_from_packed(input, output_mode, result.pixels(), result.dispatches())
}

fn image_from_packed(
    input: &RgbaF32Image,
    output_mode: CpuPixelpipeOutputMode,
    packed: &[f32],
    dispatches: u32,
) -> Result<(RgbaF32Image, u32), GpuExecutionError> {
    let dimensions = input.descriptor().dimensions();
    let (packed_pixels, remainder) = packed.as_chunks::<4>();
    if !remainder.is_empty() || packed_pixels.len() != input.pixels().len() {
        return Err(BasicPointError::InvalidPixelPacking.into());
    }
    let mut working_pixels = Vec::with_capacity(input.pixels().len());
    for (index, pixel) in packed_pixels.iter().enumerate() {
        working_pixels.push(LinearRgb::new(
            FiniteF32::new(pixel[0]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4,
            })?,
            FiniteF32::new(pixel[1]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 1,
            })?,
            FiniteF32::new(pixel[2]).map_err(|_| BasicPointError::NonFiniteInput {
                component: index * 4 + 2,
            })?,
        ));
    }
    let working = WorkingRgbImage::new(dimensions, working_pixels)
        .map_err(|_| BasicPointError::InvalidPixelPacking)?;
    let output_pixels = match output_mode {
        CpuPixelpipeOutputMode::FullExport => working
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
        CpuPixelpipeOutputMode::Preview => encode_linear_srgb(&working)
            .image()
            .pixels()
            .zip(input.pixels())
            .map(|(pixel, source)| {
                RgbaF32Pixel::new(
                    pixel.red().get(),
                    pixel.green().get(),
                    pixel.blue().get(),
                    source.alpha(),
                )
            })
            .collect(),
    };
    let encoding = match output_mode {
        CpuPixelpipeOutputMode::Preview => RgbaF32ColorEncoding::SrgbD65,
        CpuPixelpipeOutputMode::FullExport => RgbaF32ColorEncoding::LinearSrgbD65,
    };
    let descriptor = RgbaF32Descriptor::new(dimensions, encoding);
    let image = RgbaF32Image::new(descriptor, output_pixels).map_err(|_| {
        BasicPointError::Readback("GPU output failed the typed image boundary".to_owned())
    })?;
    Ok((image, dispatches))
}

fn execute_gpu_tiled(
    gpu: &GpuRuntime,
    snapshot: &CpuPixelpipeSnapshot,
    plan: &GpuPlan,
    tile_plan: crate::CpuTilePlan,
) -> Result<(RgbaF32Image, u32, u64), GpuExecutionError> {
    let grid = tile_plan
        .grid_for(snapshot.input().descriptor().dimensions())
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    let input = snapshot.input();
    let mut assembled = vec![None; input.pixels().len()];
    let mut dispatches = 0_u32;
    for tile_index in 0..grid.tile_count() {
        let tile = grid
            .tile_at(tile_index)
            .map_err(|error| BasicPointError::Readback(error.to_string()))?
            .ok_or_else(|| BasicPointError::Readback("tile disappeared from grid".to_owned()))?;
        let tile_input = extract_tile(input, tile)?;
        let (tile_output, tile_dispatches) = match plan {
            GpuPlan::Basic(operations) => {
                execute_gpu_image(gpu, &tile_input, snapshot.output_mode(), operations)?
            }
            GpuPlan::Grain(config) => execute_gpu_grain_image(
                gpu,
                &tile_input,
                snapshot.output_mode(),
                *config,
                input.descriptor().dimensions(),
                (tile.origin_x(), tile.origin_y()),
            )?,
        };
        dispatches = dispatches.saturating_add(tile_dispatches);
        place_tile(&mut assembled, input, tile, &tile_output)?;
    }
    let pixels = assembled
        .into_iter()
        .map(|pixel| pixel.ok_or_else(|| BasicPointError::Readback("tiled output gap".to_owned())))
        .collect::<Result<Vec<_>, _>>()?;
    let descriptor = RgbaF32Descriptor::new(
        input.descriptor().dimensions(),
        snapshot.output_mode().color_encoding(),
    );
    let output = RgbaF32Image::new(descriptor, pixels)
        .map_err(|error| BasicPointError::Readback(error.to_string()))?;
    Ok((output, dispatches, grid.tile_count()))
}

fn extract_tile(
    input: &RgbaF32Image,
    tile: crate::CpuPixelpipeTile,
) -> Result<RgbaF32Image, BasicPointError> {
    let dimensions = tile.dimensions();
    let source_width = input.descriptor().dimensions().width();
    let pixel_count = usize::try_from(dimensions.pixel_count())
        .map_err(|_| BasicPointError::Readback("tile pixel count is too large".to_owned()))?;
    let mut pixels = Vec::with_capacity(pixel_count);
    for y in 0..dimensions.height() {
        let row = u64::from(tile.origin_y() + y)
            .checked_mul(u64::from(source_width))
            .and_then(|offset| offset.checked_add(u64::from(tile.origin_x())))
            .ok_or_else(|| BasicPointError::Readback("tile source index overflow".to_owned()))?;
        let start = usize::try_from(row)
            .map_err(|_| BasicPointError::Readback("tile source index is too large".to_owned()))?;
        let end = start
            .checked_add(dimensions.width() as usize)
            .ok_or_else(|| BasicPointError::Readback("tile row overflow".to_owned()))?;
        let row_pixels = input.pixels().get(start..end).ok_or_else(|| {
            BasicPointError::Readback("tile source row is out of bounds".to_owned())
        })?;
        pixels.extend_from_slice(row_pixels);
    }
    RgbaF32Image::new(
        RgbaF32Descriptor::new(dimensions, input.descriptor().color_encoding()),
        pixels,
    )
    .map_err(|error| BasicPointError::Readback(error.to_string()))
}

fn place_tile(
    assembled: &mut [Option<RgbaF32Pixel>],
    input: &RgbaF32Image,
    tile: crate::CpuPixelpipeTile,
    output: &RgbaF32Image,
) -> Result<(), BasicPointError> {
    let source_width = input.descriptor().dimensions().width();
    let dimensions = tile.dimensions();
    for y in 0..dimensions.height() {
        let destination_row = u64::from(tile.origin_y() + y)
            .checked_mul(u64::from(source_width))
            .and_then(|offset| offset.checked_add(u64::from(tile.origin_x())))
            .ok_or_else(|| {
                BasicPointError::Readback("tile destination index overflow".to_owned())
            })?;
        let destination = usize::try_from(destination_row).map_err(|_| {
            BasicPointError::Readback("tile destination index is too large".to_owned())
        })?;
        let source = usize::try_from(u64::from(y) * u64::from(dimensions.width()))
            .map_err(|_| BasicPointError::Readback("tile output index is too large".to_owned()))?;
        for x in 0..dimensions.width() as usize {
            let destination_index = destination.checked_add(x).ok_or_else(|| {
                BasicPointError::Readback("tile destination row overflow".to_owned())
            })?;
            if assembled.get(destination_index).is_none()
                || output.pixels().get(source + x).is_none()
            {
                return Err(BasicPointError::Readback(
                    "tile output is out of bounds".to_owned(),
                ));
            }
            if assembled[destination_index].is_some() {
                return Err(BasicPointError::Readback("tiled output overlap".to_owned()));
            }
            assembled[destination_index] = Some(output.pixels()[source + x]);
        }
    }
    Ok(())
}
