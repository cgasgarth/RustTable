//! Bounded, unregistered LUT 3D CPU leaf.
//!
//! Direct source lineage:
//! - `src/iop/lut3d.c`
//! - `src/iop/lut3dgmic.cpp` (deferred GMIC boundary)
//! - `src/common/iop_profile.c` and `src/common/iop_profile.h` (explicit profile seam)
//! - `src/common/colorspaces.c`, `src/common/colorspaces.h`, and
//!   `src/common/colorspaces_inline_conversions.h` (profile evidence)
//! - `data/kernels/lut3d.cl` (interpolation parity)
//!
//! This module is intentionally not added to the shared operation module,
//! registry, evaluator, pixelpipe, history/import, UI, or GPU routes.  It owns
//! only the fixed history codec, safe text readers, native interpolation, and
//! an explicit profile/cancellation CPU boundary.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_precision_loss,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::similar_names,
    clippy::too_many_arguments,
    reason = "source-shaped numeric code and the unregistered leaf boundary keep native field order explicit"
)]

mod codec;
mod execution;
mod interpolation;
mod parser;
mod profile;

pub use codec::{
    LUT3D_CLUT_LEVEL, LUT3D_COMPRESSED_CLUT_BYTES, LUT3D_MAX_KEYPOINTS, LUT3D_MAX_LUTNAME,
    LUT3D_MAX_PATHNAME, LUT3D_SCHEMA_VERSION, LUT3D_V1_PARAMETER_BYTES, LUT3D_V2_PARAMETER_BYTES,
    LUT3D_V3_PARAMETER_BYTES, Lut3dCodecError, Lut3dColorspace, Lut3dHistory, Lut3dInterpolation,
    Lut3dParameters,
};
pub use execution::{FrameDimensions, Lut3dExecutionError, Lut3dPlan};
pub use parser::{Lut3d, Lut3dParseError};
#[allow(unused_imports)]
pub use profile::{Lut3dProfileContext, Lut3dProfileError, Lut3dProfileEvidence, Matrix3};
