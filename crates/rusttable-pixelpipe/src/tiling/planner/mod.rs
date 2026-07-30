use std::fmt;

use super::models::{
    EstimateComponent, EstimateFailure, MemoryEstimate, PlannedTileGrid, TilePlan,
    TilePlanIdentity, TilePlanReceipt,
};
use super::requirements::{
    AreaSource, BackendRequirement, EstimateError, NodeRequirements, ResourceKind, TileBackend,
};
use crate::{GeometryError, TileAlignment, TileRect};
use sha2::{Digest, Sha256};

mod helpers;
use helpers::{
    align_and_clip, aligned_initial, can_reduce, combine_alignment, expand_and_clip, make_grid,
    next_dimension,
};

const CPU_RESERVE_MILLI: u64 = 50;
const GPU_RESERVE_MILLI: u64 = 100;
const CPU_RESERVE_FLOOR: u64 = 16 * 1024 * 1024;
const GPU_RESERVE_FLOOR: u64 = 32 * 1024 * 1024;
const MAX_RECEIPT_COMPONENTS: usize = 512;

/// Immutable hard memory policy for one backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MemoryBudget {
    backend: TileBackend,
    hard_limit: u64,
    max_allocation: u64,
    additional_reserve: u64,
}

impl MemoryBudget {
    /// Creates a backend budget with the issue-defined safety reserve policy.
    ///
    /// # Errors
    ///
    /// Returns an error when either hard limit is zero.
    pub const fn new(
        backend: TileBackend,
        hard_limit: u64,
        max_allocation: u64,
    ) -> Result<Self, TilePlanError> {
        if hard_limit == 0 || max_allocation == 0 {
            return Err(TilePlanError::InvalidBudget);
        }
        Ok(Self {
            backend,
            hard_limit,
            max_allocation,
            additional_reserve: 0,
        })
    }

    /// Raises the immutable reserve for a device-specific policy.
    #[must_use]
    pub const fn with_additional_reserve(mut self, bytes: u64) -> Self {
        self.additional_reserve = bytes;
        self
    }

    #[must_use]
    pub const fn backend(self) -> TileBackend {
        self.backend
    }
    #[must_use]
    pub const fn hard_limit(self) -> u64 {
        self.hard_limit
    }
    #[must_use]
    pub const fn max_allocation(self) -> u64 {
        self.max_allocation
    }

    /// Computes the backend reserve using checked arithmetic.
    ///
    /// # Errors
    ///
    /// Returns an error if reserve arithmetic overflows.
    pub fn reserve(self) -> Result<u64, TilePlanError> {
        // The reserve is deliberately checked because policy inputs are immutable but not trusted.
        let (percent, floor) = match self.backend {
            TileBackend::Cpu => (CPU_RESERVE_MILLI, CPU_RESERVE_FLOOR),
            TileBackend::Gpu => (GPU_RESERVE_MILLI, GPU_RESERVE_FLOOR),
        };
        self.hard_limit
            .checked_mul(percent)
            .and_then(|value| value.checked_add(999))
            .map(|value| value / 1000)
            .ok_or(TilePlanError::ArithmeticOverflow)?
            .max(floor)
            .checked_add(self.additional_reserve)
            .ok_or(TilePlanError::ArithmeticOverflow)
    }

    fn available(self) -> Result<(u64, u64), TilePlanError> {
        let reserve = self.reserve()?;
        if reserve >= self.hard_limit {
            return Err(TilePlanError::BudgetBelowReserve {
                hard_limit: self.hard_limit,
                reserve,
            });
        }
        Ok((self.hard_limit - reserve, reserve))
    }
}

/// Maximum dimensions supplied by an immutable device capability snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileDeviceLimits {
    max_width: u32,
    max_height: u32,
}

impl TileDeviceLimits {
    /// Creates checked device tile dimensions.
    ///
    /// # Errors
    ///
    /// Returns an error when either device dimension is zero.
    pub const fn new(max_width: u32, max_height: u32) -> Result<Self, TilePlanError> {
        if max_width == 0 || max_height == 0 {
            return Err(TilePlanError::InvalidDeviceLimits);
        }
        Ok(Self {
            max_width,
            max_height,
        })
    }
    #[must_use]
    pub const fn max_width(self) -> u32 {
        self.max_width
    }
    #[must_use]
    pub const fn max_height(self) -> u32 {
        self.max_height
    }
}

/// Immutable request to construct one exact-cover tile plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TilePlanRequest {
    output: TileRect,
    device: TileDeviceLimits,
    budget: MemoryBudget,
    allow_empty: bool,
}

impl TilePlanRequest {
    /// Creates a nonempty tile request.
    #[must_use]
    pub const fn new(output: TileRect, device: TileDeviceLimits, budget: MemoryBudget) -> Self {
        Self {
            output,
            device,
            budget,
            allow_empty: false,
        }
    }
    /// Allows #267's explicit empty-output policy to produce a no-allocation plan.
    #[must_use]
    pub const fn allow_empty(mut self) -> Self {
        self.allow_empty = true;
        self
    }
    #[must_use]
    pub const fn output(self) -> TileRect {
        self.output
    }
    #[must_use]
    pub const fn device(self) -> TileDeviceLimits {
        self.device
    }
    #[must_use]
    pub const fn budget(self) -> MemoryBudget {
        self.budget
    }
}

/// Deterministic pure planner for requirement estimates and exact-cover grids.
#[derive(Debug, Clone, Copy, Default)]
pub struct TilePlanner;

impl TilePlanner {
    /// Plans one backend without allocating product buffers or querying a device.
    ///
    /// # Errors
    ///
    /// Returns a typed failure when requirements, geometry, arithmetic, or the independent budget cannot support a plan.
    #[allow(clippy::trivially_copy_pass_by_ref)]
    pub fn plan(
        &self,
        requirements: &NodeRequirements,
        request: TilePlanRequest,
    ) -> Result<TilePlan, TilePlanError> {
        let backend = request.budget().backend();
        verify_backend(requirements, backend)?;
        let output = request.output();
        if output.is_empty() {
            if !request.allow_empty {
                return Err(TilePlanError::EmptyOutputNotAllowed);
            }
            let estimate = empty_estimate(request.budget())?;
            let grid = PlannedTileGrid {
                output,
                columns: 0,
                rows: 0,
                tiles: Vec::new(),
            };
            let receipt = make_receipt(requirements, backend, 0, 0, 0, estimate.clone());
            return Ok(TilePlan {
                backend,
                full_frame: false,
                grid,
                estimate,
                receipt,
            });
        }
        if !requirements.roi_chain().output_bounds().contains(output) {
            return Err(TilePlanError::OutputOutsideRoi);
        }
        let policy = aggregate_policy(requirements, backend, output, request.device())?;
        let full_frame = requirements.requires_full_frame(backend);
        let (width, height, estimate, steps) = if full_frame {
            let estimate = Self::estimate(requirements, backend, output, output, request.budget())?;
            (output.width(), output.height(), estimate, 0)
        } else {
            Self::search(requirements, request, policy)?
        };
        let grid = make_grid(requirements, output, width, height, backend)?;
        let receipt = make_receipt(
            requirements,
            backend,
            width,
            height,
            steps,
            estimate.clone(),
        );
        Ok(TilePlan {
            backend,
            full_frame,
            grid,
            estimate,
            receipt,
        })
    }

    fn search(
        requirements: &NodeRequirements,
        request: TilePlanRequest,
        policy: AggregatePolicy,
    ) -> Result<(u32, u32, MemoryEstimate, u32), TilePlanError> {
        let mut width = policy.initial_width;
        let mut height = policy.initial_height;
        let mut steps = 0;
        loop {
            let candidate =
                TileRect::new(request.output().x(), request.output().y(), width, height);
            match Self::estimate(
                requirements,
                request.budget().backend(),
                candidate,
                request.output(),
                request.budget(),
            ) {
                Ok(estimate) => return Ok((width, height, estimate, steps)),
                Err(error) if can_reduce(&error) => {
                    steps = steps.saturating_add(1);
                    if width >= height && width > policy.minimum_width {
                        width = next_dimension(
                            width,
                            policy.minimum_width,
                            policy.alignment.extent_x(),
                        );
                    } else if height > policy.minimum_height {
                        height = next_dimension(
                            height,
                            policy.minimum_height,
                            policy.alignment.extent_y(),
                        );
                    } else if width > policy.minimum_width {
                        width = next_dimension(
                            width,
                            policy.minimum_width,
                            policy.alignment.extent_x(),
                        );
                    } else {
                        return Err(TilePlanError::MinimumTileDoesNotFit {
                            estimate: Box::new(error),
                        });
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    // This keeps bounded component accounting together so every live resource
    // follows the same checked-budget path.
    #[allow(clippy::too_many_lines)]
    fn estimate(
        requirements: &NodeRequirements,
        backend: TileBackend,
        candidate: TileRect,
        request_output: TileRect,
        budget: MemoryBudget,
    ) -> Result<MemoryEstimate, TilePlanError> {
        let (available, reserve) = budget.available()?;
        let roi_inputs = requirements
            .roi_chain()
            .required_inputs(candidate)
            .map_err(TilePlanError::Geometry)?;
        let mut components = Vec::new();
        let mut total = 0_u64;
        for (offset, (node_index, node)) in requirements.enabled_nodes().enumerate() {
            let record = node
                .backend(backend)
                .ok_or(TilePlanError::BackendUnavailable { backend })?;
            let stage = requirements
                .roi_chain()
                .stages()
                .get(offset)
                .ok_or(TilePlanError::RoiNodeMismatch)?;
            let roi = roi_inputs
                .get(offset)
                .ok_or(TilePlanError::RoiNodeMismatch)?
                .1;
            let input = if record.full_frame().full_input() || record.full_frame().full_pipeline() {
                stage.input_bounds()
            } else {
                align_and_clip(
                    expand_and_clip(roi, record.overlap(), stage.input_bounds())?,
                    stage.input_bounds(),
                    record.alignment(),
                )?
            };
            let output = candidate
                .intersect(request_output)
                .ok_or(TilePlanError::OutputOutsideRoi)?;
            let larger = if input.area().unwrap_or(0) >= output.area().unwrap_or(0) {
                input
            } else {
                output
            };
            for requirement in [record.input(), record.output(), record.temporary()] {
                let area = choose_area(requirement.area(), input, output, larger);
                let bytes = resource_bytes(requirement, area).map_err(|reason| {
                    map_estimate_error(reason, node_index, node, requirement.kind(), candidate)
                })?;
                add_component(&mut components, node_index, node, requirement.kind(), bytes)?;
                total = total
                    .checked_add(bytes)
                    .ok_or(TilePlanError::ArithmeticOverflow)?;
                check_allocation(
                    record,
                    budget,
                    bytes,
                    node_index,
                    node,
                    candidate,
                    requirement.kind(),
                )?;
            }
            for requirement in record.auxiliary() {
                let area = choose_area(requirement.area(), input, output, larger);
                let bytes = resource_bytes(*requirement, area).map_err(|reason| {
                    map_estimate_error(reason, node_index, node, requirement.kind(), candidate)
                })?;
                add_component(&mut components, node_index, node, requirement.kind(), bytes)?;
                total = total
                    .checked_add(bytes)
                    .ok_or(TilePlanError::ArithmeticOverflow)?;
                check_allocation(
                    record,
                    budget,
                    bytes,
                    node_index,
                    node,
                    candidate,
                    requirement.kind(),
                )?;
            }
            if record.fixed_bytes() != 0 {
                let bytes = record.fixed_bytes();
                add_component(
                    &mut components,
                    node_index,
                    node,
                    ResourceKind::Fixed,
                    bytes,
                )?;
                total = total
                    .checked_add(bytes)
                    .ok_or(TilePlanError::ArithmeticOverflow)?;
                check_allocation(
                    record,
                    budget,
                    bytes,
                    node_index,
                    node,
                    candidate,
                    ResourceKind::Fixed,
                )?;
            }
        }
        let required = total
            .checked_add(reserve)
            .ok_or(TilePlanError::ArithmeticOverflow)?;
        if total > available {
            let dominant = components
                .iter()
                .max_by_key(|component| component.bytes)
                .map(|component| EstimateFailure {
                    node_index: component.node_index,
                    operation_id: component.operation_id,
                    operation_name: component.operation_name.clone(),
                    resource: component.resource,
                    reason: EstimateError::ArithmeticOverflow,
                    candidate,
                });
            return Err(TilePlanError::OverBudget {
                required,
                available: budget.hard_limit(),
                dominant,
            });
        }
        let dominant = components
            .iter()
            .max_by_key(|component| component.bytes)
            .map(|component| crate::DominantResource {
                node_index: component.node_index,
                operation_id: component.operation_id,
                operation_name: component.operation_name.clone(),
                resource: component.resource,
                bytes: component.bytes,
            });
        Ok(MemoryEstimate {
            backend,
            total_bytes: total,
            reserve_bytes: reserve,
            required_bytes: required,
            available_bytes: available,
            components,
            dominant,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct AggregatePolicy {
    minimum_width: u32,
    minimum_height: u32,
    initial_width: u32,
    initial_height: u32,
    alignment: TileAlignment,
}

fn aggregate_policy(
    requirements: &NodeRequirements,
    backend: TileBackend,
    output: TileRect,
    device: TileDeviceLimits,
) -> Result<AggregatePolicy, TilePlanError> {
    let mut min_w = 1;
    let mut min_h = 1;
    let mut pref_w = u32::MAX;
    let mut pref_h = u32::MAX;
    let mut max_w = device.max_width().min(output.width());
    let mut max_h = device.max_height().min(output.height());
    let mut alignment = TileAlignment::new(1, 1, 1, 1).map_err(TilePlanError::Geometry)?;
    for (_, node) in requirements.enabled_nodes() {
        let record = node
            .backend(backend)
            .ok_or(TilePlanError::BackendUnavailable { backend })?;
        let dimensions = record.dimensions();
        min_w = min_w.max(dimensions.minimum_width());
        min_h = min_h.max(dimensions.minimum_height());
        pref_w = pref_w.min(dimensions.preferred_width());
        pref_h = pref_h.min(dimensions.preferred_height());
        max_w = max_w.min(dimensions.maximum_width());
        max_h = max_h.min(dimensions.maximum_height());
        alignment = combine_alignment(alignment, record.alignment())?;
    }
    min_w = min_w.min(output.width());
    min_h = min_h.min(output.height());
    if min_w == 0 || min_h == 0 || max_w < min_w || max_h < min_h {
        return Err(TilePlanError::ConflictingTileDimensions);
    }
    Ok(AggregatePolicy {
        minimum_width: min_w,
        minimum_height: min_h,
        initial_width: aligned_initial(pref_w.min(max_w), min_w, alignment.extent_x()),
        initial_height: aligned_initial(pref_h.min(max_h), min_h, alignment.extent_y()),
        alignment,
    })
}

fn verify_backend(
    requirements: &NodeRequirements,
    backend: TileBackend,
) -> Result<(), TilePlanError> {
    if requirements
        .enabled_nodes()
        .all(|(_, node)| node.backend(backend).is_some())
    {
        Ok(())
    } else {
        Err(TilePlanError::BackendUnavailable { backend })
    }
}

fn resource_bytes(
    requirement: crate::BufferRequirement,
    area: TileRect,
) -> Result<u64, EstimateError> {
    let base = area
        .area()
        .ok_or(EstimateError::ArithmeticOverflow)?
        .checked_mul(requirement.bytes_per_pixel().get())
        .ok_or(EstimateError::ArithmeticOverflow)?;
    requirement.factor().checked_bytes(base)
}

fn choose_area(area: AreaSource, input: TileRect, output: TileRect, larger: TileRect) -> TileRect {
    match area {
        AreaSource::Input => input,
        AreaSource::Output => output,
        AreaSource::Larger => larger,
    }
}

fn add_component(
    components: &mut Vec<EstimateComponent>,
    node_index: usize,
    node: &crate::NodeRequirement,
    resource: ResourceKind,
    bytes: u64,
) -> Result<(), TilePlanError> {
    if components.len() >= MAX_RECEIPT_COMPONENTS {
        return Err(TilePlanError::ReceiptLimit);
    }
    components.push(EstimateComponent {
        node_index,
        operation_id: node.operation_id(),
        operation_name: node.operation_name().to_owned(),
        resource,
        bytes,
    });
    Ok(())
}

fn check_allocation(
    record: &BackendRequirement,
    budget: MemoryBudget,
    bytes: u64,
    node_index: usize,
    node: &crate::NodeRequirement,
    candidate: TileRect,
    resource: ResourceKind,
) -> Result<(), TilePlanError> {
    let limit = record
        .max_allocation()
        .unwrap_or(budget.max_allocation())
        .min(budget.max_allocation());
    if bytes > limit {
        return Err(TilePlanError::MaxAllocation {
            node_index,
            operation_id: node.operation_id(),
            operation_name: node.operation_name().to_owned(),
            resource,
            bytes,
            limit,
            candidate,
        });
    }
    Ok(())
}

fn empty_estimate(budget: MemoryBudget) -> Result<MemoryEstimate, TilePlanError> {
    let (available, reserve) = budget.available()?;
    Ok(MemoryEstimate {
        backend: budget.backend(),
        total_bytes: 0,
        reserve_bytes: reserve,
        required_bytes: reserve,
        available_bytes: available,
        components: Vec::new(),
        dominant: None,
    })
}

fn make_receipt(
    requirements: &NodeRequirements,
    backend: TileBackend,
    width: u32,
    height: u32,
    search_steps: u32,
    estimate: MemoryEstimate,
) -> TilePlanReceipt {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"rusttable.pixelpipe.tiling.v1");
    bytes.extend_from_slice(backend.tag().as_bytes());
    bytes.extend_from_slice(&(requirements.enabled_range().start as u64).to_le_bytes());
    bytes.extend_from_slice(&(requirements.enabled_range().end as u64).to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&estimate.total_bytes().to_le_bytes());
    bytes.extend_from_slice(&estimate.reserve_bytes().to_le_bytes());
    for component in estimate.components() {
        bytes.extend_from_slice(&(component.node_index() as u64).to_le_bytes());
        bytes.extend_from_slice(&component.operation_id().to_le_bytes());
        bytes.extend_from_slice(component.resource().tag().as_bytes());
        bytes.extend_from_slice(&component.bytes().to_le_bytes());
    }
    TilePlanReceipt {
        identity: TilePlanIdentity(Sha256::digest(bytes).into()),
        backend,
        enabled_range: requirements.enabled_range().clone(),
        candidate_width: width,
        candidate_height: height,
        search_steps,
        estimate,
    }
}

/// Typed planner failures with bounded remediation evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TilePlanError {
    InvalidBudget,
    InvalidDeviceLimits,
    BackendUnavailable {
        backend: TileBackend,
    },
    BudgetBelowReserve {
        hard_limit: u64,
        reserve: u64,
    },
    EmptyOutputNotAllowed,
    OutputOutsideRoi,
    ConflictingTileDimensions,
    MinimumTileDoesNotFit {
        estimate: Box<TilePlanError>,
    },
    UnknownRequiredResource {
        node_index: usize,
        operation_id: u128,
        operation_name: String,
        resource: ResourceKind,
        candidate: TileRect,
    },
    UnboundedRequiredResource {
        node_index: usize,
        operation_id: u128,
        operation_name: String,
        resource: ResourceKind,
        candidate: TileRect,
    },
    Estimate {
        node_index: usize,
        operation_id: u128,
        operation_name: String,
        resource: ResourceKind,
        reason: EstimateError,
        candidate: TileRect,
    },
    MaxAllocation {
        node_index: usize,
        operation_id: u128,
        operation_name: String,
        resource: ResourceKind,
        bytes: u64,
        limit: u64,
        candidate: TileRect,
    },
    OverBudget {
        required: u64,
        available: u64,
        dominant: Option<EstimateFailure>,
    },
    RoiNodeMismatch,
    Geometry(GeometryError),
    ArithmeticOverflow,
    ReceiptLimit,
}

fn map_estimate_error(
    reason: EstimateError,
    node_index: usize,
    node: &crate::NodeRequirement,
    resource: ResourceKind,
    candidate: TileRect,
) -> TilePlanError {
    let operation_name = node.operation_name().to_owned();
    match reason {
        EstimateError::UnknownResource => TilePlanError::UnknownRequiredResource {
            node_index,
            operation_id: node.operation_id(),
            operation_name,
            resource,
            candidate,
        },
        EstimateError::UnboundedResource => TilePlanError::UnboundedRequiredResource {
            node_index,
            operation_id: node.operation_id(),
            operation_name,
            resource,
            candidate,
        },
        EstimateError::ArithmeticOverflow => TilePlanError::Estimate {
            node_index,
            operation_id: node.operation_id(),
            operation_name,
            resource,
            reason,
            candidate,
        },
    }
}

impl fmt::Display for TilePlanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBudget => formatter.write_str("tile budget must be nonzero"),
            Self::InvalidDeviceLimits => formatter.write_str("device tile limits must be nonzero"),
            Self::BackendUnavailable { backend } => write!(
                formatter,
                "{} backend requirements are unavailable",
                backend.tag()
            ),
            Self::BudgetBelowReserve {
                hard_limit,
                reserve,
            } => write!(
                formatter,
                "budget {hard_limit} is below safety reserve {reserve}"
            ),
            Self::EmptyOutputNotAllowed => {
                formatter.write_str("empty output is not allowed by the ROI request")
            }
            Self::OutputOutsideRoi => {
                formatter.write_str("requested output is outside the ROI chain")
            }
            Self::ConflictingTileDimensions => {
                formatter.write_str("tile dimension requirements have no candidate")
            }
            Self::MinimumTileDoesNotFit { .. } => {
                formatter.write_str("minimum tile does not fit the hard budget")
            }
            Self::UnknownRequiredResource {
                operation_name,
                resource,
                ..
            } => write!(
                formatter,
                "unknown {resource:?} resource for {operation_name}"
            ),
            Self::UnboundedRequiredResource {
                operation_name,
                resource,
                ..
            } => write!(
                formatter,
                "unbounded {resource:?} resource for {operation_name}"
            ),
            Self::Estimate { reason, .. } => reason.fmt(formatter),
            Self::MaxAllocation {
                operation_name,
                resource,
                bytes,
                limit,
                ..
            } => write!(
                formatter,
                "{operation_name} {resource:?} allocation {bytes} exceeds {limit}"
            ),
            Self::OverBudget {
                required,
                available,
                ..
            } => write!(
                formatter,
                "tile estimate {required} exceeds usable budget {available}"
            ),
            Self::RoiNodeMismatch => {
                formatter.write_str("ROI stage count does not match enabled nodes")
            }
            Self::Geometry(error) => error.fmt(formatter),
            Self::ArithmeticOverflow => formatter.write_str("tile planning arithmetic overflowed"),
            Self::ReceiptLimit => {
                formatter.write_str("tile estimate receipt exceeded its component bound")
            }
        }
    }
}

impl std::error::Error for TilePlanError {}
