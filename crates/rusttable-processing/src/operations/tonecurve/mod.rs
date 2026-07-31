//! Bounded, operation-local CPU Tone Curve leaf ported from
//! `src/iop/tonecurve.c`.
//!
//! This module owns the native v5 ABI, exact legacy migrations, V1 curve
//! sampling/quantization, D50 Lab/XYZ and ProPhoto conversions, explicit
//! profile evidence, cancellation/publication boundaries, and alpha-preserving
//! CPU execution. Registry, production history, pixelpipe, GPU, GTK, blending,
//! and preset integration remain deferred rather than approximated.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::large_enum_variant,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::struct_excessive_bools,
    clippy::unused_self,
    clippy::collapsible_if,
    clippy::large_types_passed_by_value,
    clippy::neg_cmp_op_on_partial_ord,
    unused_imports,
    dead_code,
    reason = "the source-shaped leaf keeps native ABI and numerical boundaries explicit"
)]

mod codec;
mod curve;
mod execution;
mod parameters;
pub mod source_map;

pub use codec::{ToneCurveCodecError, ToneCurveHistory, abi_offsets};
pub use curve::{
    CompiledToneCurve, CompiledToneCurveSet, CurveCompileError, PROFILE_MATRIX_ORIENTATION,
    ToneCurveProfileError, ToneCurveProfileEvidence, compile_parameters,
};
pub use execution::{
    ALLOW_TILING, DEFAULT_COLORSPACE, DEFAULT_GROUPS, DESCRIPTION, GPU_SUPPORTED, GTK_SUPPORTED,
    OPERATION_NAME, SUPPORTS_BLENDING, ToneCurveCapabilities, ToneCurveExecution,
    ToneCurveExecutionError, ToneCurvePixel, ToneCurvePlan, ToneCurveRuntime, ToneCurveTile,
    capabilities, channel_count, lut_resolution,
};
pub use parameters::{
    CHANNELS, LEGACY_V1_BYTES, LEGACY_V3_BYTES, LEGACY_V4_BYTES, LUT_RESOLUTION, MAX_NODES,
    PARAMETER_BYTES, PARAMETER_VERSION, ParameterError, PreserveColors, ToneCurveAutoscale,
    ToneCurveChannel, ToneCurveNode, ToneCurveParametersV5, ToneCurveType,
};

/// Operation-local source fixture used by focused contract tests.
pub const DEFAULT_V5_FIXTURE: &str = include_str!("fixtures/default_v5.txt");
