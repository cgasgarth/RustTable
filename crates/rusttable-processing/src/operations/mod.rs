//! Darktable-compatible image operations implemented at the Rust processing boundary.
//!
//! The modules in this directory own parameter migrations, checked planning,
//! deterministic scalar execution, and diagnostic receipts.  They are called
//! by the existing operation registry and pixelpipe; they are not a second
//! pipeline.

pub mod agx;
pub mod basecurve;
pub mod basicadj;
pub mod bloom;
pub mod borders;
pub mod censorize;
pub mod channelmixer;
pub mod clahe;
pub mod clipping;
pub mod colisa;
pub mod colorcontrast;
pub mod colorcorrection;
pub mod colorharmonizer;
pub mod colorin;
pub mod colormapping;
pub mod colorout;
pub mod colorreconstruction;
pub mod colortransfer;
pub mod colorzones;
pub(crate) mod common;
pub mod convolution;
pub mod crop;
pub mod defringe;
pub mod dither;
pub mod enlargecanvas;
pub mod finalscale;
pub mod flip;
pub mod graduatednd;
pub mod grain;
pub mod highlights;
pub mod highpass;
pub mod invert;
pub mod lenscorrection;
pub mod levels;
pub mod liquify;
pub mod mask_manager;
pub mod overlay;
pub mod perspective;
pub mod primaries;
pub mod rasterfile;
pub mod rawprepare;
pub mod relight;
pub mod retouch;
pub mod rgblevels;
pub mod rotatepixels;
pub mod scalepixels;
pub mod shadhi;
pub mod sharpen;
pub mod soften;
pub mod spots;
pub mod temperature;
pub mod tonecurve;
pub mod velvia;
pub mod vibrance;
pub mod vignette;
pub mod watermark;

pub use common::{
    OperationExecutionError, ReconstructionBudget, ReconstructionDiagnostics, ReconstructionReceipt,
};
