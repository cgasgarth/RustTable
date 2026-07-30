//! Hidden Darktable-compatible sensor-pixel rotation.
//!
//! The implementation is organized by codec, descriptor, execution, geometry,
//! and sampling responsibilities so each owner remains auditable.

mod codec;
mod descriptor;
mod execution;
mod geometry;
mod sampling;

pub use codec::*;
pub use descriptor::*;
pub use execution::*;
pub use geometry::*;
