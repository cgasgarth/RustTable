//! Operation-local Tone Equalizer leaf ported from `src/iop/toneequal.c`.
//!
//! The leaf owns the native v2 ABI and v1 migration, radial-basis tone curve,
//! luminance estimators, source-shaped guided/EIGF filters, RGBA CPU
//! execution, cancellation/publication boundary, and a fail-closed whole-
//! raster tile contract. Registry, history routing, pixelpipe, GPU/OpenCL,
//! masks/blending, presets, and GTK integration remain deferred in their
//! owning hubs; no generic blur is substituted for the retained helpers.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::doc_markdown,
    clippy::enum_variant_names,
    clippy::float_cmp,
    clippy::large_stack_arrays,
    clippy::large_types_passed_by_value,
    clippy::manual_midpoint,
    clippy::many_single_char_names,
    clippy::needless_range_loop,
    clippy::similar_names,
    clippy::struct_excessive_bools,
    clippy::too_many_arguments,
    clippy::unused_self,
    unused_imports,
    dead_code,
    reason = "the source-shaped bounded leaf retains deferred integration symbols and f32 equations"
)]

mod codec;
mod execution;
mod filters;
mod math;
mod parameters;
pub mod source_map;

pub use codec::{ToneEqualizerCodecError, ToneEqualizerHistory};
pub use execution::{
    DEFAULT_COLORSPACE, DESCRIPTION, GPU_SUPPORTED, GTK_SUPPORTED, OPERATION_NAME,
    SUPPORTS_BLENDING, TILING_SUPPORTED, ToneEqualizerCapabilities, ToneEqualizerExecution,
    ToneEqualizerExecutionError, ToneEqualizerOutputMode, ToneEqualizerPixel, ToneEqualizerPlan,
    ToneEqualizerTile, ToneEqualizerTileContract, capabilities, channel_count,
    exposure_channel_count, lut_resolution,
};
pub use math::{CENTERS_OPS, CENTERS_PARAMS, compute_channel_gains, default_contrast_fulcrum};
pub use parameters::{
    BLENDING_DEFAULT, CHANNELS, CONTRAST_FULCRUM, DetailsFilter, LEGACY_V1_BYTES, LUT_ENTRIES,
    LUT_RESOLUTION, LuminanceMethod, MAX_EV, MIN_EV, MIN_FLOAT, PARAMETER_BYTES, PARAMETER_VERSION,
    PIXEL_CHANNELS, ParameterError, ToneEqualizerParametersV2,
};

/// Operation-local source fixture used by focused ABI tests.
pub const DEFAULT_V2_FIXTURE: &str = include_str!("fixtures/default_v2.txt");
