//! Hidden Darktable-compatible sensor-pixel rotation.
//!
//! The implementation is split by responsibility so the operation remains
//! auditable and each source file stays within the project line-size policy.

#[path = "rotatepixels/codec.rs"]
mod codec;
#[path = "rotatepixels/descriptor.rs"]
mod descriptor;
#[path = "rotatepixels/execution.rs"]
mod execution;
#[path = "rotatepixels/geometry.rs"]
mod geometry;
#[path = "rotatepixels/sampling.rs"]
mod sampling;

pub use codec::*;
pub use descriptor::*;
pub use execution::*;
pub use geometry::*;
