#![forbid(unsafe_code)]
#![doc = "Pixel pipeline composition boundary for the `RustTable` rewrite."]

mod cpu;
mod image;
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
pub use receipt::{
    CpuImplementation, CpuNodeReceipt, CpuPipelineReceipt, CpuPipelineReceiptError, PixelIdentity,
};
pub use snapshot::{CpuPixelpipeSnapshot, CpuPixelpipeSnapshotError, CpuPixelpipeSnapshotIdentity};
pub use tile::{CpuPixelpipeTile, CpuTileGrid, CpuTilePlan, CpuTilePlanError};

/// Compatibility name for callers not yet migrated to explicit snapshots.
pub type CpuPixelpipeRequest = CpuPixelpipeSnapshot;
