#![forbid(unsafe_code)]
#![doc = "Pure, deterministic logical export artifact planning for `RustTable`."]

mod artifact;
pub mod encoders;
mod manifest;
mod png;

pub use artifact::{CanonicalArtifact, Density, DensityUnit, ExportMetadata, MetadataText};

pub use manifest::{
    ArtifactKind, CollisionGroup, DestinationCapabilities, ExportPlan, ExportPlanError,
    ExportRequest, LogicalArtifact,
};
pub use png::{
    CollisionPolicy, PngCollisionResult, PngExportLimits, PngExportLimitsError, PngExportReceipt,
    PngPublishCompletion, PngPublishControl, PngPublishError, PngPublishObserver,
    PngPublishProgress, PngPublishStage, PngPublisher, PngVerificationReceipt,
};
