#![forbid(unsafe_code)]
#![doc = "Codec-neutral image input contracts for `RustTable`."]

mod format;
mod image;
mod input;

pub use format::{InputFormat, SUPPORTED_INPUT_FORMATS};
pub use image::{
    ColorEncoding, DecodedImage, DecodedImageError, ImageDimensions, ImageDimensionsError,
    PixelLayout,
};
pub use input::{DecodeLimits, DecodeLimitsError, ImageInput, ImageInputError, ImageProbe};
