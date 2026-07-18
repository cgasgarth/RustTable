#![forbid(unsafe_code)]
#![doc = "Bounded JPEG, PNG, and classic-TIFF file input/output for `RustTable`."]

mod input;
mod output;

pub use input::FileImageInput;
pub use output::FileImageOutput;
