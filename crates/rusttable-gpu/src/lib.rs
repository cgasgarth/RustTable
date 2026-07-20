#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "GPU execution boundary contracts for the `RustTable` rewrite."]

mod cache;
mod contracts;
pub mod purpose;
mod runtime;

pub use contracts::{
    AdapterCandidate, AdapterIdentity, AdapterSelection, AdvertisedFeatures, Backend,
    BackendPolicy, DeviceClass, ExecutionTier, FaultKind, FaultState, FaultTracker,
    GpuCapabilitySnapshot, GpuFaultSnapshot, GpuFeaturePlan, KernelPlanError, LimitEnvelope,
    Platform, PowerPreference, ProbeLedger, RejectedAdapter, SelectionError, select_adapter,
};

pub use cache::{CacheError, PipelineCacheIdentity, PipelineCacheStore};
pub use runtime::{GpuInitError, GpuRuntime, GpuRuntimeConfig};
