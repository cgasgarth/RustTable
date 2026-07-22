#![forbid(unsafe_code)]
#![doc = "Codec-neutral image input contracts for `RustTable`."]

mod decode;
mod format;
mod geometry;
mod host_pool;
mod image;
mod input;
mod output;
mod pixel;
mod raw;
mod source_color;
mod view;

pub use decode::{
    DecodeError, DecodeReceipt, DecodeRequest, DecodeResult, DecodeStage, DecodedFrame,
    DecodedFrameError, DecoderCapabilities, DecoderDescriptor, DecoderIdentity,
};
pub use format::{InputFormat, SUPPORTED_INPUT_EXTENSIONS, SUPPORTED_INPUT_FORMATS};
pub use geometry::{Orientation, Roi, RoiError};
pub use host_pool::{
    AcquireOptions, AllocationClass, BufferAlignment, BufferLease, BufferRead, BufferRequest,
    BufferUsage, BufferWrite, CancellationToken, HostBufferPool, HostImageView, HostPoolError,
    InitializationPolicy, LeaseState, MIN_HOST_ALIGNMENT, PoolAccounting, PoolBudgets, PoolEvent,
    PriorityClass, ReturnReceipt, SharedBufferLease, ShutdownReport,
};
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
pub use pixel::{
    AlphaMode, ByteOrder, ChannelLayout, PixelFormat, PixelFormatError, SampleType, StorageLayout,
};
pub use raw::{
    BlackWhiteLevels, BlackWhiteLevelsError, CfaColor, CfaDescriptor, CfaPattern, CfaPhase,
    RawMosaic, RawMosaicError,
};
pub use source_color::{SourceColor, SourceColorError, SourceColorEvidence, SourceColorFallback};
pub use view::{
    AlignedRgbaF32, CanonicalRgbaF32, ImageDescriptor, ImageDescriptorError, ImageView,
    ImageViewError, ImageViewMut, OwnedImage, PlaneDescriptor,
};
