use crate::{
    AlphaMode, ByteOrder, ChannelLayout, DecodeLimits, ImageDescriptor, InputFormat, Orientation,
    OwnedImage, PixelFormat, Roi, SampleType, SourceColor, StorageLayout,
};
use sha2::{Digest, Sha256};

/// Explicit RAW development stages retained in decode evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DecodeStage {
    RawRescale,
    RawActiveAreaCrop,
    RawCfa,
    RawDemosaic,
    RawWhiteBalance,
    RawColorCalibration,
    RawDefaultCrop,
}

impl DecodeStage {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RawRescale => "raw.rescale",
            Self::RawActiveAreaCrop => "raw.active_area_crop",
            Self::RawCfa => "raw.cfa",
            Self::RawDemosaic => "raw.demosaic",
            Self::RawWhiteBalance => "raw.white_balance",
            Self::RawColorCalibration => "raw.color_calibration",
            Self::RawDefaultCrop => "raw.default_crop",
        }
    }
}

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
    source_color: SourceColor,
    processing_stages: Vec<DecodeStage>,
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
        let source_color = SourceColor::from_encoding(descriptor.color_encoding())
            .map_err(|_| DecodeError::InvalidSourceColor)?;
        Ok(Self {
            format,
            source_bytes,
            descriptor,
            source_color,
            processing_stages: Vec::new(),
        })
    }

    /// Creates decode evidence with the decoder's complete source-color decision.
    ///
    /// # Errors
    ///
    /// Returns an error for empty sources or a descriptor/color mismatch.
    pub fn new_with_source_color(
        format: InputFormat,
        source_bytes: u64,
        descriptor: ImageDescriptor,
        source_color: SourceColor,
    ) -> Result<Self, DecodeError> {
        if source_bytes == 0 {
            return Err(DecodeError::EmptySource);
        }
        if descriptor.color_encoding() != source_color.encoding() {
            return Err(DecodeError::InvalidSourceColor);
        }
        Ok(Self {
            format,
            source_bytes,
            descriptor,
            source_color,
            processing_stages: Vec::new(),
        })
    }

    /// Retains the explicit processing stages used to produce this frame.
    #[must_use]
    pub fn with_processing_stages(mut self, stages: impl IntoIterator<Item = DecodeStage>) -> Self {
        self.processing_stages = stages.into_iter().collect();
        self
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

    #[must_use]
    pub const fn source_color(&self) -> SourceColor {
        self.source_color
    }

    /// Returns the ordered, explicit processing stages used by the decoder.
    #[must_use]
    pub fn processing_stages(&self) -> &[DecodeStage] {
        &self.processing_stages
    }
}

/// A decoded owned image and the descriptor facts needed to audit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    image: OwnedImage,
    receipt: DecodeReceipt,
    embedded_icc: Option<Vec<u8>>,
}

impl DecodedFrame {
    /// Pairs an owned image with matching decode evidence.
    ///
    /// # Errors
    ///
    /// Returns [`DecodeError::ReceiptMismatch`] when descriptors differ.
    pub fn new(image: OwnedImage, receipt: DecodeReceipt) -> Result<Self, DecodeError> {
        if image.descriptor() != receipt.descriptor() {
            return Err(DecodeError::ReceiptMismatch);
        }
        Ok(Self {
            image,
            receipt,
            embedded_icc: None,
        })
    }

    /// Retains the exact embedded ICC payload after validating its identity.
    ///
    /// # Errors
    ///
    /// Returns an error when the source-color contract is not ICC-backed or
    /// the payload does not match the retained profile identity.
    pub fn with_embedded_icc(mut self, bytes: Vec<u8>) -> Result<Self, DecodeError> {
        let Some(profile) = self.receipt.source_color().profile() else {
            return Err(DecodeError::InvalidSourceColor);
        };
        if profile.size() != u64::try_from(bytes.len()).unwrap_or(u64::MAX)
            || profile.sha256() != <[u8; 32]>::from(Sha256::digest(&bytes))
        {
            return Err(DecodeError::InvalidSourceColor);
        }
        self.embedded_icc = Some(bytes);
        Ok(self)
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
    pub const fn source_color(&self) -> SourceColor {
        self.receipt.source_color()
    }

    #[must_use]
    pub fn embedded_icc(&self) -> Option<&[u8]> {
        self.embedded_icc.as_deref()
    }

    #[must_use]
    pub fn into_image(self) -> OwnedImage {
        self.image
    }

    /// Converts the declared native samples to finite RGBA f32 values.
    ///
    /// Integer samples are normalized by their declared code depth. Floating
    /// samples are copied as-is; this is the explicit bridge immediately
    /// before the first processing input, not a presentation conversion.
    ///
    /// # Errors
    ///
    /// Returns an error when the declared storage is not a supported
    /// interleaved native frame or contains non-finite floating samples.
    pub fn rgba_f32_pixels(&self) -> Result<Vec<[f32; 4]>, DecodedFrameError> {
        let descriptor = self.image.descriptor();
        let format = descriptor.format();
        if format.storage() != StorageLayout::Interleaved {
            return Err(DecodedFrameError::UnsupportedStorage);
        }
        if format.byte_order() != ByteOrder::Native {
            return Err(DecodedFrameError::UnsupportedByteOrder);
        }
        if format.channels().is_mosaic() {
            return Err(DecodedFrameError::UnsupportedChannels);
        }
        if !matches!(format.alpha(), AlphaMode::None | AlphaMode::Straight) {
            return Err(DecodedFrameError::UnsupportedAlpha);
        }
        let channels = format.channels().channels();
        let bytes = self.image.bytes();
        let sample_bytes = format.bytes_per_sample();
        let pixel_count = usize::try_from(
            descriptor
                .dimensions()
                .pixel_count()
                .map_err(|_| DecodedFrameError::ArithmeticOverflow)?,
        )
        .map_err(|_| DecodedFrameError::ArithmeticOverflow)?;
        let mut output = Vec::new();
        output
            .try_reserve_exact(pixel_count)
            .map_err(|_| DecodedFrameError::AllocationFailure)?;
        for pixel_index in 0..pixel_count {
            let mut values = [0.0; 4];
            for channel in 0..channels {
                let offset = pixel_index
                    .checked_mul(channels)
                    .and_then(|value| value.checked_add(channel))
                    .and_then(|value| value.checked_mul(sample_bytes))
                    .ok_or(DecodedFrameError::ArithmeticOverflow)?;
                let target = if channels == 4 && channel == 3 {
                    3
                } else {
                    channel.min(2)
                };
                values[target] = read_sample(
                    format.sample_type(),
                    bytes
                        .get(offset..offset + sample_bytes)
                        .ok_or(DecodedFrameError::BufferInvariant)?,
                )?;
                if channels == 2 && channel == 1 {
                    values[3] = values[1];
                }
            }
            if channels == 1 {
                values[1] = values[0];
                values[2] = values[0];
                values[3] = 1.0;
            } else if channels == 2 {
                values[1] = values[0];
                values[2] = values[0];
            } else if channels == 3 {
                values[3] = 1.0;
            }
            if !values.iter().all(|value| value.is_finite()) {
                return Err(DecodedFrameError::NonFinite { pixel_index });
            }
            output.push(values);
        }
        Ok(output)
    }

    /// Returns the native scalar representation published by the decoder.
    #[must_use]
    pub const fn sample_type(&self) -> SampleType {
        self.image.descriptor().format().sample_type()
    }
}

/// Compatibility name retained for callers of the original decode contract.
pub type DecodeResult = DecodedFrame;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodedFrameError {
    ArithmeticOverflow,
    AllocationFailure,
    BufferInvariant,
    UnsupportedStorage,
    UnsupportedByteOrder,
    UnsupportedChannels,
    UnsupportedAlpha,
    NonFinite { pixel_index: usize },
}

fn read_sample(sample_type: SampleType, bytes: &[u8]) -> Result<f32, DecodedFrameError> {
    match sample_type {
        SampleType::U8 => Ok(f32::from(bytes[0]) / 255.0),
        SampleType::U16 => Ok(f32::from(u16::from_ne_bytes(
            bytes
                .try_into()
                .map_err(|_| DecodedFrameError::BufferInvariant)?,
        )) / 65_535.0),
        SampleType::F16 => Ok(half_to_f32(u16::from_ne_bytes(
            bytes
                .try_into()
                .map_err(|_| DecodedFrameError::BufferInvariant)?,
        ))),
        SampleType::F32 => Ok(f32::from_ne_bytes(
            bytes
                .try_into()
                .map_err(|_| DecodedFrameError::BufferInvariant)?,
        )),
    }
}

fn half_to_f32(bits: u16) -> f32 {
    let sign = u32::from(bits & 0x8000) << 16;
    let exponent = u32::from((bits >> 10) & 0x1f);
    let fraction = u32::from(bits & 0x03ff);
    let value = match exponent {
        0 => {
            if fraction == 0 {
                sign
            } else {
                let mut fraction = fraction;
                let mut exponent = 127 - 14;
                while fraction & 0x0400 == 0 {
                    fraction <<= 1;
                    exponent -= 1;
                }
                sign | (exponent << 23) | ((fraction & 0x03ff) << 13)
            }
        }
        0x1f => sign | 0x7f80_0000 | (fraction << 13),
        _ => sign | ((exponent + 112) << 23) | (fraction << 13),
    };
    f32::from_bits(value)
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
    InvalidSourceColor,
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
            Self::InvalidSourceColor => "decoded frame source-color evidence is invalid",
        })
    }
}

impl std::error::Error for DecodeError {}
