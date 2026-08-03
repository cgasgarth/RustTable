//! Bounded, operation-local CPU Tone Curve leaf ported from
//! `src/iop/tonecurve.c`.
//!
//! This module owns the native v5 ABI, exact legacy migrations, V1 curve
//! sampling/quantization, D50 Lab/XYZ and ProPhoto conversions, explicit
//! profile evidence, cancellation/publication boundaries, and alpha-preserving
//! CPU execution. Registry, production history, and pixelpipe CPU routing are
//! provided; GPU, GTK, blending extensions, and preset integration remain
//! explicitly unavailable.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::doc_markdown,
    clippy::large_enum_variant,
    clippy::many_single_char_names,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::cast_sign_loss,
    clippy::excessive_precision,
    clippy::struct_excessive_bools,
    clippy::unused_self,
    clippy::collapsible_if,
    clippy::large_types_passed_by_value,
    clippy::must_use_candidate,
    clippy::neg_cmp_op_on_partial_ord,
    clippy::unreadable_literal,
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
    ToneCurveProfileError, ToneCurveProfileEvidence, compile_parameters, requires_profile_evidence,
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

/// Checked v5 parameters carried by the processing graph.
///
/// The native payload is the identity for equality and hashing so snapshot
/// identity cannot depend on Rust's floating-point representation.
#[derive(Debug, Clone)]
pub struct ToneCurveConfig {
    parameters: ToneCurveParametersV5,
}

impl ToneCurveConfig {
    #[must_use]
    pub const fn new(parameters: ToneCurveParametersV5) -> Self {
        Self { parameters }
    }

    #[must_use]
    pub const fn parameters(&self) -> &ToneCurveParametersV5 {
        &self.parameters
    }

    #[must_use]
    pub fn payload(&self) -> [u8; PARAMETER_BYTES] {
        self.parameters.to_bytes()
    }
}

impl PartialEq for ToneCurveConfig {
    fn eq(&self, other: &Self) -> bool {
        self.payload() == other.payload()
    }
}

impl Eq for ToneCurveConfig {}

impl std::hash::Hash for ToneCurveConfig {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.payload().hash(state);
    }
}
