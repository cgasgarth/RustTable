//! Bounded CPU RGB Curve leaf ported from Darktable `src/iop/rgbcurve.c`.
//!
//! The native kernel and UI are retained read-only or deferred: this module
//! owns version-1 bytes, checked parameters, V1 curve compilation, CPU RGBA
//! execution, operation-local commit/editor state, and source-derived tests.
//! Production history routing, registry/descriptor projection, snapshot and
//! pixelpipe dispatch, GPU registration, shared blending, and GTK composition
//! remain explicit integration seams.
//!
//! Source lineage also includes `src/common/curve_tools.c`,
//! `src/common/curve_tools.h`, `src/common/iop_profile.h`,
//! `src/common/colorspaces_inline_conversions.h`, `src/common/rgb_norms.h`,
//! `src/develop/imageop_math.h`, `src/gui/draw.h`, and
//! `data/kernels/rgbcurve.cl`.

#![forbid(unsafe_code)]
#![allow(
    unused_imports,
    dead_code,
    clippy::unreadable_literal,
    clippy::similar_names,
    clippy::needless_range_loop,
    clippy::large_enum_variant,
    clippy::doc_markdown,
    clippy::double_must_use,
    clippy::too_many_arguments,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unused_self,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::cast_possible_wrap,
    clippy::struct_excessive_bools,
    clippy::collapsible_if,
    clippy::large_types_passed_by_value,
    reason = "the leaf preserves source-shaped numerical and state boundaries while integration owners wire its public contract"
)]

mod codec;
mod curve;
mod editor;
mod execution;
mod parameters;
mod presets;
pub mod source_map;

pub use codec::{RgbCurveCodecError, RgbCurveHistory, abi_offsets};
pub use curve::{
    CompiledCurve, CompiledCurveSet, CurveCompileError, PROFILE_MATRIX_ORIENTATION,
    RgbCurveProfileCacheKey, RgbCurveProfileError, RgbCurveProfileEvidence, compile_parameters,
    native_gpu_extrapolation_mismatch,
};
pub use editor::{EditorError, NODE_HIT_RADIUS_SQUARED, RgbCurveEditorState};
pub use execution::{
    ALLOW_TILING, DEFAULT_COLORSPACE, DEFAULT_GROUPS, DESCRIPTION, GPU_KERNEL_NAME,
    GPU_PROGRAM_INDEX, GPU_SUPPORTED, GTK_SUPPORTED, OPERATION_NAME, RgbCurveCapabilities,
    RgbCurveExecution, RgbCurveExecutionError, RgbCurvePixel, RgbCurvePlan, RgbCurveRuntime,
    RgbCurveTile, SUPPORTS_BLENDING, capabilities,
};
pub use parameters::{
    CHANNELS, LUT_RESOLUTION, MAX_NODES, MIN_X_DISTANCE, PARAMETER_BYTES, PARAMETER_VERSION,
    ParameterError, PreserveColors, RgbCurveAutoscale, RgbCurveChannel, RgbCurveNode,
    RgbCurveParametersV1, RgbCurveType,
};
pub use presets::{RgbCurvePreset, RgbCurvePresetBlendColorspace, init_presets};

/// The source fixture is kept beside the operation so integration tests can
/// assert the default V1 contract without duplicating its expected values.
pub const DEFAULT_V1_FIXTURE: &str = include_str!("fixtures/default_v1.txt");
