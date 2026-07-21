use super::{
    CoverageError, EdgeOverlap, ResidencyPlan, ResourceAllocationEstimate, TileAlignment, TileArea,
    TileMemoryBudget, TileMemoryEstimate, TileResourceSpec, TilingError,
};
use crate::{DeviceGeneration, DispatchRegion, ResourceClass, ResourceKind, ResourceRequest, Tile};
use rusttable_image::{ImageDimensions, Roi};
use sha2::{Digest, Sha256};

const DEFAULT_MAX_CANDIDATES: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTileRequest {
    pub generation: DeviceGeneration,
    pub dimensions: ImageDimensions,
    pub output_roi: Roi,
    pub preferred: [u32; 2],
    pub minimum: [u32; 2],
    pub maximum: [u32; 2],
    pub overlap: EdgeOverlap,
    pub alignment: TileAlignment,
    pub budget: TileMemoryBudget,
    pub fixed_bytes: u64,
    pub resources: Vec<TileResourceSpec>,
    pub residency: ResidencyPlan,
    pub max_candidates: usize,
}

impl GpuTileRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        generation: DeviceGeneration,
        dimensions: ImageDimensions,
        output_roi: Roi,
        preferred: [u32; 2],
        minimum: [u32; 2],
        maximum: [u32; 2],
        overlap: EdgeOverlap,
        alignment: TileAlignment,
        budget: TileMemoryBudget,
        resources: Vec<TileResourceSpec>,
    ) -> Result<Self, TilingError> {
        output_roi
            .within(dimensions)
            .map_err(|_| TilingError::RoiOutOfBounds)?;
        if output_roi.is_empty() {
            return Err(TilingError::EmptyRoi);
        }
        if preferred.contains(&0) || minimum.contains(&0) || maximum.contains(&0) {
            return Err(TilingError::ZeroTileDimension);
        }
        if minimum[0] > maximum[0]
            || minimum[1] > maximum[1]
            || preferred[0] < minimum[0]
            || preferred[1] < minimum[1]
            || minimum[0] > output_roi.width()
            || minimum[1] > output_roi.height()
        {
            return Err(TilingError::InvalidTileBounds);
        }
        if resources.iter().any(|resource| {
            resource.class.generation != generation
                || resource.class.alignment == 0
                || !resource.class.alignment.is_power_of_two()
        }) {
            return Err(
                if resources
                    .iter()
                    .any(|resource| resource.class.generation != generation)
                {
                    TilingError::GenerationMismatch
                } else {
                    TilingError::InvalidResourceAlignment
                },
            );
        }
        let residency = ResidencyPlan::new(generation, budget.hard_bytes());
        Ok(Self {
            generation,
            dimensions,
            output_roi,
            preferred,
            minimum,
            maximum,
            overlap,
            alignment,
            budget,
            fixed_bytes: 0,
            resources,
            residency,
            max_candidates: DEFAULT_MAX_CANDIDATES,
        })
    }

    #[must_use]
    pub const fn with_fixed_bytes(mut self, fixed_bytes: u64) -> Self {
        self.fixed_bytes = fixed_bytes;
        self
    }

    #[must_use]
    pub const fn with_max_candidates(mut self, max_candidates: usize) -> Self {
        self.max_candidates = if max_candidates < DEFAULT_MAX_CANDIDATES {
            max_candidates
        } else {
            DEFAULT_MAX_CANDIDATES
        };
        self
    }

    pub fn with_residency(mut self, residency: ResidencyPlan) -> Result<Self, TilingError> {
        if residency.generation() != self.generation {
            return Err(TilingError::GenerationMismatch);
        }
        residency.validate().map_err(TilingError::Residency)?;
        self.residency = residency;
        Ok(self)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlannedGpuTile {
    pub output: Tile,
    pub input: Tile,
    pub input_roi: Roi,
    pub dispatch_region: DispatchRegion,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageModel {
    roi: Roi,
    tiles: Vec<Tile>,
}

impl CoverageModel {
    #[must_use]
    pub const fn new(roi: Roi) -> Self {
        Self {
            roi,
            tiles: Vec::new(),
        }
    }

    pub fn add(&mut self, tile: Tile) -> Result<(), CoverageError> {
        let rect = roi_from_tile(tile).map_err(|_| CoverageError::ArithmeticOverflow)?;
        if self.roi.intersection(rect) != Some(rect) {
            return Err(CoverageError::OutsideRoi);
        }
        if self.tiles.iter().any(|existing| {
            roi_from_tile(*existing)
                .ok()
                .and_then(|value| value.intersection(rect))
                .is_some_and(|value| !value.is_empty())
        }) {
            return Err(CoverageError::Overlap);
        }
        self.tiles.push(tile);
        Ok(())
    }
    pub fn validate(&self) -> Result<(), CoverageError> {
        let covered = self.tiles.iter().try_fold(0_u64, |sum, tile| {
            let area = u64::from(tile.width)
                .checked_mul(u64::from(tile.height))
                .ok_or(CoverageError::ArithmeticOverflow)?;
            sum.checked_add(area)
                .ok_or(CoverageError::ArithmeticOverflow)
        })?;
        let expected = u64::from(self.roi.width())
            .checked_mul(u64::from(self.roi.height()))
            .ok_or(CoverageError::ArithmeticOverflow)?;
        if covered != expected {
            return Err(CoverageError::Gap);
        }
        Ok(())
    }

    #[must_use]
    pub const fn roi(&self) -> Roi {
        self.roi
    }

    #[must_use]
    pub fn tiles(&self) -> &[Tile] {
        &self.tiles
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTileCandidate {
    pub width: u32,
    pub height: u32,
    pub full_frame: bool,
    pub tiles: Vec<PlannedGpuTile>,
    pub coverage: CoverageModel,
    pub memory: TileMemoryEstimate,
    pub resource_requests: Vec<ResourceRequest>,
    pub identity: [u8; 32],
}

impl GpuTileCandidate {
    #[must_use]
    pub const fn tile_count(&self) -> usize {
        self.tiles.len()
    }
    pub fn validate_coverage(&self) -> Result<(), CoverageError> {
        self.coverage.validate()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuTilingPlan {
    pub generation: DeviceGeneration,
    pub candidates: Vec<GpuTileCandidate>,
    pub receipt: TilingReceipt,
}

impl GpuTilingPlan {
    #[must_use]
    pub fn candidates(&self) -> &[GpuTileCandidate] {
        &self.candidates
    }

    #[must_use]
    pub fn candidate(&self, index: usize) -> Option<&GpuTileCandidate> {
        self.candidates.get(index)
    }

    #[must_use]
    pub const fn retry_count(&self) -> usize {
        self.candidates.len().saturating_sub(1)
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilingReceipt {
    pub identity: [u8; 32],
    pub candidate_count: usize,
    pub full_frame_fits: bool,
    pub resident_bytes: u64,
    pub max_retries: usize,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct GpuTilePlanner;

impl GpuTilePlanner {
    pub fn plan(request: &GpuTileRequest) -> Result<GpuTilingPlan, TilingError> {
        if request.max_candidates == 0 {
            return Err(TilingError::ZeroCandidateLimit);
        }
        request
            .residency
            .validate()
            .map_err(TilingError::Residency)?;
        if request.residency.generation() != request.generation {
            return Err(TilingError::GenerationMismatch);
        }
        let full_width = request.output_roi.width();
        let full_height = request.output_roi.height();
        let mut dimensions = Vec::new();
        let full_estimate = estimate(request, full_width, full_height)?;
        if full_estimate.required_bytes <= full_estimate.available_bytes
            && full_estimate
                .allocations
                .iter()
                .all(|allocation| allocation.bytes <= request.budget.max_allocation_bytes())
        {
            dimensions.push((full_width, full_height, true));
        }

        let mut width = align_down(
            request.preferred[0].min(request.maximum[0]).min(full_width),
            request.alignment.extent_x(),
        );
        let mut height = align_down(
            request.preferred[1]
                .min(request.maximum[1])
                .min(full_height),
            request.alignment.extent_y(),
        );
        width = width.max(request.minimum[0]);
        height = height.max(request.minimum[1]);
        while dimensions.len() < request.max_candidates {
            if width == 0
                || height == 0
                || dimensions
                    .iter()
                    .any(|value| value.0 == width && value.1 == height)
            {
                break;
            }
            let memory = estimate(request, width, height)?;
            let fits = memory.required_bytes <= memory.available_bytes
                && memory
                    .allocations
                    .iter()
                    .all(|allocation| allocation.bytes <= request.budget.max_allocation_bytes());
            if fits {
                dimensions.push((width, height, false));
            }
            let next = smaller_dimensions(width, height, request.minimum, request.alignment);
            if next == (width, height) {
                break;
            }
            (width, height) = next;
        }

        if dimensions.is_empty() {
            return Err(TilingError::NoLegalCandidate {
                required: full_estimate.required_bytes,
                available: full_estimate.available_bytes,
            });
        }
        let mut candidates = Vec::with_capacity(dimensions.len());
        for (width, height, full_frame) in dimensions {
            candidates.push(build_candidate(request, width, height, full_frame)?);
        }
        let identity = plan_identity(request, &candidates);
        Ok(GpuTilingPlan {
            generation: request.generation,
            receipt: TilingReceipt {
                identity,
                candidate_count: candidates.len(),
                full_frame_fits: candidates.first().is_some_and(|value| value.full_frame),
                resident_bytes: request.residency.resident_bytes(),
                max_retries: candidates.len().saturating_sub(1).min(2),
            },
            candidates,
        })
    }
}
pub type TileCandidate = GpuTileCandidate;
pub type TilePlan = GpuTilingPlan;
pub type TileRequest = GpuTileRequest;

fn build_candidate(
    request: &GpuTileRequest,
    width: u32,
    height: u32,
    full_frame: bool,
) -> Result<GpuTileCandidate, TilingError> {
    let memory = estimate(request, width, height)?;
    if memory.required_bytes > memory.available_bytes {
        return Err(TilingError::NoLegalCandidate {
            required: memory.required_bytes,
            available: memory.available_bytes,
        });
    }
    let mut coverage = CoverageModel::new(request.output_roi);
    let mut tiles = Vec::new();
    let mut y = request.output_roi.y();
    while y < request.output_roi.bottom() {
        let tile_height = height.min(request.output_roi.bottom() - y);
        let mut x = request.output_roi.x();
        while x < request.output_roi.right() {
            let tile_width = width.min(request.output_roi.right() - x);
            let output = Tile::new(x, y, tile_width, tile_height)
                .map_err(|_| TilingError::ArithmeticOverflow)?;
            coverage.add(output).map_err(TilingError::Coverage)?;
            let output_roi = roi_from_tile(output).map_err(|_| TilingError::ArithmeticOverflow)?;
            let input_roi = expanded_roi(output_roi, request.overlap, request.dimensions)?;
            let input = Tile::new(
                input_roi.x(),
                input_roi.y(),
                input_roi.width(),
                input_roi.height(),
            )
            .map_err(|_| TilingError::ArithmeticOverflow)?;
            let dispatch_region = DispatchRegion::new(
                request.dimensions.width(),
                request.dimensions.height(),
                output_roi,
                input,
            )
            .map_err(TilingError::Dispatch)?;
            tiles.push(PlannedGpuTile {
                output,
                input,
                input_roi,
                dispatch_region,
            });
            x = x
                .checked_add(tile_width)
                .ok_or(TilingError::ArithmeticOverflow)?;
        }
        y = y
            .checked_add(tile_height)
            .ok_or(TilingError::ArithmeticOverflow)?;
    }
    coverage.validate().map_err(TilingError::Coverage)?;
    let resource_requests = memory
        .allocations
        .iter()
        .zip(request.resources.iter())
        .map(|(allocation, spec)| ResourceRequest::new(allocation.class, spec.initialization))
        .collect();
    let identity = candidate_identity(request, width, height, &memory, &tiles);
    Ok(GpuTileCandidate {
        width,
        height,
        full_frame,
        tiles,
        coverage,
        memory,
        resource_requests,
        identity,
    })
}

fn estimate(
    request: &GpuTileRequest,
    width: u32,
    height: u32,
) -> Result<TileMemoryEstimate, TilingError> {
    let output = Tile::new(
        request.output_roi.x(),
        request.output_roi.y(),
        width,
        height,
    )
    .map_err(|_| TilingError::ArithmeticOverflow)?;
    let output_roi = roi_from_tile(output).map_err(|_| TilingError::ArithmeticOverflow)?;
    let input = expanded_roi(output_roi, request.overlap, request.dimensions)?;
    let output_area = area(output_roi)?;
    let input_area = area(input)?;
    let larger_area = output_area.max(input_area);
    let mut allocations = Vec::with_capacity(request.resources.len());
    let mut tile_bytes = request.fixed_bytes;
    for spec in &request.resources {
        let pixels = match spec.area {
            TileArea::Input => input_area,
            TileArea::Output => output_area,
            TileArea::Larger => larger_area,
        };
        let bytes = pixels
            .checked_mul(spec.bytes_per_pixel)
            .and_then(|value| value.checked_add(spec.fixed_bytes))
            .and_then(|value| value.checked_add(request.budget.backend_overhead()))
            .ok_or(TilingError::ArithmeticOverflow)?;
        tile_bytes = tile_bytes
            .checked_add(bytes)
            .ok_or(TilingError::ArithmeticOverflow)?;
        allocations.push(ResourceAllocationEstimate {
            name: spec.name.clone(),
            bytes,
            class: materialize_class(spec, request.generation, width, height, bytes),
            resident: spec.resident,
        });
    }
    let resident_bytes = request.residency.resident_bytes();
    let required_bytes = resident_bytes
        .checked_add(request.budget.reserve_bytes())
        .and_then(|value| value.checked_add(tile_bytes))
        .ok_or(TilingError::ArithmeticOverflow)?;
    Ok(TileMemoryEstimate {
        tile_bytes,
        resident_bytes,
        reserve_bytes: request.budget.reserve_bytes(),
        required_bytes,
        available_bytes: request.budget.hard_bytes(),
        allocations,
    })
}

fn materialize_class(
    spec: &TileResourceSpec,
    generation: DeviceGeneration,
    width: u32,
    height: u32,
    bytes: u64,
) -> ResourceClass {
    match spec.class.kind {
        ResourceKind::Buffer => ResourceClass::buffer(generation, bytes, spec.class.usage)
            .with_alignment(spec.class.alignment)
            .with_mapping(spec.class.mapped)
            .with_compatibility(spec.class.compatibility),
        ResourceKind::Texture => ResourceClass::texture(
            generation,
            [width, height, 1],
            spec.class.format,
            spec.class.usage,
            spec.class.mip_level_count,
            spec.class.sample_count,
        )
        .with_size(bytes)
        .with_alignment(spec.class.alignment)
        .with_compatibility(spec.class.compatibility),
        _ => spec.class,
    }
}

fn expanded_roi(
    output: Roi,
    overlap: EdgeOverlap,
    dimensions: ImageDimensions,
) -> Result<Roi, TilingError> {
    let x = output.x().saturating_sub(overlap.left());
    let y = output.y().saturating_sub(overlap.top());
    let right = output
        .right()
        .checked_add(overlap.right())
        .ok_or(TilingError::ArithmeticOverflow)?
        .min(dimensions.width());
    let bottom = output
        .bottom()
        .checked_add(overlap.bottom())
        .ok_or(TilingError::ArithmeticOverflow)?
        .min(dimensions.height());
    Roi::new(x, y, right - x, bottom - y).map_err(|_| TilingError::ArithmeticOverflow)
}

fn smaller_dimensions(
    width: u32,
    height: u32,
    minimum: [u32; 2],
    alignment: TileAlignment,
) -> (u32, u32) {
    let reduce_width = width >= height;
    if reduce_width && width > minimum[0] {
        (reduce(width, minimum[0], alignment.extent_x()), height)
    } else if height > minimum[1] {
        (width, reduce(height, minimum[1], alignment.extent_y()))
    } else if width > minimum[0] {
        (reduce(width, minimum[0], alignment.extent_x()), height)
    } else {
        (width, height)
    }
}

fn reduce(value: u32, minimum: u32, alignment: u32) -> u32 {
    let target = value.saturating_add(1) / 2;
    align_down(target.max(minimum), alignment).max(minimum)
}

fn align_down(value: u32, alignment: u32) -> u32 {
    value.checked_div(alignment).unwrap_or_default() * alignment
}

fn roi_from_tile(tile: Tile) -> Result<Roi, TilingError> {
    Roi::new(tile.x, tile.y, tile.width, tile.height).map_err(|_| TilingError::ArithmeticOverflow)
}

fn area(roi: Roi) -> Result<u64, TilingError> {
    u64::from(roi.width())
        .checked_mul(u64::from(roi.height()))
        .ok_or(TilingError::ArithmeticOverflow)
}

fn candidate_identity(
    request: &GpuTileRequest,
    width: u32,
    height: u32,
    memory: &TileMemoryEstimate,
    tiles: &[PlannedGpuTile],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.gpu.tiling.candidate.v1");
    hasher.update(request.generation.value().to_le_bytes());
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    hasher.update(memory.required_bytes.to_le_bytes());
    for tile in tiles {
        hasher.update(tile.output.x.to_le_bytes());
        hasher.update(tile.output.y.to_le_bytes());
        hasher.update(tile.output.width.to_le_bytes());
        hasher.update(tile.output.height.to_le_bytes());
        hasher.update(tile.input_roi.x().to_le_bytes());
        hasher.update(tile.input_roi.y().to_le_bytes());
        hasher.update(tile.input_roi.width().to_le_bytes());
        hasher.update(tile.input_roi.height().to_le_bytes());
    }
    hasher.finalize().into()
}

fn plan_identity(request: &GpuTileRequest, candidates: &[GpuTileCandidate]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"rusttable.gpu.tiling.plan.v1");
    hasher.update(request.generation.value().to_le_bytes());
    hasher.update(request.output_roi.x().to_le_bytes());
    hasher.update(request.output_roi.y().to_le_bytes());
    hasher.update(request.output_roi.width().to_le_bytes());
    hasher.update(request.output_roi.height().to_le_bytes());
    for candidate in candidates {
        hasher.update(candidate.identity);
    }
    hasher.finalize().into()
}
