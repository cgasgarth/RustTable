#![forbid(unsafe_code)]
#![doc = "Bounded, deterministic AI image workflows for `RustTable`."]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::format_collect,
    clippy::implicit_clone,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::too_many_arguments,
    clippy::too_many_lines,
    clippy::uninlined_format_args,
    clippy::unreadable_literal,
    clippy::useless_conversion
)]

mod cache;
mod color;
mod contracts;
mod inference;
mod model;
mod package;
mod planning;
mod qualification;
mod registry;
mod tiff;
mod types;
mod workflow;

/// Color/profile and model-space boundary used by AI workflows.
pub mod rgb {
    pub use crate::color::{ColorProfile, ColorProfileError, ProfileKind, RgbMatrix};
    pub use crate::rgb_service::*;
}

mod rgb_service;

/// Workflow namespaces kept stable as later AI tasks are added.
pub mod workflows;

/// Safe native boundary types. No runtime pointers, allocators, or provider handles are exported.
pub mod native {
    pub use rusttable_ai_native::{
        AdapterError, Cancellation, Dimension, GraphMetadata, GraphOptimization, RuntimeAdapter,
        RuntimeConfiguration, RuntimeIdentity, Session, TensorBuffer, TensorDataType,
        TensorDescriptor, UnavailableRuntime,
    };
}

pub use cache::{SessionCache, SessionKey};
pub use contracts::{
    BatchPolicy, BlendWindow, ColorContract, ColorPrimaries, ColorRange, ContractError, DataAsset,
    DimensionSpec, MODEL_PACKAGE_SCHEMA, ManifestHashes, OnnxContract, RegistryManifest,
    ResourceContract, TensorContract, TensorDtype, TensorSpec, TileContract, TileCropContract,
};
pub use inference::{InferenceError, InferenceOutput, InferenceRequest, InferenceService};
pub use package::{ModelIdentity, ModelPackage, PackageError, PackageLimits};
pub use planning::{InferenceLimits, InferencePlan, InferenceTile, PlanningError};
pub use qualification::{
    ProviderQualificationReceipt, QualificationError, QualificationFixture, QualificationStore,
};
pub use registry::{InstalledModel, ModelRegistry, ModelRegistrySnapshot, RegistryError};

pub use color::{ColorProfile, ColorProfileError, ProfileKind, RgbMatrix};
pub use model::{
    AlphaPolicy, AuxiliaryTensor, EdgePadding, ModelColorContract, ModelInferenceError,
    ModelManifest, ModelTask, ModelTileContract, ModelValidationError, Provider, ProviderPolicy,
    SuperResolutionModel, TensorLayout, TileCrop, TransferFunction,
};
pub use tiff::{
    TiffBitDepth, TiffProbe, TiffSampleFormat, TiffSettings, TiffSettingsError,
    verify_tiff_artifact,
};
pub use types::{
    CancellationToken, ImageDimensions, LinearRgbaImage, LinearRgbaImageError, OutputProfile,
    ShadowPolicy, SuperResolutionRequest, SuperResolutionScale, SuperResolutionSettings,
};
pub use workflow::{
    CatalogError, CatalogPort, CatalogReceipt, FileTiffPublisher, MemoryLimits, ModelInput,
    ModelOutput, PlannedTile, PublicationError, PublicationReceipt, SuperResolutionError,
    SuperResolutionPlan, SuperResolutionProgress, SuperResolutionPublisher, SuperResolutionReceipt,
    execute_super_resolution, plan_super_resolution,
};
