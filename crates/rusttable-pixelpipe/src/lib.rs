#![forbid(unsafe_code)]
#![doc = "Pixel pipeline composition boundary for the `RustTable` rewrite."]

mod cpu;
mod image;
pub mod purpose;
mod receipt;
mod tile;

pub use cpu::{
    CpuPixelpipeError, CpuPixelpipeExecutor, CpuPixelpipeOutputMode, CpuPixelpipeRequest,
    CpuPixelpipeResult, CpuTileAssemblyError,
};
pub use image::{
    RgbaF32AlphaMode, RgbaF32Channel, RgbaF32ColorEncoding, RgbaF32Descriptor, RgbaF32Image,
    RgbaF32ImageError, RgbaF32Pixel, SourceRasterIdentity,
};
pub use receipt::{
    CpuImplementation, CpuNodeReceipt, CpuPipelineReceipt, CpuPipelineReceiptError, PixelIdentity,
};
pub use tile::{CpuPixelpipeTile, CpuTileGrid, CpuTilePlan, CpuTilePlanError};
