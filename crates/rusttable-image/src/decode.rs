use crate::{
    ChannelLayout, DecodeLimits, ImageDescriptor, InputFormat, Orientation, OwnedImage,
    PixelFormat, Roi, SampleType,
};

/// A request independent of any particular decoder implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeRequest {
    format: Option<InputFormat>,
    roi: Option<Roi>,
    orientation: Orientation,
    output_format: Option<PixelFormat>,
    limits: DecodeLimits,
}

impl DecodeRequest {
    #[must_use]
    pub const fn new(limits: DecodeLimits) -> Self {
        Self {
            format: None,
            roi: None,
            orientation: Orientation::Normal,
            output_format: None,
            limits,
        }
    }

    #[must_use]
    pub const fn format(&self) -> Option<InputFormat> {
        self.format
    }

    #[must_use]
    pub const fn roi(&self) -> Option<Roi> {
        self.roi
    }

    #[must_use]
    pub const fn orientation(&self) -> Orientation {
        self.orientation
    }

    #[must_use]
    pub const fn output_format(&self) -> Option<PixelFormat> {
        self.output_format
    }

    #[must_use]
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    #[must_use]
    pub const fn with_format(mut self, format: InputFormat) -> Self {
        self.format = Some(format);
        self
    }

    #[must_use]
    pub const fn with_roi(mut self, roi: Roi) -> Self {
        self.roi = Some(roi);
        self
    }

    #[must_use]
    pub const fn with_orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    #[must_use]
    pub const fn with_output_format(mut self, format: PixelFormat) -> Self {
        self.output_format = Some(format);
        self
    }
}

/// The stable evidence returned with a successful decode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeReceipt {
    format: InputFormat,
    source_bytes: u64,
    descriptor: ImageDescriptor,
}

impl DecodeReceipt {
    /// Creates evidence for a non-empty source decode.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::EmptySource`] for zero source bytes.
    pub fn new(
        format: InputFormat,
        source_bytes: u64,
        descriptor: ImageDescriptor,
    ) -> Result<Self, DecodeError> {
        if source_bytes == 0 {
            return Err(DecodeError::EmptySource);
        }
        Ok(Self {
            format,
            source_bytes,
            descriptor,
        })
    }

    #[must_use]
    pub const fn format(&self) -> InputFormat {
        self.format
    }

    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }

    #[must_use]
    pub const fn descriptor(&self) -> &ImageDescriptor {
        &self.descriptor
    }
}

/// A decoded owned image and the descriptor facts needed to audit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeResult {
    image: OwnedImage,
    receipt: DecodeReceipt,
}

impl DecodeResult {
    /// Pairs an owned image with matching decode evidence.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::ReceiptMismatch`] when descriptors differ.
    pub fn new(image: OwnedImage, receipt: DecodeReceipt) -> Result<Self, DecodeError> {
        if image.descriptor() != receipt.descriptor() {
            return Err(DecodeError::ReceiptMismatch);
        }
        Ok(Self { image, receipt })
    }

    #[must_use]
    pub const fn image(&self) -> &OwnedImage {
        &self.image
    }

    #[must_use]
    pub const fn receipt(&self) -> &DecodeReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn into_image(self) -> OwnedImage {
        self.image
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    EmptySource,
    ReceiptMismatch,
    InvalidRequest,
    UnsupportedFormat,
    UnsupportedSampleType,
    LimitExceeded,
    ArithmeticOverflow,
}

/// Format and output guarantees advertised by a decoder without registering it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecoderCapabilities {
    identity: DecoderIdentity,
    formats: Vec<InputFormat>,
    sample_types: Vec<SampleType>,
    layouts: Vec<ChannelLayout>,
    max_planes: usize,
}

impl DecoderCapabilities {
    /// Creates a non-empty capability declaration.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::InvalidRequest`] when any capability set is
    /// empty or no plane can be produced.
    pub fn new(
        identity: DecoderIdentity,
        formats: Vec<InputFormat>,
        sample_types: Vec<SampleType>,
        layouts: Vec<ChannelLayout>,
        max_planes: usize,
    ) -> Result<Self, DecodeError> {
        if formats.is_empty() || sample_types.is_empty() || layouts.is_empty() || max_planes == 0 {
            return Err(DecodeError::InvalidRequest);
        }
        Ok(Self {
            identity,
            formats,
            sample_types,
            layouts,
            max_planes,
        })
    }

    #[must_use]
    pub const fn identity(&self) -> DecoderIdentity {
        self.identity
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
    pub const fn max_planes(&self) -> usize {
        self.max_planes
    }
}

/// Stable identity for a decoder implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecoderIdentity {
    id: &'static str,
    version: u32,
    implementation: &'static str,
}

impl DecoderIdentity {
    #[must_use]
    pub const fn new(id: &'static str, version: u32, implementation: &'static str) -> Self {
        Self {
            id,
            version,
            implementation,
        }
    }

    #[must_use]
    pub const fn id(self) -> &'static str {
        self.id
    }

    #[must_use]
    pub const fn version(self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn implementation(self) -> &'static str {
        self.implementation
    }
}

/// A decoder identity paired with one advertised input format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DecoderDescriptor {
    identity: DecoderIdentity,
    format: InputFormat,
}

impl DecoderDescriptor {
    #[must_use]
    pub const fn new(identity: DecoderIdentity, format: InputFormat) -> Self {
        Self { identity, format }
    }

    #[must_use]
    pub const fn identity(self) -> DecoderIdentity {
        self.identity
    }

    #[must_use]
    pub const fn format(self) -> InputFormat {
        self.format
    }
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::EmptySource => "decode source is empty",
            Self::ReceiptMismatch => "decode receipt does not match image descriptor",
            Self::InvalidRequest => "decode request is invalid",
            Self::UnsupportedFormat => "requested decode format is unsupported",
            Self::UnsupportedSampleType => "requested decode sample type is unsupported",
            Self::LimitExceeded => "decode request exceeds configured limits",
            Self::ArithmeticOverflow => "decode arithmetic overflowed",
        })
    }
}

impl std::error::Error for DecodeError {}
