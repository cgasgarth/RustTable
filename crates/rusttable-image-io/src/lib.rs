#![forbid(unsafe_code)]
#![doc = "Bounded JPEG, PNG, and classic-TIFF file input/output for `RustTable`."]

mod input;
mod output;
mod raster_samples;
mod raw;
mod registry;
mod svg;

pub mod dng_output;

pub use input::FileImageInput;
pub use output::FileImageOutput;
pub use raster_samples::{DecodedRgbSamples, decode_png_rgb_samples};
pub use registry::{ImageDecoderRegistry, PROBE_BUDGET_BYTES, ProbeOutcome};
pub use rusttable_image::{DecoderDescriptor, DecoderIdentity};
pub use svg::{ManagedSvgAsset, SVG_SCHEMA_VERSION, SvgError, SvgLimits, SvgRaster};
