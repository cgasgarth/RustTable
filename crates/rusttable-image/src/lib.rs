#![forbid(unsafe_code)]
#![doc = "Codec-neutral image input contracts for `RustTable`."]

mod format;
mod image;
mod input;
mod output;

pub use format::{InputFormat, SUPPORTED_INPUT_EXTENSIONS, SUPPORTED_INPUT_FORMATS};
pub use image::{
    ColorEncoding, DecodedImage, DecodedImageError, ImageDimensions, ImageDimensionsError,
    PixelLayout,
};
pub use input::{
    DecodeLimits, DecodeLimitsError, ImageInput, ImageInputError, ImageProbe,
    UnsupportedImageFeature,
};
pub use output::{
    DurableImageOutput, DurableImageOutputError, DurableOutputReceipt, DurableOutputStage,
    DurableOutputTag, ImageOutput, ImageOutputError, JpegQuality, JpegQualityError, OutputFormat,
    OutputLimits, OutputLimitsError, OutputOptions, OutputReceipt, OutputReceiptError,
    SUPPORTED_OUTPUT_FORMATS,
};
