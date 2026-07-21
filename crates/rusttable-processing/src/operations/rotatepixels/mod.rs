//! Hidden Darktable-compatible sensor-pixel rotation.
//!
//! The implementation is split by responsibility so the operation remains
//! auditable and each source file stays within the project line-size policy.

mod codec;
mod descriptor;
mod execution;
mod geometry;
mod sampling;

pub use codec::*;
pub use descriptor::*;
pub use execution::*;
pub use geometry::*;
