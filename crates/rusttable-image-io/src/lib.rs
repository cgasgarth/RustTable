#![forbid(unsafe_code)]
#![doc = "Explicit JPEG and PNG file input for `RustTable`."]

mod input;
mod output;

pub use input::FileImageInput;
pub use output::FileImageOutput;
