#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "GPU execution boundary contracts for the `RustTable` rewrite."]

mod cache;
mod contracts;
mod dispatch;
mod point;
pub mod purpose;
pub mod recovery;
mod resource;
mod runtime;
pub mod shader;
mod submission;
pub mod tiling;
pub mod transfer;

pub use contracts::{
    AdapterCandidate, AdapterIdentity, AdapterSelection, AdvertisedFeatures, Backend,
    BackendPolicy, DeviceClass, ExecutionTier, FaultKind, FaultState, FaultTracker,
    GpuCapabilitySnapshot, GpuFaultSnapshot, GpuFeaturePlan, KernelPlanError, LimitEnvelope,
    Platform, PowerPreference, ProbeLedger, RejectedAdapter, SelectionError, select_adapter,
};

pub use cache::{CacheError, PipelineCacheIdentity, PipelineCacheStore};
pub use dispatch::{
    BindingResource, CancellationToken, CommandEncoder, DispatchBatch, DispatchError,
    DispatchFailure, DispatchRegion, EncodedBatch, EncodedDispatch, EncodingReceipt, GridPlan,
    KernelIdentity, ParameterBlock, ParameterValue, ParityContract, PrepareRequest,
    PreparedGpuKernel, ReceiptStatus, ScalarValue, Tile, TypedParameters,
};
pub use point::{BasicPointError, BasicPointOperation, BasicPointRequest, BasicPointResult};
pub use recovery::{
    AssemblyPlan, AssemblyReceipt, AssemblyTile, AttemptFailure, AttemptFailureKind, AttemptId,
    AttemptOutcome, AttemptReceipt, AttemptResources, CleanupStatus,
    CoverageError as AssemblyCoverageError, CoverageReceipt as AssemblyCoverageReceipt,
    CoverageRect, MAX_GPU_ATTEMPTS, MAX_OOM_RETRIES, OutputFragment, PlanIdentity,
    PublicationBackend, PublicationReceipt, RecoveryAttemptPlan, RecoveryContext, RecoveryDecision,
    RecoveryError, RecoveryRequest, RecoverySession, SnapshotIdentity,
    TileCandidate as RecoveryTileCandidate,
};
pub use resource::{
    DeviceGeneration, GpuResourcePool, InitializationPolicy, PoolError, PoolEvent,
    ResourceAccounting, ResourceClass, ResourceFormat, ResourceId, ResourceKind, ResourceLease,
    ResourceMetrics, ResourcePoolConfig, ResourcePriority, ResourceRequest, ResourceState,
    SubmissionId, SubmissionToken,
};
pub use runtime::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
pub use submission::{
    AdmissionLimitKind, CancellationOutcome, CompletionOutcome, CompletionReceipt,
    CompletionSignal, DispatchOutcome, RuntimeSubmissionError, SubmissionBackend, SubmissionError,
    SubmissionLimits, SubmissionPacket, SubmissionQueue, SubmissionRequest, SubmissionRuntime,
    SubmissionState,
};
pub use tiling::{
    CoverageError, CoverageModel, EdgeOverlap, GpuTileCandidate, GpuTileCandidate as TileCandidate,
    GpuTilePlanner, GpuTileRequest, GpuTileRequest as TileRequest, GpuTilingPlan,
    GpuTilingPlan as TilePlan, HostBoundary, Lifetime, PlannedGpuTile, ResidencyError,
    ResidencyPlan, ResidentIntermediate, ResourceAllocationEstimate, TileAlignment, TileArea,
    TileMemoryBudget, TileMemoryEstimate, TileResourceSpec, TilingError, TilingReceipt,
};

#[cfg(test)]
#[path = "tiling_tests.rs"]
mod gpu_tiling_tests;
