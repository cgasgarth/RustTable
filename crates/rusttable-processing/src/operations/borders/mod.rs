//! Darktable-compatible borders and frame-line rendering.
//!
//! This backend is intentionally independent of UI controls.  [`BordersPlan`]
//! is the canonical geometry and scalar renderer used by full-frame and tiled
//! callers.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc, clippy::must_use_candidate)]

mod descriptor;
mod execution;
mod geometry;
mod parameters;

pub use descriptor::borders_descriptor;
pub use execution::{BordersExecution, BordersExecutionError};
pub use geometry::{BordersFrame, BordersGeometry, BordersPlan, BordersPlanError};
pub use parameters::{
    BordersAspect, BordersBasis, BordersCodecError, BordersColor, BordersConfig, BordersHistory,
    BordersOrientation, BordersParametersV1, BordersParametersV2, BordersParametersV3,
    BordersParametersV4, decode_history, migrate_history,
};

pub const BORDERS_COMPATIBILITY_ID: &str = "borders";
pub const BORDERS_RUST_ID: &str = "rusttable.borders";
pub const BORDERS_SCHEMA_VERSION: u16 = 4;
pub const BORDERS_PARAMETER_VERSION: u16 = 4;
pub const BORDERS_IMPLEMENTATION_VERSION: u16 = 1;
pub const BORDERS_PARAMETER_BYTES_V1: usize = 24;
pub const BORDERS_PARAMETER_BYTES_V2: usize = 112;
pub const BORDERS_PARAMETER_BYTES_V3: usize = 116;
pub const BORDERS_PARAMETER_BYTES_V4: usize = 120;

/// Reflected GPU entry points.  The CPU plan remains the rounding authority.
pub const BORDERS_WGSL: &str = r"
struct BordersParams { width: u32, height: u32, source_x: u32, source_y: u32,
  source_width: u32, source_height: u32, border: vec4<f32>, frame: vec4<f32> }
@group(0) @binding(0) var<uniform> params: BordersParams;
@compute @workgroup_size(8, 8, 1)
fn borders_fill(@builtin(global_invocation_id) id: vec3<u32>) {
  if (id.x >= params.width || id.y >= params.height) { return; }
}
";
