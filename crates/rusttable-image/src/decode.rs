use std::fmt;
use std::num::NonZeroU64;

use crate::{
    BufferAllocationError, BufferPool, CfaDescriptor, ChannelLayout, ColorEncodingReference,
    DecodeLimits, ExifOrientation, ImageDimensions, InputFormat, OwnedPlane, PixelFormat,
    PlaneError, SampleType,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrientationHandling {
    Preserve,
    Logical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeRequest {
    limits: DecodeLimits,
    orientation: OrientationHandling,
    requested_format: Option<PixelFormat>,
}

impl DecodeRequest {
    #[must_use]
    pub const fn new(limits: DecodeLimits) -> Self {
        Self {
            limits,
            orientation: OrientationHandling::Logical,
            requested_format: None,
        }
    }

    #[must_use]
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    #[must_use]
    pub const fn orientation(&self) -> OrientationHandling {
        self.orientation
    }

    #[must_use]
    pub const fn requested_format(&self) -> Option<PixelFormat> {
        self.requested_format
    }

    #[must_use]
    pub const fn with_orientation(mut self, orientation: OrientationHandling) -> Self {
        self.orientation = orientation;
        self
    }

    #[must_use]
    pub const fn with_requested_format(mut self, format: PixelFormat) -> Self {
        self.requested_format = Some(format);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    dimensions: ImageDimensions,
    planes: Vec<OwnedPlane>,
    color_encoding: ColorEncodingReference,
    orientation: ExifOrientation,
    cfa: Option<CfaDescriptor>,
}

pub type ImageFrame = DecodedFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    NoPlanes,
    DimensionsMismatch,
    CfaNotAllowed,
}

impl DecodedFrame {
    /// Builds a frame after validating all plane and CFA relationships.
    ///
    /// # Errors
    ///
    /// Returns an error for an empty frame, mismatched plane dimensions, or
    /// CFA metadata attached to more than one plane.
    pub fn new(
        dimensions: ImageDimensions,
        planes: Vec<OwnedPlane>,
        color_encoding: ColorEncodingReference,
        orientation: ExifOrientation,
        cfa: Option<CfaDescriptor>,
    ) -> Result<Self, FrameError> {
        if planes.is_empty() {
            return Err(FrameError::NoPlanes);
        }
        if planes
            .iter()
            .any(|plane| plane.descriptor().dimensions() != dimensions)
        {
            return Err(FrameError::DimensionsMismatch);
        }
        if cfa.is_some_and(|_| planes.len() != 1) {
            return Err(FrameError::CfaNotAllowed);
        }
        Ok(Self {
            dimensions,
            planes,
            color_encoding,
            orientation,
            cfa,
        })
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn planes(&self) -> &[OwnedPlane] {
        &self.planes
    }

    #[must_use]
    pub const fn color_encoding(&self) -> ColorEncodingReference {
        self.color_encoding
    }

    #[must_use]
    pub const fn orientation(&self) -> ExifOrientation {
        self.orientation
    }

    #[must_use]
    pub const fn cfa(&self) -> Option<CfaDescriptor> {
        self.cfa
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeReceipt {
    format: InputFormat,
    source_bytes: NonZeroU64,
    dimensions: ImageDimensions,
    plane_count: u16,
    orientation: ExifOrientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptError {
    ZeroSourceBytes,
    ZeroPlanes,
}

impl DecodeReceipt {
    /// Creates a receipt containing only checked decode facts.
    ///
    /// # Errors
    ///
    /// Returns an error for empty source data or an invalid plane count.
    pub fn new(
        format: InputFormat,
        source_bytes: u64,
        dimensions: ImageDimensions,
        plane_count: usize,
        orientation: ExifOrientation,
    ) -> Result<Self, ReceiptError> {
        let source_bytes = NonZeroU64::new(source_bytes).ok_or(ReceiptError::ZeroSourceBytes)?;
        let plane_count = u16::try_from(plane_count).map_err(|_| ReceiptError::ZeroPlanes)?;
        if plane_count == 0 {
            return Err(ReceiptError::ZeroPlanes);
        }
        Ok(Self {
            format,
            source_bytes,
            dimensions,
            plane_count,
            orientation,
        })
    }

    #[must_use]
    pub const fn format(&self) -> InputFormat {
        self.format
    }

    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes.get()
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub const fn plane_count(&self) -> u16 {
        self.plane_count
    }

    #[must_use]
    pub const fn orientation(&self) -> ExifOrientation {
        self.orientation
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeResult {
    frame: DecodedFrame,
    receipt: DecodeReceipt,
}

impl DecodeResult {
    /// Combines a decoded frame with a matching receipt.
    ///
    /// # Errors
    ///
    /// Returns an error when receipt dimensions or plane count differ.
    pub fn new(frame: DecodedFrame, receipt: DecodeReceipt) -> Result<Self, DecodeError> {
        if frame.dimensions() != receipt.dimensions()
            || frame.planes().len() != usize::from(receipt.plane_count())
        {
            return Err(DecodeError::ReceiptMismatch);
        }
        Ok(Self { frame, receipt })
    }

    #[must_use]
    pub const fn frame(&self) -> &DecodedFrame {
        &self.frame
    }

    #[must_use]
    pub const fn receipt(&self) -> &DecodeReceipt {
        &self.receipt
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderCapabilities {
    formats: Vec<InputFormat>,
    sample_types: Vec<SampleType>,
    layouts: Vec<ChannelLayout>,
    applies_orientation_logically: bool,
    preserves_cfa: bool,
}

impl DecoderCapabilities {
    #[must_use]
    pub fn new(
        formats: Vec<InputFormat>,
        sample_types: Vec<SampleType>,
        layouts: Vec<ChannelLayout>,
        applies_orientation_logically: bool,
        preserves_cfa: bool,
    ) -> Self {
        Self {
            formats,
            sample_types,
            layouts,
            applies_orientation_logically,
            preserves_cfa,
        }
    }

    #[must_use]
    pub fn formats(&self) -> &[InputFormat] {
        &self.formats
    }

    #[must_use]
    pub fn sample_types(&self) -> &[SampleType] {
        &self.sample_types
    }

    #[must_use]
    pub fn layouts(&self) -> &[ChannelLayout] {
        &self.layouts
    }

    #[must_use]
    pub const fn applies_orientation_logically(&self) -> bool {
        self.applies_orientation_logically
    }

    #[must_use]
    pub const fn preserves_cfa(&self) -> bool {
        self.preserves_cfa
    }
}

pub trait Decoder: Send + Sync {
    fn capabilities(&self) -> &DecoderCapabilities;

    /// Decodes an owned source snapshot according to a typed request.
    ///
    /// # Errors
    ///
    /// Returns a typed decode, validation, limit, or implementation error.
    fn decode(&self, request: &DecodeRequest, source: &[u8]) -> Result<DecodeResult, DecodeError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    EmptySource,
    UnsupportedFormat(InputFormat),
    InvalidRequest,
    LimitsExceeded,
    Frame(FrameError),
    Plane(PlaneError),
    Allocation(BufferAllocationError),
    ReceiptMismatch,
    Implementation(String),
}

/// Allocates a canonical processing buffer through the shared pool boundary.
///
/// # Errors
///
/// Returns an allocation error from the supplied pool.
pub fn allocate_canonical(
    pool: &impl BufferPool,
    dimensions: ImageDimensions,
) -> Result<crate::CanonicalRgbaBuffer, DecodeError> {
    pool.allocate_canonical(dimensions)
        .map_err(DecodeError::Allocation)
}

impl fmt::Display for FrameError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoPlanes => "decoded frame must contain a plane",
            Self::DimensionsMismatch => "decoded planes have inconsistent dimensions",
            Self::CfaNotAllowed => "CFA metadata requires exactly one plane",
        })
    }
}

impl std::error::Error for FrameError {}

impl fmt::Display for ReceiptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroSourceBytes => "decode receipt source size must be nonzero",
            Self::ZeroPlanes => "decode receipt plane count must be nonzero and fit u16",
        })
    }
}

impl std::error::Error for ReceiptError {}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptySource => formatter.write_str("decode source must be nonempty"),
            Self::UnsupportedFormat(format) => {
                write!(formatter, "unsupported input format: {format:?}")
            }
            Self::InvalidRequest => formatter.write_str("decode request is invalid"),
            Self::LimitsExceeded => formatter.write_str("decode limits were exceeded"),
            Self::Frame(error) => error.fmt(formatter),
            Self::Plane(error) => error.fmt(formatter),
            Self::Allocation(error) => error.fmt(formatter),
            Self::ReceiptMismatch => formatter.write_str("decode receipt does not match frame"),
            Self::Implementation(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for DecodeError {}
