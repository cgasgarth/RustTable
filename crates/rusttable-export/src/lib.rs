#![forbid(unsafe_code)]
#![doc = "Pure, deterministic logical export artifact planning for `RustTable`."]

mod artifact;
mod capabilities;
mod contract;
pub mod encoders;
mod errors;
mod hash_helpers;
mod manifest;
mod png;
mod queue_store;
mod recipe;
mod recipe_parse;

pub use artifact::{CanonicalArtifact, Density, DensityUnit, ExportMetadata, MetadataText};
pub use capabilities::{
    CapabilityAlternative, CapabilityFinding, CapabilityReport, CapabilitySet,
    DestinationCapabilityDescriptor, EncoderCapabilityDescriptor, MetadataField,
};
pub use contract::{
    AlphaPolicy, ArtifactBuffer, ArtifactError, BitDepth, ChannelLayout, Dependency,
    DependencySnapshot, DestinationSettings, DitherPolicy, EXPORT_CONTRACT_SCHEMA, EncoderSettings,
    ExportArtifact, ExportPriority, ExportRequest, Interpolation, MetadataAction, MetadataPolicy,
    OutputProfile, PipelineQuality, PixelEncoding,
};
pub use encoders::copy;
pub use encoders::resource::{EncodeBudget, EncodeCancellation, NeverCancel};
pub mod queue;
pub use errors::{ExportContractError, ExportValidationError};
pub use manifest::{
    ArtifactKind, CollisionGroup, DestinationCapabilities, ExportPlan, ExportPlanError,
    LogicalArtifact,
};
pub use png::{
    CollisionPolicy, PngCollisionResult, PngExportLimits, PngExportLimitsError, PngExportReceipt,
    PngPublishCompletion, PngPublishControl, PngPublishError, PngPublishObserver,
    PngPublishProgress, PngPublishStage, PngPublisher, PngVerificationReceipt,
};
pub use queue_store::{
    ExportJobId, ExportJobPriority, ExportJobRecord, ExportJobStage, ExportJobState,
    ExportQueueError, RedbExportQueueStore, queue_now_millis,
};
pub use recipe::{
    EXPORT_RECIPE_SCHEMA, ExportRecipe, ExportRecipeDraft, ImportConflictPolicy, OutputProfileSpec,
    PostSuccessAction, ProfileReference, RecipeDestination, RecipeError, RecipeId, RecipeRevision,
    RecipeTemplate,
};
