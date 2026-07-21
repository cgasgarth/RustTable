#![forbid(unsafe_code)]
#![doc = "Pixel pipeline composition boundary for the `RustTable` rewrite."]

mod cache;
mod cancellation;
mod cpu;
mod failure;
mod gpu;
mod histogram;
mod host_pool;
mod image;
mod mode;
mod pipeline;
pub mod purpose;
mod roi;
mod scheduler;
mod snapshot;
mod tiling;

pub use cache::key::{
    CacheKey, CacheKeyBuilder, CacheKeyComponent, CacheKeyDigest, CacheKeyError, CachePrecision,
    CacheQuality, NodeBoundary, OutputIdentity,
};
pub use cache::value::{
    AnalysisValue, CacheValue, CreationCost, PlanValue, ValueDescriptor, ValueKind,
};
pub use cache::{
    Cache, CacheConfig, CacheError, CacheEvent, CacheLease, CacheMetrics, CacheReceipt, CacheScope,
    FailureDiagnostic, InvalidationReceipt, ShutdownReport,
};
pub use cancellation::{
    CancellationDeadline, CancellationError, CancellationReason, CancellationScope,
    CancellationStage, CancellationToken, CleanupRegistration, GenerationClock,
    GenerationClockError,
};
pub use pipeline::publication::{
    CachePublicationPermit, GpuRetirement, ProductPublicationPermit, PublicationContext,
    PublicationError, PublicationGate, PublicationIdentity, PublicationPermit, PublicationTarget,
    RequestId, ResourceRetirementReceipt,
};

pub use cpu::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeResult,
    CpuTileAssemblyError,
};
pub use failure::{
    AttemptReceipt, CacheAction, CleanupAction, Failure, FailureBackend, FailureCacheHook,
    FailureCategory, FailureCleanupHook, FailureError, FailureLedger, FailurePolicy,
    FailurePrecedence, FailurePublicationHook, FailureRetryability, FailureScope, FailureStage,
    FinalImplementation, FinalReceipt, HookError, MAX_ATTEMPTS, MAX_SECONDARY_FAILURES,
    OutputCandidate, OutputExpectation, OutputValidationError, OutputValidationReceipt,
    OutputValidator, PolicyAction, PolicyError, PolicyRequest, PublicationAction, QuarantineHint,
    ReceiptBuilder, integrate_failure,
};
pub use gpu::{
    PixelpipeBackend, PixelpipeExecutionReceipt, PixelpipeExecutionResult,
    PixelpipeExecutionService, PixelpipeTilingReceipt,
};
pub use histogram::{
    HistogramAggregationError, HistogramAggregator, HistogramChannel, HistogramChannelModel,
    HistogramChannelResult, HistogramMaskPolicy, HistogramMergeError, HistogramNonFinitePolicy,
    HistogramRange, HistogramRangeError, HistogramRaster, HistogramRasterError, HistogramRequest,
    HistogramRequestError, HistogramResult,
};
pub use host_pool::temporary_buffer_request;
pub use image::{
    RgbaF32AlphaMode, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, SourceRasterIdentity,
};
pub use mode::{
    AnalysisRequest, ApproximationId, ApproximationId as FailureApproximationId, BackendPolicy,
    BasicStackFixture, DegradationPolicy, EmbeddedPreviewProvenance, Interpolation, LatencyClass,
    MODE_SCHEMA_VERSION, MaskRequest, ModeFinding, ModeOperationCapability, ModePlan, ModePlanner,
    ModePlanningError, ModeQuality, ModeReceipt, ModeRequest, ModeRequestError, OperationInclusion,
    PipelineModePlan, PipelineModePlanner, PipelineModeRequest, QualityPreset, Synchronization,
    TargetIdentity,
};
pub use pipeline::contracts::{
    Background, BlendStatus, ColorIdentity, ContractError, GenerationError, ImplementationIdentity,
    MaskStatus, OutputSpec, PIPELINE_SCHEMA_VERSION, PipelineGeneration, PipelineInput,
    PipelineMode, PipelinePurpose, PipelineQuality, PipelineSnapshotIdentity,
    PublicationGeneration, RasterStatus, ResourceMetadata, SnapshotDiff, SnapshotDiffComponent,
    SourceDescriptor, SourceIdentity, WORKING_COLOR,
};
pub use pipeline::preparation::{
    DescriptorPreparationSource, NodeCacheability, OperationPreparationSource, PipelinePreparer,
    PreparationContext, PreparationError, PreparationReceipt, PreparationSourceError, PreparedNode,
    PreparedNodeIdentity, PreparedOperation, PreparedPipeline,
};
pub use pipeline::receipt::{
    CpuImplementation, CpuNodeReceipt, CpuPipelineReceipt, CpuPipelineReceiptError, PixelIdentity,
};
pub use pipeline::snapshot::{PipelineSnapshot, PipelineSnapshotInput};
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
pub use scheduler::executor::{CpuWorkerPoolBoundary, WorkUnitCancellationBoundary};
pub use scheduler::metrics::{
    FairnessReceipt, SchedulerMetrics, SchedulerSnapshot,
    ShutdownReport as SchedulerShutdownReport, WorkUnitReceipt,
};
pub use scheduler::model::{
    AdmitReceipt, CancellationBoundary, CpuPriority, LeaseClaim, PublicationTargetKind,
    ResourceClaim, RunningTask, SchedulerConfig, SchedulerConfigError, SchedulerError,
    SchedulerPublicationTarget, SchedulerReceipt, ShutdownMode, TaskError, TaskFailure, TaskId,
    TaskSpec, TaskState, WorkUnitBoundary,
};
pub use snapshot::{CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, CpuPixelpipeSnapshotIdentity};
pub use tiling::geometry::{
    EdgeOverlap, GeometryError, RoiChain, RoiStage, ScaleRatio, TileAlignment, TileRect,
};
pub use tiling::models::{
    DominantResource, EstimateComponent, EstimateFailure, MemoryEstimate, PlannedTile,
    PlannedTileGrid, TileInputRoi, TilePlan, TilePlanIdentity, TilePlanReceipt,
};
pub use tiling::planner::{
    MemoryBudget, TileDeviceLimits, TilePlanError, TilePlanRequest, TilePlanner,
};
pub use tiling::requirements::{
    AreaSource, BackendRequirement, BufferRequirement, BytesPerPixel, EstimateError,
    FullFrameRequirements, MemoryFactor, NodeRequirement, NodeRequirements, ResourceKind,
    TileBackend, TileDimensions,
};
pub use tiling::tile::{CpuPixelpipeTile, CpuTileGrid, CpuTilePlan, CpuTilePlanError};

/// Compatibility name for callers not yet migrated to explicit snapshots.
pub type CpuPixelpipeRequest = CpuPixelpipeSnapshot;
