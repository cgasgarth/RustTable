#![forbid(unsafe_code)]
#![doc = "Bounded JPEG, PNG, and classic-TIFF file input/output for `RustTable`."]

mod input;
mod output;
mod registry;

pub use input::FileImageInput;
pub use output::FileImageOutput;
pub use registry::ImageDecoderRegistry;
