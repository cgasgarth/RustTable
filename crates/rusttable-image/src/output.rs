use std::fmt;
use std::num::{NonZeroU8, NonZeroU64};
use std::path::{Path, PathBuf};

use crate::{DecodedImage, ImageDimensions};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OutputFormat {
    Png,
    Jpeg,
    JpegXl,
    Tiff,
    Webp,
    Avif,
    Heif,
    Heic,
}

pub const SUPPORTED_OUTPUT_FORMATS: [OutputFormat; 8] = [
    OutputFormat::Png,
    OutputFormat::Jpeg,
    OutputFormat::JpegXl,
    OutputFormat::Tiff,
    OutputFormat::Webp,
    OutputFormat::Avif,
    OutputFormat::Heif,
    OutputFormat::Heic,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegQualityError {
    OutOfRange { value: u8 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JpegQuality(NonZeroU8);

impl JpegQuality {
    /// Creates an integer JPEG quality in the inclusive range `1..=100`.
    ///
    /// # Errors
    ///
    /// Returns [`JpegQualityError::OutOfRange`] for every other value.
    pub fn new(value: u8) -> Result<Self, JpegQualityError> {
        if !(1..=100).contains(&value) {
            return Err(JpegQualityError::OutOfRange { value });
        }
        Ok(Self(
            NonZeroU8::new(value).ok_or(JpegQualityError::OutOfRange { value })?,
        ))
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputLimitsError {
    ZeroEncodedBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OutputLimits(NonZeroU64);

impl OutputLimits {
    /// Creates a nonzero maximum encoded-output byte limit.
    ///
    /// # Errors
    ///
    /// Returns [`OutputLimitsError::ZeroEncodedBytes`] for zero.
    pub fn new(max_encoded_bytes: u64) -> Result<Self, OutputLimitsError> {
        NonZeroU64::new(max_encoded_bytes)
            .map(Self)
            .ok_or(OutputLimitsError::ZeroEncodedBytes)
    }

    #[must_use]
    pub const fn max_encoded_bytes(self) -> u64 {
        self.0.get()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OutputOptions {
    Png,
    Jpeg {
        quality: JpegQuality,
    },
    /// Writes one uncompressed classic TIFF image with RGBA8 straight-alpha samples.
    Tiff,
}

impl OutputOptions {
    #[must_use]
    pub const fn format(self) -> OutputFormat {
        match self {
            Self::Png => OutputFormat::Png,
            Self::Jpeg { .. } => OutputFormat::Jpeg,
            Self::Tiff => OutputFormat::Tiff,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputReceipt {
    destination: PathBuf,
    format: OutputFormat,
    dimensions: ImageDimensions,
    encoded_byte_length: NonZeroU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputReceiptError {
    ZeroEncodedBytes,
}

impl OutputReceipt {
    /// Creates a receipt for a completed nonempty output publication.
    ///
    /// # Errors
    ///
    /// Returns [`OutputReceiptError::ZeroEncodedBytes`] for an empty output.
    pub fn new(
        destination: PathBuf,
        format: OutputFormat,
        dimensions: ImageDimensions,
        encoded_byte_length: u64,
    ) -> Result<Self, OutputReceiptError> {
        let encoded_byte_length =
            NonZeroU64::new(encoded_byte_length).ok_or(OutputReceiptError::ZeroEncodedBytes)?;
        Ok(Self {
            destination,
            format,
            dimensions,
            encoded_byte_length,
        })
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.format
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn encoded_byte_length(&self) -> u64 {
        self.encoded_byte_length.get()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageOutputError {
    InvalidDestination { path: PathBuf },
    MissingDestinationParent { path: PathBuf },
    DestinationExists { path: PathBuf },
    NonOpaqueJpegInput { pixel_index: u64 },
    EncodedOutputTooLarge { limit: u64, actual: u64 },
    AllocationFailure,
    EncodeFailure { format: OutputFormat },
    TemporaryFileCreationFailure,
    WriteFailure,
    SyncFailure,
    PublishFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableOutputStage {
    DestinationValidation,
    DirectoryCapability,
    Encoding,
    TemporaryCreation,
    Write,
    FileSync,
    Publication,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurableOutputTag {
    FileAndParentDirectorySynchronized,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableOutputReceipt {
    receipt: OutputReceipt,
    tag: DurableOutputTag,
}

impl DurableOutputReceipt {
    #[doc(hidden)]
    #[must_use]
    pub fn new(receipt: OutputReceipt) -> Self {
        Self {
            receipt,
            tag: DurableOutputTag::FileAndParentDirectorySynchronized,
        }
    }

    #[must_use]
    pub fn output(&self) -> &OutputReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        self.receipt.destination()
    }

    #[must_use]
    pub const fn format(&self) -> OutputFormat {
        self.receipt.format()
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.receipt.dimensions()
    }

    #[must_use]
    pub const fn encoded_byte_length(&self) -> u64 {
        self.receipt.encoded_byte_length()
    }

    #[must_use]
    pub const fn durability(&self) -> DurableOutputTag {
        self.tag
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableImageOutputError {
    BeforePublication {
        source: ImageOutputError,
    },
    DurabilityUnsupported {
        destination: PathBuf,
    },
    BeforePublicationCleanupFailure {
        destination: PathBuf,
        stage: DurableOutputStage,
    },
    PublishedTemporaryCleanupFailure {
        receipt: OutputReceipt,
    },
    PublishedDirectorySyncFailure {
        receipt: OutputReceipt,
    },
}

impl fmt::Display for JpegQualityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange { value } => {
                write!(formatter, "JPEG quality {value} is outside 1..=100")
            }
        }
    }
}
impl std::error::Error for JpegQualityError {}

impl fmt::Display for OutputLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("encoded output limit must be nonzero")
    }
}
impl std::error::Error for OutputLimitsError {}

impl fmt::Display for OutputReceiptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("encoded output length must be nonzero")
    }
}
impl std::error::Error for OutputReceiptError {}

impl fmt::Display for ImageOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDestination { path } => {
                write!(formatter, "invalid output destination: {}", path.display())
            }
            Self::MissingDestinationParent { path } => {
                write!(formatter, "output parent is missing: {}", path.display())
            }
            Self::DestinationExists { path } => write!(
                formatter,
                "output destination already exists: {}",
                path.display()
            ),
            Self::NonOpaqueJpegInput { pixel_index } => {
                write!(formatter, "JPEG input pixel {pixel_index} is not opaque")
            }
            Self::EncodedOutputTooLarge { limit, actual } => write!(
                formatter,
                "encoded output is {actual} bytes, limit is {limit}"
            ),
            Self::AllocationFailure => formatter.write_str("output allocation failed"),
            Self::EncodeFailure { format } => write!(formatter, "{format:?} encoding failed"),
            Self::TemporaryFileCreationFailure => {
                formatter.write_str("temporary output creation failed")
            }
            Self::WriteFailure => formatter.write_str("output write failed"),
            Self::SyncFailure => formatter.write_str("output sync failed"),
            Self::PublishFailure => formatter.write_str("output publication failed"),
        }
    }
}
impl std::error::Error for ImageOutputError {}

impl fmt::Display for DurableImageOutputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "durable output failed: {self:?}")
    }
}

impl std::error::Error for DurableImageOutputError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BeforePublication { source } => Some(source),
            Self::DurabilityUnsupported { .. }
            | Self::BeforePublicationCleanupFailure { .. }
            | Self::PublishedTemporaryCleanupFailure { .. }
            | Self::PublishedDirectorySyncFailure { .. } => None,
        }
    }
}

pub trait ImageOutput {
    /// Publishes one new output without consulting or replacing the destination.
    ///
    /// Output options, not the destination extension, select the format. No
    /// metadata, orientation, color transform, or crash-durability guarantee
    /// is part of this portable boundary.
    ///
    /// # Errors
    ///
    /// Returns a typed output error; a normally returned error must not leave
    /// adapter-owned temporary state behind.
    fn write_new(
        &self,
        image: &DecodedImage,
        destination: &Path,
        options: OutputOptions,
    ) -> Result<OutputReceipt, ImageOutputError>;
}

pub trait DurableImageOutput {
    /// Publishes one new output and confirms file plus parent-directory sync.
    ///
    /// # Errors
    ///
    /// Returns a typed state-aware error. A published failure carries the
    /// complete final-output receipt and never removes that final file.
    fn write_new_durable(
        &self,
        image: &DecodedImage,
        destination: &Path,
        options: OutputOptions,
    ) -> Result<DurableOutputReceipt, DurableImageOutputError>;
}
