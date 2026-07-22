use std::fmt;
use std::path::Path;

use crate::{DecodeError, DecodeReceipt, DecodeResult, DecodedFrame};
use crate::{ImageDescriptor, ImageDimensions, InputFormat};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeLimitsError {
    ZeroLimit,
    InconsistentPixelCount,
    InconsistentDecodedBytes,
    ArithmeticOverflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeLimits {
    source_bytes: u64,
    width: u32,
    height: u32,
    pixel_count: u64,
    decoded_bytes: u64,
}

impl DecodeLimits {
    /// Creates checked nonzero source, dimension, pixel, and decoded-byte caps.
    ///
    /// # Errors
    ///
    /// Returns an error when a cap is zero, when a cap cannot be represented,
    /// or when a pixel/byte cap exceeds the dimension envelope.
    pub fn new(
        max_source_bytes: u64,
        max_width: u32,
        max_height: u32,
        max_pixel_count: u64,
        max_decoded_bytes: u64,
    ) -> Result<Self, DecodeLimitsError> {
        if max_source_bytes == 0
            || max_width == 0
            || max_height == 0
            || max_pixel_count == 0
            || max_decoded_bytes == 0
        {
            return Err(DecodeLimitsError::ZeroLimit);
        }
        let max_dimensions = u64::from(max_width)
            .checked_mul(u64::from(max_height))
            .ok_or(DecodeLimitsError::ArithmeticOverflow)?;
        if max_pixel_count > max_dimensions {
            return Err(DecodeLimitsError::InconsistentPixelCount);
        }
        let max_bytes = max_pixel_count
            .checked_mul(4)
            .ok_or(DecodeLimitsError::ArithmeticOverflow)?;
        if max_decoded_bytes > max_bytes {
            return Err(DecodeLimitsError::InconsistentDecodedBytes);
        }
        Ok(Self {
            source_bytes: max_source_bytes,
            width: max_width,
            height: max_height,
            pixel_count: max_pixel_count,
            decoded_bytes: max_decoded_bytes,
        })
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn max_width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn max_height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub const fn max_pixel_count(self) -> u64 {
        self.pixel_count
    }

    #[must_use]
    pub const fn max_decoded_bytes(self) -> u64 {
        self.decoded_bytes
    }

    /// Validates an arbitrary typed image descriptor against these limits.
    ///
    /// # Errors
    ///
    /// Returns a typed limit error when dimensions, pixel count, or all-plane
    /// byte storage exceeds the configured envelope.
    pub fn validate_descriptor(self, descriptor: &ImageDescriptor) -> Result<(), ImageInputError> {
        let dimensions = descriptor.dimensions();
        if dimensions.width() > self.max_width() {
            return Err(ImageInputError::WidthLimit {
                actual: dimensions.width(),
                limit: self.max_width(),
            });
        }
        if dimensions.height() > self.max_height() {
            return Err(ImageInputError::HeightLimit {
                actual: dimensions.height(),
                limit: self.max_height(),
            });
        }
        let pixels = dimensions
            .pixel_count()
            .map_err(|_| ImageInputError::ArithmeticOverflow)?;
        if pixels > self.max_pixel_count() {
            return Err(ImageInputError::PixelLimit {
                actual: pixels,
                limit: self.max_pixel_count(),
            });
        }
        let bytes = u64::try_from(descriptor.byte_length())
            .map_err(|_| ImageInputError::ArithmeticOverflow)?;
        if bytes > self.max_decoded_bytes() {
            return Err(ImageInputError::DecodedByteLimit {
                actual: bytes,
                limit: self.max_decoded_bytes(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageProbe {
    format: InputFormat,
    dimensions: ImageDimensions,
}

impl ImageProbe {
    #[must_use]
    pub const fn new(format: InputFormat, dimensions: ImageDimensions) -> Self {
        Self { format, dimensions }
    }

    #[must_use]
    pub const fn format(self) -> InputFormat {
        self.format
    }

    #[must_use]
    pub const fn dimensions(self) -> ImageDimensions {
        self.dimensions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageInputError {
    SourceTooLarge {
        limit: u64,
        actual: u64,
    },
    Io {
        message: String,
    },
    UnsupportedSignature {
        signature: Vec<u8>,
    },
    ProbeBudgetExceeded {
        limit: u64,
    },
    UnsupportedFeature {
        format: InputFormat,
        reason: UnsupportedImageFeature,
    },
    MalformedInput {
        format: InputFormat,
        message: String,
    },
    WidthLimit {
        actual: u32,
        limit: u32,
    },
    HeightLimit {
        actual: u32,
        limit: u32,
    },
    PixelLimit {
        actual: u64,
        limit: u64,
    },
    DecodedByteLimit {
        actual: u64,
        limit: u64,
    },
    ArithmeticOverflow,
    AllocationFailure,
    DecodedBufferInvariant {
        expected: u64,
        actual: u64,
    },
    DecodeContract {
        reason: DecodeError,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsupportedImageFeature {
    BigTiff,
    Animation,
    MultipleImages,
    BitDepth,
    SampleFormat,
    ColorModel,
    PlanarConfiguration,
    ArithmeticCoding,
    CodingProcess,
    Region,
    Sampling,
    RestartInterval,
}

impl fmt::Display for ImageInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceTooLarge { limit, actual } => {
                write!(formatter, "source is {actual} bytes, limit is {limit}")
            }
            Self::Io { message } => write!(formatter, "image input I/O failed: {message}"),
            Self::UnsupportedSignature { .. } => {
                formatter.write_str("image signature is unsupported")
            }
            Self::ProbeBudgetExceeded { limit } => {
                write!(formatter, "image probe exceeded the {limit}-byte budget")
            }
            Self::UnsupportedFeature { format, reason } => {
                write!(
                    formatter,
                    "unsupported {format:?} image feature: {reason:?}"
                )
            }
            Self::MalformedInput { format, message } => {
                write!(formatter, "malformed {format:?} input: {message}")
            }
            Self::WidthLimit { actual, limit } => {
                write!(formatter, "image width {actual} exceeds limit {limit}")
            }
            Self::HeightLimit { actual, limit } => {
                write!(formatter, "image height {actual} exceeds limit {limit}")
            }
            Self::PixelLimit { actual, limit } => write!(
                formatter,
                "image pixel count {actual} exceeds limit {limit}"
            ),
            Self::DecodedByteLimit { actual, limit } => {
                write!(formatter, "decoded bytes {actual} exceed limit {limit}")
            }
            Self::ArithmeticOverflow => formatter.write_str("image arithmetic overflowed"),
            Self::AllocationFailure => formatter.write_str("image allocation failed"),
            Self::DecodedBufferInvariant { expected, actual } => write!(
                formatter,
                "decoded buffer has {actual} bytes, expected {expected}"
            ),
            Self::DecodeContract { reason } => {
                write!(formatter, "decode contract failed: {reason}")
            }
        }
    }
}

impl std::error::Error for ImageInputError {}

pub trait ImageInput: Send + Sync {
    /// Probes one already-owned byte snapshot using signature dispatch.
    ///
    /// # Errors
    ///
    /// Returns a typed error for unsupported signatures, malformed bytes, or limits.
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError>;

    /// Decodes one already-owned byte snapshot into checked RGBA8 samples.
    ///
    /// # Errors
    ///
    /// Returns a typed error for unsupported signatures, malformed bytes, or limits.
    fn decode_bytes(&self, bytes: &[u8]) -> Result<crate::DecodedImage, ImageInputError>;

    /// Decodes one source into the native typed frame contract.
    ///
    /// Implementations with a native decoder should override this method. The
    /// default keeps older image inputs source-compatible by making their
    /// explicit RGBA8 projection visible at this boundary.
    ///
    /// # Errors
    ///
    /// Returns the underlying probe, decode, allocation, or contract error.
    fn decode_frame_bytes(&self, bytes: &[u8]) -> Result<DecodedFrame, ImageInputError> {
        let probe = self.probe_bytes(bytes)?;
        let image = self.decode_bytes(bytes)?;
        let source_bytes =
            u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
        let owned = image.into_owned();
        let receipt = DecodeReceipt::new(probe.format(), source_bytes, owned.descriptor().clone())
            .map_err(|reason| ImageInputError::DecodeContract { reason })?;
        DecodedFrame::new(owned, receipt)
            .map_err(|reason| ImageInputError::DecodeContract { reason })
    }

    /// Decodes bytes and publishes the storage-neutral owned image plus a
    /// receipt binding it to the probed format and source length.
    ///
    /// # Errors
    ///
    /// Returns the underlying input failure or a typed contract failure.
    fn decode_result_bytes(&self, bytes: &[u8]) -> Result<DecodeResult, ImageInputError> {
        self.decode_frame_bytes(bytes)
    }

    /// Probes a path using the bounded, signature-selected input contract.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for I/O, unsupported, malformed, or limit
    /// failures.
    fn probe_path(&self, path: &Path) -> Result<ImageProbe, ImageInputError>;

    /// Decodes a path into checked packed RGBA8 samples.
    ///
    /// # Errors
    ///
    /// Returns a typed input error for I/O, unsupported, malformed, limit, or
    /// decoded-buffer failures.
    fn decode_path(&self, path: &Path) -> Result<crate::DecodedImage, ImageInputError>;
}
