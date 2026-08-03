//! Managed raster overlay compositing.
//!
//! Assets are decoded once into immutable, content-addressed storage.  The
//! plan owns the inverse transform and sampling policy so full-frame and tiled
//! execution use identical coordinates.

#![forbid(unsafe_code)]
#![expect(
    clippy::missing_errors_doc,
    reason = "Overlay errors are documented by the shared typed operation contract."
)]
#![expect(
    clippy::must_use_candidate,
    reason = "Overlay plans are intentionally usable as owned side-effecting execution values."
)]

mod asset;
mod descriptor;
mod execution;
mod parameters;

pub use asset::{OverlayAsset, OverlayAssetError, OverlayAssetStore, OverlayDecodedFormat};
pub use descriptor::overlay_descriptor;
pub use execution::{
    OVERLAY_WGSL, OverlayExecution, OverlayExecutionError, OverlayPlan, OverlayReceipt,
    default_asset_limits,
};
pub use parameters::{
    OverlayAlpha, OverlayAnchor, OverlayBaseScale, OverlayChannel, OverlayCodecError,
    OverlayConfig, OverlayEdge, OverlayHistory, OverlayImageScale, OverlayInterpolation,
    OverlayParametersV1, OverlayProfilePolicy, OverlayReference, decode_history, migrate_history,
};

pub const OVERLAY_COMPATIBILITY_ID: &str = "overlay";
pub const OVERLAY_RUST_ID: &str = "rusttable.overlay";
pub const OVERLAY_SCHEMA_VERSION: u16 = 1;
pub const OVERLAY_PARAMETER_VERSION: u16 = 1;
pub const OVERLAY_IMPLEMENTATION_VERSION: u16 = 1;
pub const OVERLAY_PARAMETER_BYTES: usize = 1088;
