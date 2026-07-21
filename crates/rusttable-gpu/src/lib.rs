#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "GPU execution boundary contracts for the `RustTable` rewrite."]

mod cache;
mod contracts;
mod dispatch;
mod point;
pub mod purpose;
mod resource;
mod runtime;
pub mod shader;
mod submission;
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
