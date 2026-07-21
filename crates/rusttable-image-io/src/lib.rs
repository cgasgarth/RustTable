#![forbid(unsafe_code)]
#![doc = "Bounded JPEG, PNG, and classic-TIFF file input/output for `RustTable`."]

mod input;
mod output;
mod raw;
mod registry;

pub mod dng_output;

pub use input::FileImageInput;
pub use output::FileImageOutput;
pub use registry::{ImageDecoderRegistry, PROBE_BUDGET_BYTES, ProbeOutcome};
pub use rusttable_image::{DecoderDescriptor, DecoderIdentity};
