#![forbid(unsafe_code)]
#![doc = "Pixel pipeline composition boundary for the `RustTable` rewrite."]

mod cache;
mod cache_key;
mod cache_value;
mod cancellation;
mod cpu;
mod host_pool;
mod image;
mod mode;
mod pipeline_contracts;
mod pipeline_snapshot;
mod preparation;
mod publication;
pub mod purpose;
mod receipt;
mod roi;
mod scheduler;
mod scheduler_executor;
mod scheduler_metrics;
mod scheduler_model;
mod snapshot;
mod tile;
mod tile_geometry;
mod tiling_models;
mod tiling_planner;
mod tiling_requirements;

pub use cache::{
    Cache, CacheConfig, CacheError, CacheEvent, CacheLease, CacheMetrics, CacheReceipt, CacheScope,
    FailureDiagnostic, InvalidationReceipt, ShutdownReport,
};
pub use cache_key::{
    CacheKey, CacheKeyBuilder, CacheKeyComponent, CacheKeyDigest, CacheKeyError, CachePrecision,
    CacheQuality, NodeBoundary, OutputIdentity,
};
pub use cache_value::{
    AnalysisValue, CacheValue, CreationCost, PlanValue, ValueDescriptor, ValueKind,
};
pub use cancellation::{
    CancellationDeadline, CancellationError, CancellationReason, CancellationScope,
    CancellationStage, CancellationToken, CleanupRegistration, GenerationClock,
    GenerationClockError,
};
pub use publication::{
    CachePublicationPermit, GpuRetirement, ProductPublicationPermit, PublicationContext,
    PublicationError, PublicationGate, PublicationIdentity, PublicationPermit, PublicationTarget,
    RequestId, ResourceRetirementReceipt,
};

pub use cpu::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeResult,
    CpuTileAssemblyError,
};
pub use host_pool::temporary_buffer_request;
pub use image::{
    RgbaF32AlphaMode, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, SourceRasterIdentity,
};
pub use mode::{
    AnalysisRequest, ApproximationId, BackendPolicy, BasicStackFixture, DegradationPolicy,
    EmbeddedPreviewProvenance, Interpolation, LatencyClass, MODE_SCHEMA_VERSION, MaskRequest,
    ModeFinding, ModeOperationCapability, ModePlan, ModePlanner, ModePlanningError, ModeQuality,
    ModeReceipt, ModeRequest, ModeRequestError, OperationInclusion, PipelineModePlan,
    PipelineModePlanner, PipelineModeRequest, QualityPreset, Synchronization, TargetIdentity,
};
pub use pipeline_contracts::{
    Background, BlendStatus, ColorIdentity, ContractError, GenerationError, ImplementationIdentity,
    MaskStatus, OutputSpec, PIPELINE_SCHEMA_VERSION, PipelineGeneration, PipelineInput,
    PipelineMode, PipelinePurpose, PipelineQuality, PipelineSnapshotIdentity,
    PublicationGeneration, RasterStatus, ResourceMetadata, SnapshotDiff, SnapshotDiffComponent,
    SourceDescriptor, SourceIdentity, WORKING_COLOR,
};
pub use pipeline_snapshot::{PipelineSnapshot, PipelineSnapshotInput};
pub use preparation::{
    DescriptorPreparationSource, NodeCacheability, OperationPreparationSource, PipelinePreparer,
    PreparationContext, PreparationError, PreparationReceipt, PreparationSourceError, PreparedNode,
    PreparedNodeIdentity, PreparedOperation, PreparedPipeline,
};
pub use receipt::{
    CpuImplementation, CpuNodeReceipt, CpuPipelineReceipt, CpuPipelineReceiptError, PixelIdentity,
};
pub use roi::{
    DistortionBinding, DistortionError, DistortionMapping, FillValue, NodeRoiContract,
    ROI_SCHEMA_VERSION, RationalScale, RoiBackwardStep, RoiDescriptor, RoiDescriptorIdentity,
    RoiError, RoiForwardStep, RoiNode, RoiPlan, RoiPlanIdentity, RoiPlanner, RoiPlanningError,
    RoiRect, RoiRequest, RoiRequestPolicy, RoiSupport,
};
pub use rusttable_image::{
    AcquireOptions, AllocationClass, BufferAlignment, BufferLease, BufferRead, BufferRequest,
    BufferUsage, BufferWrite, CancellationToken as HostPoolCancellationToken, HostBufferPool,
    HostImageView, HostPoolError, InitializationPolicy, LeaseState, PoolAccounting, PoolBudgets,
    PoolEvent, PriorityClass, ReturnReceipt, SharedBufferLease,
    ShutdownReport as HostPoolShutdownReport,
};
pub use scheduler::CpuScheduler;
pub use scheduler_executor::{CpuWorkerPoolBoundary, WorkUnitCancellationBoundary};
pub use scheduler_metrics::{
    FairnessReceipt, SchedulerMetrics, SchedulerSnapshot,
    ShutdownReport as SchedulerShutdownReport, WorkUnitReceipt,
};
pub use scheduler_model::{
    AdmitReceipt, CancellationBoundary, CpuPriority, LeaseClaim, PublicationTargetKind,
    ResourceClaim, RunningTask, SchedulerConfig, SchedulerConfigError, SchedulerError,
    SchedulerPublicationTarget, SchedulerReceipt, ShutdownMode, TaskError, TaskFailure, TaskId,
    TaskSpec, TaskState, WorkUnitBoundary,
};
pub use snapshot::{CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, CpuPixelpipeSnapshotIdentity};
pub use tile::{CpuPixelpipeTile, CpuTileGrid, CpuTilePlan, CpuTilePlanError};
pub use tile_geometry::{
    EdgeOverlap, GeometryError, RoiChain, RoiStage, ScaleRatio, TileAlignment, TileRect,
};
pub use tiling_models::{
    DominantResource, EstimateComponent, EstimateFailure, MemoryEstimate, PlannedTile,
    PlannedTileGrid, TileInputRoi, TilePlan, TilePlanIdentity, TilePlanReceipt,
};
pub use tiling_planner::{
    MemoryBudget, TileDeviceLimits, TilePlanError, TilePlanRequest, TilePlanner,
};
pub use tiling_requirements::{
    AreaSource, BackendRequirement, BufferRequirement, BytesPerPixel, EstimateError,
    FullFrameRequirements, MemoryFactor, NodeRequirement, NodeRequirements, ResourceKind,
    TileBackend, TileDimensions,
};

/// Compatibility name for callers not yet migrated to explicit snapshots.
pub type CpuPixelpipeRequest = CpuPixelpipeSnapshot;
