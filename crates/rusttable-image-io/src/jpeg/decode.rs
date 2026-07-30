use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use jpeg_decoder::{ColorTransform, Decoder as BackendDecoder, Error as BackendError};
use rusttable_image::{DecodeLimits, ImageInputError, InputFormat, UnsupportedImageFeature};
use sha2::{Digest, Sha256};

use super::parser;
use super::types::{JpegCodingProcess, JpegComponentModel, JpegHeader, JpegPixelData};
use crate::raw::{
    RawByteSource, RawCancellationToken, RawDecodeLimits, RawSourceError, SliceRawSource,
};

const BACKEND_ID: &str = "jpeg-decoder-0.3.2";
const COPY_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegDecodeMode {
    Header,
    Full,
    Region {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
}

#[derive(Debug, Clone)]
pub struct JpegDecodeRequest {
    pub limits: RawDecodeLimits,
    pub mode: JpegDecodeMode,
    pub cancellation: RawCancellationToken,
}

impl JpegDecodeRequest {
    #[must_use]
    pub fn new(limits: RawDecodeLimits) -> Self {
        Self {
            limits,
            mode: JpegDecodeMode::Full,
            cancellation: RawCancellationToken::new(),
        }
    }

    #[must_use]
    pub const fn header(mut self) -> Self {
        self.mode = JpegDecodeMode::Header;
        self
    }

    #[must_use]
    pub const fn region(mut self, x: u32, y: u32, width: u32, height: u32) -> Self {
        self.mode = JpegDecodeMode::Region {
            x,
            y,
            width,
            height,
        };
        self
    }

    #[must_use]
    pub fn with_cancellation(mut self, cancellation: RawCancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegDecodeReceipt {
    pub backend: String,
    pub source_bytes: u64,
    pub source_sha256: [u8; 32],
    pub output_bytes: u64,
    pub sof: u8,
    pub precision: u8,
    pub components: JpegComponentModel,
    pub sampling: Vec<super::types::JpegSampling>,
    pub scans: u16,
    pub restart_interval: u16,
    pub metadata_segments: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegDecodeResult {
    pub header: JpegHeader,
    pub pixels: Option<JpegPixelData>,
    pub receipt: JpegDecodeReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JpegDecodeError {
    Input(ImageInputError),
    Cancelled,
    Source(RawSourceError),
    UnsupportedRegion,
    Backend(String),
}

impl std::fmt::Display for JpegDecodeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Input(error) => error.fmt(formatter),
            Self::Cancelled => formatter.write_str("JPEG decode cancelled"),
            Self::Source(error) => write!(formatter, "JPEG source failed: {error:?}"),
            Self::UnsupportedRegion => formatter.write_str("JPEG region decoding is unsupported"),
            Self::Backend(message) => write!(formatter, "JPEG backend failed: {message}"),
        }
    }
}

impl std::error::Error for JpegDecodeError {}

impl From<ImageInputError> for JpegDecodeError {
    fn from(error: ImageInputError) -> Self {
        Self::Input(error)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct JpegDecoder;

impl JpegDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Probes a bounded JPEG header without decoding entropy coefficients.
    ///
    /// # Errors
    ///
    /// Returns malformed-input, unsupported-feature, or configured-limit errors.
    pub fn probe_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<JpegHeader, ImageInputError> {
        parser::probe(bytes, limits)
    }

    /// Validates the complete marker and entropy structure without publishing pixels.
    ///
    /// # Errors
    ///
    /// Returns malformed-input, unsupported-feature, or configured-limit errors.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<JpegHeader, ImageInputError> {
        parser::inspect(bytes, limits)
    }

    /// Decodes an immutable byte slice through the bounded JPEG boundary.
    ///
    /// # Errors
    ///
    /// Returns source, cancellation, malformed-input, unsupported-feature, or backend errors.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &JpegDecodeRequest,
    ) -> Result<JpegDecodeResult, JpegDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Decodes a bounded random-access source and rejects source mutation before publication.
    ///
    /// # Errors
    ///
    /// Returns source, cancellation, malformed-input, unsupported-feature, or backend errors.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &JpegDecodeRequest,
    ) -> Result<JpegDecodeResult, JpegDecodeError> {
        check_cancel(request)?;
        let snapshot = Snapshot::read(source, request)?;
        check_cancel(request)?;
        let probe_limits = decode_limits(request.limits)?;
        let header = parser::inspect(snapshot.bytes.as_slice(), probe_limits)?;
        validate_header(&header, request)?;
        if matches!(request.mode, JpegDecodeMode::Region { .. }) {
            return Err(JpegDecodeError::UnsupportedRegion);
        }
        let pixels = if matches!(request.mode, JpegDecodeMode::Header) {
            None
        } else {
            Some(decode_pixels(snapshot.bytes.as_slice(), &header, request)?)
        };
        check_cancel(request)?;
        let output_bytes = pixels.as_ref().map_or(0, JpegPixelData::byte_len);
        let receipt = JpegDecodeReceipt {
            backend: BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            output_bytes: u64::try_from(output_bytes)
                .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?,
            sof: header.sof.marker(),
            precision: header.precision,
            components: header.components,
            sampling: header.sampling.clone(),
            scans: header.scans,
            restart_interval: header.restart_interval,
            metadata_segments: u16::try_from(header.metadata.len())
                .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?,
        };
        Ok(JpegDecodeResult {
            header,
            pixels,
            receipt,
        })
    }

    /// Decodes a JPEG for the legacy registry's checked opaque RGBA8 facade.
    ///
    /// # Errors
    ///
    /// Returns malformed-input, unsupported-feature, limit, allocation, or backend errors.
    pub fn decode_rgba8(
        &self,
        bytes: &[u8],
        limits: DecodeLimits,
    ) -> Result<
        (
            rusttable_image::ImageDimensions,
            rusttable_image::Orientation,
            Vec<u8>,
        ),
        ImageInputError,
    > {
        let raw_limits = raw_limits(limits)?;
        let request = JpegDecodeRequest::new(raw_limits);
        let result = self.decode_bytes(bytes, &request).map_err(map_error)?;
        let pixels = result
            .pixels
            .ok_or_else(|| malformed("JPEG header-only decode returned no pixels"))?;
        let rgba = to_rgba8(&pixels, result.header.dimensions)?;
        Ok((result.header.dimensions, result.header.orientation, rgba))
    }
}

fn decode_pixels(
    bytes: &[u8],
    header: &JpegHeader,
    request: &JpegDecodeRequest,
) -> Result<JpegPixelData, JpegDecodeError> {
    if header.precision != 8 {
        return Err(JpegDecodeError::Input(unsupported(
            UnsupportedImageFeature::BitDepth,
        )));
    }
    let max = usize::try_from(request.limits.max_decoded_bytes)
        .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    let transform = match header.components {
        JpegComponentModel::Gray => ColorTransform::Grayscale,
        JpegComponentModel::Rgb => ColorTransform::RGB,
        JpegComponentModel::Ycbcr => ColorTransform::YCbCr,
        JpegComponentModel::Cmyk | JpegComponentModel::Ycck => ColorTransform::None,
        JpegComponentModel::Unknown(_) => {
            return Err(JpegDecodeError::Input(unsupported(
                UnsupportedImageFeature::ColorModel,
            )));
        }
    };
    let backend = catch_unwind(AssertUnwindSafe(|| {
        let mut decoder = BackendDecoder::new(Cursor::new(bytes));
        decoder.set_color_transform(transform);
        decoder.set_max_decoding_buffer_size(max);
        decoder.decode()
    }))
    .map_err(|_| JpegDecodeError::Backend("decoder panicked on validated input".to_owned()))?;
    let decoded = backend.map_err(map_backend_error)?;
    let expected = expected_sample_bytes(header)?;
    if decoded.len() != expected {
        return Err(JpegDecodeError::Input(
            ImageInputError::DecodedBufferInvariant {
                expected: u64::try_from(expected).unwrap_or(u64::MAX),
                actual: u64::try_from(decoded.len()).unwrap_or(u64::MAX),
            },
        ));
    }
    let pixels = match header.components {
        JpegComponentModel::Gray => JpegPixelData::GrayU8(decoded),
        JpegComponentModel::Rgb | JpegComponentModel::Ycbcr => JpegPixelData::RgbU8(decoded),
        JpegComponentModel::Cmyk | JpegComponentModel::Ycck => {
            JpegPixelData::CmykU8(normalize_cmyk(decoded, header))
        }
        JpegComponentModel::Unknown(_) => unreachable!("validated component model"),
    };
    let byte_length = u64::try_from(pixels.byte_len())
        .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    if byte_length > request.limits.max_decoded_bytes {
        return Err(JpegDecodeError::Input(ImageInputError::DecodedByteLimit {
            actual: byte_length,
            limit: request.limits.max_decoded_bytes,
        }));
    }
    Ok(pixels)
}

fn validate_header(
    header: &JpegHeader,
    request: &JpegDecodeRequest,
) -> Result<(), JpegDecodeError> {
    let dimensions =
        crate::raw::RawDimensions::new(header.dimensions.width(), header.dimensions.height())
            .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    let pixels = u64::from(dimensions.width)
        .checked_mul(u64::from(dimensions.height))
        .ok_or(JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    if pixels > request.limits.max_pixels {
        return Err(JpegDecodeError::Input(ImageInputError::PixelLimit {
            actual: pixels,
            limit: request.limits.max_pixels,
        }));
    }
    if header.precision != 8 && header.precision != 12 {
        return Err(JpegDecodeError::Input(unsupported(
            UnsupportedImageFeature::BitDepth,
        )));
    }
    match header.coding_process {
        JpegCodingProcess::Baseline
        | JpegCodingProcess::ExtendedSequential
        | JpegCodingProcess::Progressive => {}
        JpegCodingProcess::Arithmetic => {
            return Err(JpegDecodeError::Input(unsupported(
                UnsupportedImageFeature::ArithmeticCoding,
            )));
        }
        JpegCodingProcess::Lossless | JpegCodingProcess::Unsupported(_) => {
            return Err(JpegDecodeError::Input(unsupported(
                UnsupportedImageFeature::CodingProcess,
            )));
        }
    }
    if header.components.channels() == 0 || header.components.channels() > 4 {
        return Err(JpegDecodeError::Input(unsupported(
            UnsupportedImageFeature::ColorModel,
        )));
    }
    Ok(())
}

fn expected_sample_bytes(header: &JpegHeader) -> Result<usize, JpegDecodeError> {
    let pixels = usize::try_from(
        u64::from(header.dimensions.width())
            .checked_mul(u64::from(header.dimensions.height()))
            .ok_or(JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?,
    )
    .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))?;
    pixels
        .checked_mul(usize::from(header.channels()))
        .ok_or(JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))
}

fn normalize_cmyk(mut values: Vec<u8>, header: &JpegHeader) -> Vec<u8> {
    for pixel in values.as_chunks_mut::<4>().0 {
        if header.components == JpegComponentModel::Ycck {
            let (r, g, b) = ycbcr_to_rgb(pixel[0], pixel[1], pixel[2]);
            pixel[0] = 255 - r;
            pixel[1] = 255 - g;
            pixel[2] = 255 - b;
            pixel[3] = 255 - pixel[3];
        } else if header.adobe_color_transform == Some(0) {
            for value in pixel {
                *value = 255 - *value;
            }
        }
    }
    values
}

fn to_rgba8(
    pixels: &JpegPixelData,
    dimensions: rusttable_image::ImageDimensions,
) -> Result<Vec<u8>, ImageInputError> {
    let count = usize::try_from(
        u64::from(dimensions.width())
            .checked_mul(u64::from(dimensions.height()))
            .ok_or(ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(
        count
            .checked_mul(4)
            .ok_or(ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::AllocationFailure)?;
    match pixels {
        JpegPixelData::GrayU8(values) => {
            for &value in values {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        JpegPixelData::RgbU8(values) => {
            for pixel in values.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
        }
        JpegPixelData::CmykU8(values) => {
            for pixel in values.as_chunks::<4>().0 {
                rgba.extend_from_slice(&[
                    cmyk_channel(pixel[0], pixel[3]),
                    cmyk_channel(pixel[1], pixel[3]),
                    cmyk_channel(pixel[2], pixel[3]),
                    255,
                ]);
            }
        }
        JpegPixelData::GrayU16(_) | JpegPixelData::RgbU16(_) | JpegPixelData::CmykU16(_) => {
            return Err(unsupported(UnsupportedImageFeature::BitDepth));
        }
    }
    if rgba.len()
        != count
            .checked_mul(4)
            .ok_or(ImageInputError::ArithmeticOverflow)?
    {
        return Err(ImageInputError::DecodedBufferInvariant {
            expected: u64::try_from(count.saturating_mul(4)).unwrap_or(u64::MAX),
            actual: u64::try_from(rgba.len()).unwrap_or(u64::MAX),
        });
    }
    Ok(rgba)
}

fn cmyk_channel(channel: u8, key: u8) -> u8 {
    let value = u16::from(channel) + u16::from(key);
    u8::try_from(255_u16.saturating_sub(value.min(255))).expect("CMYK channel is bounded")
}

fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = i32::from(y);
    let cb = i32::from(cb) - 128;
    let cr = i32::from(cr) - 128;
    let r = (y * 1_000 + 1_402 * cr).clamp(0, 255_000) / 1_000;
    let g = (y * 1_000 - 344 * cb - 714 * cr).clamp(0, 255_000) / 1_000;
    let b = (y * 1_000 + 1_772 * cb).clamp(0, 255_000) / 1_000;
    (
        u8::try_from(r).expect("clamped red channel is bounded"),
        u8::try_from(g).expect("clamped green channel is bounded"),
        u8::try_from(b).expect("clamped blue channel is bounded"),
    )
}

fn decode_limits(limits: RawDecodeLimits) -> Result<DecodeLimits, JpegDecodeError> {
    DecodeLimits::new(
        limits.max_source_bytes,
        limits.max_width,
        limits.max_height,
        limits.max_pixels,
        limits
            .max_decoded_bytes
            .min(limits.max_pixels.saturating_mul(4)),
    )
    .map_err(|_| JpegDecodeError::Input(ImageInputError::ArithmeticOverflow))
}

fn raw_limits(limits: DecodeLimits) -> Result<RawDecodeLimits, ImageInputError> {
    RawDecodeLimits::new(
        limits.max_source_bytes(),
        limits.max_width(),
        limits.max_height(),
        limits.max_pixel_count(),
        limits.max_decoded_bytes(),
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)
}

fn check_cancel(request: &JpegDecodeRequest) -> Result<(), JpegDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(JpegDecodeError::Cancelled)
    } else {
        Ok(())
    }
}

fn map_backend_error(error: BackendError) -> JpegDecodeError {
    match error {
        BackendError::Unsupported(feature) => {
            let reason = match feature {
                jpeg_decoder::UnsupportedFeature::ArithmeticEntropyCoding => {
                    UnsupportedImageFeature::ArithmeticCoding
                }
                jpeg_decoder::UnsupportedFeature::SamplePrecision(_) => {
                    UnsupportedImageFeature::BitDepth
                }
                jpeg_decoder::UnsupportedFeature::ComponentCount(_) => {
                    UnsupportedImageFeature::ColorModel
                }
                jpeg_decoder::UnsupportedFeature::SubsamplingRatio
                | jpeg_decoder::UnsupportedFeature::NonIntegerSubsamplingRatio => {
                    UnsupportedImageFeature::Sampling
                }
                _ => UnsupportedImageFeature::CodingProcess,
            };
            JpegDecodeError::Input(unsupported(reason))
        }
        BackendError::Format(message) => JpegDecodeError::Input(malformed(&message)),
        BackendError::Io(error) => JpegDecodeError::Input(ImageInputError::Io {
            message: error.to_string(),
        }),
        BackendError::Internal(error) => JpegDecodeError::Backend(error.to_string()),
    }
}

fn map_error(error: JpegDecodeError) -> ImageInputError {
    match error {
        JpegDecodeError::Input(error) => error,
        JpegDecodeError::Cancelled => ImageInputError::MalformedInput {
            format: InputFormat::Jpeg,
            message: "JPEG decode cancelled".to_owned(),
        },
        JpegDecodeError::Source(error) => ImageInputError::Io {
            message: format!("{error:?}"),
        },
        JpegDecodeError::UnsupportedRegion => ImageInputError::UnsupportedFeature {
            format: InputFormat::Jpeg,
            reason: UnsupportedImageFeature::Region,
        },
        JpegDecodeError::Backend(message) => malformed(&message),
    }
}

fn malformed(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Jpeg,
        message: message.to_owned(),
    }
}

fn unsupported(reason: UnsupportedImageFeature) -> ImageInputError {
    ImageInputError::UnsupportedFeature {
        format: InputFormat::Jpeg,
        reason,
    }
}

struct Snapshot {
    bytes: Vec<u8>,
    source_bytes: u64,
    sha256: [u8; 32],
}

impl Snapshot {
    fn read<S: RawByteSource + ?Sized>(
        source: &S,
        request: &JpegDecodeRequest,
    ) -> Result<Self, JpegDecodeError> {
        let length = source.len().map_err(JpegDecodeError::Source)?;
        if length == 0 {
            return Err(JpegDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(JpegDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(JpegDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| JpegDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| JpegDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            if request.cancellation.is_cancelled() {
                return Err(JpegDecodeError::Cancelled);
            }
            let offset = index
                .checked_mul(COPY_CHUNK_BYTES)
                .and_then(|value| u64::try_from(value).ok())
                .ok_or(JpegDecodeError::Source(RawSourceError::LengthConversion))?;
            source
                .read_exact_at(offset, chunk)
                .map_err(JpegDecodeError::Source)?;
        }
        if source.revision().map_err(JpegDecodeError::Source)? != revision {
            return Err(JpegDecodeError::Source(RawSourceError::Changed));
        }
        Ok(Self {
            source_bytes: length,
            sha256: Sha256::digest(bytes.as_slice()).into(),
            bytes,
        })
    }
}
