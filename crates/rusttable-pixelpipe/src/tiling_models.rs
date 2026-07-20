use std::ops::Range;

use crate::TileRect;
use crate::tiling_requirements::{EstimateError, ResourceKind, TileBackend};

/// One bounded arithmetic component in a memory estimate receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstimateComponent {
    pub(crate) node_index: usize,
    pub(crate) operation_id: u128,
    pub(crate) operation_name: String,
    pub(crate) resource: ResourceKind,
    pub(crate) bytes: u64,
}

impl EstimateComponent {
    #[must_use]
    pub const fn node_index(&self) -> usize {
        self.node_index
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub fn operation_name(&self) -> &str {
        &self.operation_name
    }
    #[must_use]
    pub const fn resource(&self) -> ResourceKind {
        self.resource
    }
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Dominant operation/resource identified by a candidate estimate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DominantResource {
    pub(crate) node_index: usize,
    pub(crate) operation_id: u128,
    pub(crate) operation_name: String,
    pub(crate) resource: ResourceKind,
    pub(crate) bytes: u64,
}

impl DominantResource {
    #[must_use]
    pub const fn node_index(&self) -> usize {
        self.node_index
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub fn operation_name(&self) -> &str {
        &self.operation_name
    }
    #[must_use]
    pub const fn resource(&self) -> ResourceKind {
        self.resource
    }
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Complete bounded estimate, including reserve and every live component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryEstimate {
    pub(crate) backend: TileBackend,
    pub(crate) total_bytes: u64,
    pub(crate) reserve_bytes: u64,
    pub(crate) required_bytes: u64,
    pub(crate) available_bytes: u64,
    pub(crate) components: Vec<EstimateComponent>,
    pub(crate) dominant: Option<DominantResource>,
}

impl MemoryEstimate {
    #[must_use]
    pub const fn backend(&self) -> TileBackend {
        self.backend
    }
    #[must_use]
    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }
    #[must_use]
    pub const fn reserve_bytes(&self) -> u64 {
        self.reserve_bytes
    }
    #[must_use]
    pub const fn required_bytes(&self) -> u64 {
        self.required_bytes
    }
    #[must_use]
    pub const fn available_bytes(&self) -> u64 {
        self.available_bytes
    }
    #[must_use]
    pub fn components(&self) -> &[EstimateComponent] {
        &self.components
    }
    #[must_use]
    pub fn dominant(&self) -> Option<&DominantResource> {
        self.dominant.as_ref()
    }
}

/// Input ROI attached to one planned output tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileInputRoi {
    pub(crate) operation_id: u128,
    pub(crate) rect: TileRect,
}

impl TileInputRoi {
    #[must_use]
    pub const fn operation_id(self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub const fn rect(self) -> TileRect {
        self.rect
    }
}

/// One exact output interior and all #267-derived input ROIs.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlannedTile {
    pub(crate) output: TileRect,
    pub(crate) input_rois: Vec<TileInputRoi>,
}

impl PlannedTile {
    #[must_use]
    pub const fn output(&self) -> TileRect {
        self.output
    }
    #[must_use]
    pub fn input_rois(&self) -> &[TileInputRoi] {
        &self.input_rois
    }
}

/// Row-major y/x exact-cover grid.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlannedTileGrid {
    pub(crate) output: TileRect,
    pub(crate) columns: u32,
    pub(crate) rows: u32,
    pub(crate) tiles: Vec<PlannedTile>,
}

impl PlannedTileGrid {
    #[must_use]
    pub const fn output(&self) -> TileRect {
        self.output
    }
    #[must_use]
    pub const fn columns(&self) -> u32 {
        self.columns
    }
    #[must_use]
    pub const fn rows(&self) -> u32 {
        self.rows
    }
    #[must_use]
    pub fn tiles(&self) -> &[PlannedTile] {
        &self.tiles
    }
}

/// Receipt identity for one accepted candidate/grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TilePlanIdentity(pub(crate) [u8; 32]);

impl TilePlanIdentity {
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Detailed, deterministic evidence for one accepted tile plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilePlanReceipt {
    pub(crate) identity: TilePlanIdentity,
    pub(crate) backend: TileBackend,
    pub(crate) enabled_range: Range<usize>,
    pub(crate) candidate_width: u32,
    pub(crate) candidate_height: u32,
    pub(crate) search_steps: u32,
    pub(crate) estimate: MemoryEstimate,
}

impl TilePlanReceipt {
    #[must_use]
    pub const fn identity(&self) -> TilePlanIdentity {
        self.identity
    }
    #[must_use]
    pub const fn backend(&self) -> TileBackend {
        self.backend
    }
    #[must_use]
    pub const fn enabled_range(&self) -> &Range<usize> {
        &self.enabled_range
    }
    #[must_use]
    pub const fn candidate_width(&self) -> u32 {
        self.candidate_width
    }
    #[must_use]
    pub const fn candidate_height(&self) -> u32 {
        self.candidate_height
    }
    #[must_use]
    pub const fn search_steps(&self) -> u32 {
        self.search_steps
    }
    #[must_use]
    pub const fn estimate(&self) -> &MemoryEstimate {
        &self.estimate
    }
}

/// Accepted immutable tile plan; it contains no allocation or execution handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TilePlan {
    pub(crate) backend: TileBackend,
    pub(crate) full_frame: bool,
    pub(crate) grid: PlannedTileGrid,
    pub(crate) estimate: MemoryEstimate,
    pub(crate) receipt: TilePlanReceipt,
}

impl TilePlan {
    #[must_use]
    pub const fn backend(&self) -> TileBackend {
        self.backend
    }
    #[must_use]
    pub const fn full_frame(&self) -> bool {
        self.full_frame
    }
    #[must_use]
    pub const fn grid(&self) -> &PlannedTileGrid {
        &self.grid
    }
    #[must_use]
    pub const fn estimate(&self) -> &MemoryEstimate {
        &self.estimate
    }
    #[must_use]
    pub const fn receipt(&self) -> &TilePlanReceipt {
        &self.receipt
    }
}

/// Bounded evidence used when aggregate memory exceeds the usable budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EstimateFailure {
    pub(crate) node_index: usize,
    pub(crate) operation_id: u128,
    pub(crate) operation_name: String,
    pub(crate) resource: ResourceKind,
    pub(crate) reason: EstimateError,
    pub(crate) candidate: TileRect,
}

impl EstimateFailure {
    #[must_use]
    pub const fn node_index(&self) -> usize {
        self.node_index
    }
    #[must_use]
    pub const fn operation_id(&self) -> u128 {
        self.operation_id
    }
    #[must_use]
    pub fn operation_name(&self) -> &str {
        &self.operation_name
    }
    #[must_use]
    pub const fn resource(&self) -> ResourceKind {
        self.resource
    }
    #[must_use]
    pub const fn reason(&self) -> EstimateError {
        self.reason
    }
    #[must_use]
    pub const fn candidate(&self) -> TileRect {
        self.candidate
    }
}
