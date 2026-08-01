//! Bounded CPU leaf port of Darktable's `src/iop/levels.c`.
//!
//! Source lineage is the retained `src/iop/levels.c` and its directly coupled
//! `data/kernels/basic.cl` `levels` kernel. This leaf keeps the native v1/v2
//! parameter ABI, histogram thresholds, Lab-lightness equation, lookup-table
//! construction, and pointwise four-channel execution together. Registry,
//! typed history import, evaluator, and pixelpipe CPU routing are integrated;
//! GPU binding, imported masks/outer blending, automatic histogram/GUI
//! synchronization, and GTK controls remain explicitly deferred.

#![forbid(unsafe_code)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::unused_self,
    reason = "the source contract fixes native f32 histogram and raster arithmetic"
)]

mod codec;
mod descriptor;
mod execution;

#[allow(unused_imports)]
pub use codec::{
    LEVELS_BLACK_DEFAULT, LEVELS_BLACK_MAXIMUM, LEVELS_BLACK_MINIMUM, LEVELS_CHANNELS,
    LEVELS_COMPATIBILITY_ID, LEVELS_DEFAULT_GRAY, LEVELS_DEFAULT_LEVELS, LEVELS_GRAY_MAXIMUM,
    LEVELS_GRAY_MINIMUM, LEVELS_PARAMETER_BYTES_V1, LEVELS_PARAMETER_BYTES_V2, LEVELS_RUST_ID,
    LEVELS_SCHEMA_VERSION, LEVELS_WHITE_DEFAULT, LEVELS_WHITE_MAXIMUM, LEVELS_WHITE_MINIMUM,
    LevelsCodecError, LevelsConfig, LevelsHistory, LevelsMode, LevelsParameterError,
    LevelsParametersV1, LevelsParametersV2, migrate_v1_to_v2,
};
pub use descriptor::levels_descriptor;
#[allow(unused_imports)]
pub use execution::{
    LEVELS_AUTO_HISTOGRAM_BINS, LEVELS_INPUT_PROFILE, LEVELS_LUMINANCE_CHANNEL,
    LEVELS_LUMINANCE_SCALE, LEVELS_LUT_ENTRIES, LEVELS_MANUAL_HISTOGRAM_BINS,
    LEVELS_MAXIMUM_LUT_BYTES, LevelsHistogram, LevelsHistogramError, LevelsPixel, LevelsPlan,
    LevelsTiling, compute_automatic_levels, compute_manual_levels,
};
