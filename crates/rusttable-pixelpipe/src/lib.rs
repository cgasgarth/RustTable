#![forbid(unsafe_code)]
#![doc = "Pixel pipeline composition boundary for the `RustTable` rewrite."]

mod cpu;
mod image;
mod pipeline_contracts;
mod pipeline_snapshot;
mod preparation;
pub mod purpose;
mod receipt;
mod snapshot;
mod tile;

pub use cpu::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeResult,
    CpuTileAssemblyError,
};
pub use image::{
    RgbaF32AlphaMode, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, SourceRasterIdentity,
};
pub use pipeline_contracts::{
    Background, BlendStatus, ColorIdentity, ContractError, GenerationError, ImplementationIdentity,
    MaskStatus, OutputSpec, PIPELINE_SCHEMA_VERSION, PipelineGeneration, PipelineInput,
    PipelineMode, PipelinePurpose, PipelineQuality, PipelineSnapshotIdentity,
    PublicationGeneration, RasterStatus, ResourceMetadata, SnapshotDiff, SnapshotDiffComponent,
    SourceDescriptor, SourceIdentity, WORKING_COLOR,
};
pub use pipeline_snapshot::{PipelineSnapshot, PipelineSnapshotInput};
pub use preparation::{
    DescriptorPreparationSource, NodeCacheability, OperationPreparationSource, PipelinePreparer,
    PreparationContext, PreparationError, PreparationReceipt, PreparationSourceError, PreparedNode,
    PreparedNodeIdentity, PreparedOperation, PreparedPipeline,
};
pub use receipt::{
    CpuImplementation, CpuNodeReceipt, CpuPipelineReceipt, CpuPipelineReceiptError, PixelIdentity,
};
pub use snapshot::{CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, CpuPixelpipeSnapshotIdentity};
pub use tile::{CpuPixelpipeTile, CpuTileGrid, CpuTilePlan, CpuTilePlanError};

/// Compatibility name for callers not yet migrated to explicit snapshots.
pub type CpuPixelpipeRequest = CpuPixelpipeSnapshot;
