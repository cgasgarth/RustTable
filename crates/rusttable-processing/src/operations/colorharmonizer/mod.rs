//! Bounded, unregistered Color Harmonizer CPU leaf.
//!
//! Direct source lineage:
//! - `src/iop/colorharmonizer.c`
//! - `src/common/color_harmony.h`
//! - `src/common/color_ryb.h`
//! - `src/common/chromatic_adaptation.h`
//! - `src/common/colorspaces_inline_conversions.h`
//! - `src/common/gaussian.c` and `src/common/gaussian.h`
//!
//! This module is intentionally not added to the shared operation registry or
//! evaluator.  It owns exact parameter/CPU math only; the operation remains
//! unavailable until an integration owner supplies profile acquisition,
//! history routing, full-frame scheduling, and the production capability seam.

#![forbid(unsafe_code)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::too_many_arguments,
    reason = "the source-shaped compatibility boundary keeps explicit native field order"
)]

mod codec;
mod descriptor;
mod execution;
mod ucs;

#[cfg(test)]
mod tests;

pub use codec::{
    COLORHARMONIZER_DEFAULT_ANCHOR_HUE, COLORHARMONIZER_DEFAULT_NEUTRAL_PROTECTION,
    COLORHARMONIZER_DEFAULT_NUM_CUSTOM_NODES, COLORHARMONIZER_DEFAULT_PULL_STRENGTH,
    COLORHARMONIZER_DEFAULT_PULL_WIDTH, COLORHARMONIZER_DEFAULT_SMOOTHING,
    COLORHARMONIZER_MAX_NODES, COLORHARMONIZER_PARAMETER_BYTES, COLORHARMONIZER_RYB_INVERSE_STEPS,
    COLORHARMONIZER_SCHEMA_VERSION, ColorHarmonizerCodecError, ColorHarmonizerHistory,
    ColorHarmonizerParametersV1, ColorHarmonizerRule,
};
pub use descriptor::{
    COLORHARMONIZER_COMPATIBILITY_ID, COLORHARMONIZER_GPU_AVAILABLE, COLORHARMONIZER_REGISTERED,
    COLORHARMONIZER_RUST_ID, COLORHARMONIZER_UI_AVAILABLE, colorharmonizer_descriptor,
};
pub use execution::{
    ColorHarmonizerConfig, ColorHarmonizerExecutionError, ColorHarmonizerPlan, FrameDimensions,
    WeightedHueShift, WorkingProfileMatrices, smoothing_sigma, weighted_hue_shift, wrap_hue,
};
pub use ucs::{
    HarmonyTables, harmony_nodes, harmony_tables, hue_lerp, jch_to_srgb, jch_to_xyy, l_star_to_y,
    rgb_hue_to_ryb_hue, xyy_to_jch, xyy_to_xyz, xyz_d65_to_xyy, y_to_l_star,
};
