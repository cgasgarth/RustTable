//! Compatibility implementation of darktable's legacy `spots` operation.
//!
//! The history identity remains `spots`; the operation is intentionally kept
//! separate from `retouch` so imported histories retain their original form
//! IDs, mode ordering, and source coordinates.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::must_use_candidate
)]

mod codec;
mod descriptor;
mod execution;

pub use codec::{
    SPOTS_PARAMETER_BYTES_V1, SPOTS_PARAMETER_BYTES_V2, SPOTS_SCHEMA_VERSION, SpotsCodecError,
    SpotsHistory, SpotsLegacySpot, SpotsMode, SpotsParametersV1, SpotsParametersV2,
    migrate_v1_to_v2,
};
pub use descriptor::spots_descriptor;
pub use execution::{
    SpotsConfig, SpotsEntry, SpotsExecutionError, SpotsForm, SpotsFormKind, SpotsPlan, SpotsReceipt,
};

pub const SPOTS_COMPATIBILITY_ID: &str = "spots";
pub const SPOTS_RUST_ID: &str = "rusttable.spots";
pub const SPOTS_IMPLEMENTATION_VERSION: u16 = 1;
pub const SPOTS_MAX_ENTRIES: usize = 64;
