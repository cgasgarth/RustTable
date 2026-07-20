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

mod color;
mod model;
mod tiff;
mod types;
mod workflow;

/// Color/profile and model-space boundary used by AI workflows.
pub mod rgb {
    pub use crate::color::{ColorProfile, ColorProfileError, ProfileKind, RgbMatrix};
}

/// Workflow namespaces kept stable as later AI tasks are added.
pub mod workflows {
    pub mod super_resolution {
        pub use crate::model::{
            AlphaPolicy, AuxiliaryTensor, ModelColorContract, ModelInferenceError, ModelManifest,
            ModelTask, ModelTileContract, ModelValidationError, Provider, ProviderPolicy,
            SuperResolutionModel,
        };
        pub use crate::tiff::{
            TiffBitDepth, TiffProbe, TiffSampleFormat, TiffSettings, TiffSettingsError,
            verify_tiff_artifact,
        };
        pub use crate::types::{
            CancellationToken, ImageDimensions, LinearRgbaImage, LinearRgbaImageError,
            OutputProfile, ShadowPolicy, SuperResolutionRequest, SuperResolutionScale,
            SuperResolutionSettings,
        };
        pub use crate::workflow::{
            CatalogError, CatalogPort, CatalogReceipt, FileTiffPublisher, MemoryLimits, ModelInput,
            ModelOutput, PlannedTile, PublicationError, PublicationReceipt, SuperResolutionError,
            SuperResolutionPlan, SuperResolutionProgress, SuperResolutionPublisher,
            SuperResolutionReceipt, execute_super_resolution, plan_super_resolution,
        };
    }
}

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
