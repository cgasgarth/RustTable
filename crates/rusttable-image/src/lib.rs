#![forbid(unsafe_code)]
#![doc = "Codec-neutral image input contracts for `RustTable`."]

mod buffer;
mod cfa;
mod decode;
mod format;
mod image;
mod input;
mod orientation;
mod output;
mod pixel;
mod plane;
mod roi;

pub use buffer::{BufferAllocationError, BufferPool, CanonicalRgbaBuffer, DefaultBufferPool};
pub use cfa::{
    BayerPattern, BlackWhiteLevels, CfaColor, CfaDescriptor, CfaError, CfaPhase, OrientedCfa,
    RawMosaic, XTransPattern,
};
pub use decode::{
    DecodeError, DecodeReceipt, DecodeRequest, DecodeResult, DecodedFrame, Decoder,
    DecoderCapabilities, FrameError, ImageFrame, OrientationHandling, ReceiptError,
    allocate_canonical,
};
pub use format::{InputFormat, SUPPORTED_INPUT_FORMATS};
pub use image::{
    ColorEncoding, DecodedImage, DecodedImageError, ImageDimensions, ImageDimensionsError,
    PixelLayout,
};
pub use input::{
    DecodeLimits, DecodeLimitsError, ImageInput, ImageInputError, ImageProbe,
    UnsupportedImageFeature,
};
pub use orientation::{Coordinate, ExifOrientation, OrientationError, OrientationTransform};
pub use output::{
    DurableImageOutput, DurableImageOutputError, DurableOutputReceipt, DurableOutputStage,
    DurableOutputTag, ImageOutput, ImageOutputError, JpegQuality, JpegQualityError, OutputFormat,
    OutputLimits, OutputLimitsError, OutputOptions, OutputReceipt, OutputReceiptError,
    SUPPORTED_OUTPUT_FORMATS,
};
pub use pixel::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncodingRef, ColorEncodingReference, PixelFormat,
    PixelFormatError, SampleType, StorageLayout,
};
pub use plane::{OwnedPlane, PlaneDescriptor, PlaneError, PlaneRows, PlaneView, PlaneViewMut};
pub use roi::{ImageRect, ImageRectError};
