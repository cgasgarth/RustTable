#![forbid(unsafe_code)]
#![doc = "Pure, deterministic logical export artifact planning for `RustTable`."]

mod artifact;
mod contract;
pub mod encoders;
mod manifest;
mod png;

pub use artifact::{CanonicalArtifact, Density, DensityUnit, ExportMetadata, MetadataText};
pub use contract::{
    AlphaPolicy, ArtifactBuffer, ArtifactError, BitDepth, ChannelLayout, Dependency,
    DependencySnapshot, DestinationSettings, DitherPolicy, EXPORT_CONTRACT_SCHEMA, EncoderSettings,
    ExportArtifact, ExportContractError, ExportPriority, ExportRequest, ExportValidationError,
    Interpolation, MetadataAction, MetadataPolicy, OutputProfile, PipelineQuality, PixelEncoding,
};
pub use encoders::resource::{EncodeBudget, EncodeCancellation, NeverCancel};
pub use manifest::{
    ArtifactKind, CollisionGroup, DestinationCapabilities, ExportPlan, ExportPlanError,
    LogicalArtifact,
};
pub use png::{
    CollisionPolicy, PngCollisionResult, PngExportLimits, PngExportLimitsError, PngExportReceipt,
    PngPublishCompletion, PngPublishControl, PngPublishError, PngPublishObserver,
    PngPublishProgress, PngPublishStage, PngPublisher, PngVerificationReceipt,
};
