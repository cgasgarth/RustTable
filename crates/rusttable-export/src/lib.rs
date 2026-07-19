#![forbid(unsafe_code)]
#![doc = "Pure, deterministic logical export artifact planning for `RustTable`."]

mod manifest;

pub use manifest::{
    ArtifactKind, CollisionGroup, DestinationCapabilities, ExportPlan, ExportPlanError,
    ExportRequest, LogicalArtifact,
};
