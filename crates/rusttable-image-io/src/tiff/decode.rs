use std::io::Cursor;
use std::panic::{AssertUnwindSafe, catch_unwind};

use rusttable_image::{DecodeLimits, ImageInputError, ImageProbe, InputFormat, Roi};
use sha2::{Digest, Sha256};
use tiff::decoder::{Decoder, DecodingResult, Limits};

use super::parser::parse;
use super::types::{
    TIFF_BACKEND_ID, TiffAlphaSample, TiffByteOrder, TiffContainer, TiffDecodeError,
    TiffDecodeLimits, TiffDecodeMode, TiffDecodeReceipt, TiffDecodeRequest, TiffDecodeResult,
    TiffHeader, TiffPage, TiffPhotometric, TiffPixelData, TiffSampleData, TiffStorageLayout,
};
use crate::raw::{RawByteSource, RawSourceError, SliceRawSource};

const COPY_CHUNK_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, Default)]
pub struct TiffDecoder;

impl TiffDecoder {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Parses and validates the complete classic TIFF or `BigTIFF` IFD graph.
    ///
    /// # Errors
    ///
    /// Returns a typed malformed, unsupported, arithmetic, or limit error.
    pub fn inspect_bytes(
        &self,
        bytes: &[u8],
        limits: TiffDecodeLimits,
    ) -> Result<TiffHeader, TiffDecodeError> {
        Ok(parse(bytes, limits)?.header)
    }

    /// Decodes an immutable source snapshot without publishing partial output.
    ///
    /// # Errors
    ///
    /// Returns a typed source, malformed, unsupported, cancellation, backend, or limit error.
    pub fn decode_bytes(
        &self,
        bytes: &[u8],
        request: &TiffDecodeRequest,
    ) -> Result<TiffDecodeResult, TiffDecodeError> {
        self.decode_source(&SliceRawSource::new(bytes), request)
    }

    /// Copies bounded random-access bytes and rejects cancellation or source mutation.
    ///
    /// # Errors
    ///
    /// Returns a typed source, malformed, unsupported, cancellation, backend, or limit error.
    pub fn decode_source<S: RawByteSource + ?Sized>(
        &self,
        source: &S,
        request: &TiffDecodeRequest,
    ) -> Result<TiffDecodeResult, TiffDecodeError> {
        check_cancel(request)?;
        let snapshot = Snapshot::read(source, request)?;
        check_cancel(request)?;
        let header = parse(&snapshot.bytes, request.limits)?.header;
        let page_index = request.page.unwrap_or(header.default_page);
        let page = header
            .pages
            .get(page_index)
            .cloned()
            .ok_or(TiffDecodeError::InvalidPage(page_index))?;
        let region = match request.mode {
            TiffDecodeMode::Region(roi) => Some(
                roi.within(page.dimensions)
                    .map_err(|_| TiffDecodeError::InvalidRegion)?,
            ),
            TiffDecodeMode::Header | TiffDecodeMode::Full => None,
        };
        if region.is_some_and(Roi::is_empty) {
            return Err(TiffDecodeError::InvalidRegion);
        }
        let header_only = request.mode == TiffDecodeMode::Header;
        let pixels = if header_only {
            None
        } else {
            let decoded = decode_page(&snapshot.bytes, &header, &page, request.limits)?;
            Some(match region {
                Some(roi) => crop(decoded, roi)?,
                None => decoded,
            })
        };
        check_cancel(request)?;
        let output_bytes = pixels.as_ref().map_or(0, TiffPixelData::sample_bytes);
        let receipt = TiffDecodeReceipt {
            backend: TIFF_BACKEND_ID.to_owned(),
            source_bytes: snapshot.source_bytes,
            source_sha256: snapshot.sha256,
            page_index,
            ifd_offset: page.ifd_offset,
            region,
            output_bytes,
            decompressed_bytes: if header_only {
                0
            } else {
                logical_bytes(&page)?
            },
            header_only,
        };
        Ok(TiffDecodeResult {
            header,
            page,
            pixels,
            receipt,
        })
    }
}

pub(crate) fn is_tiff_signature(bytes: &[u8]) -> bool {
    matches!(
        bytes.get(..4),
        Some([b'I', b'I', 42 | 43, 0] | [b'M', b'M', 0, 42 | 43])
    )
}

pub(crate) fn decode_tiff_probe(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let header = TiffDecoder::new()
        .inspect_bytes(bytes, TiffDecodeLimits::from_common(limits))
        .map_err(map_error)?;
    Ok(ImageProbe::new(
        InputFormat::Tiff,
        header.default_page().dimensions,
    ))
}

pub(crate) fn decode_legacy_rgba8(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<(rusttable_image::ImageDimensions, Vec<u8>), ImageInputError> {
    let request = TiffDecodeRequest::new(TiffDecodeLimits::from_common(limits));
    let result = TiffDecoder::new()
        .decode_bytes(bytes, &request)
        .map_err(map_error)?;
    let pixels = result
        .pixels
        .as_ref()
        .ok_or_else(|| malformed_input("TIFF full decode returned no samples"))?;
    let rgba = to_rgba8(pixels, &result.page)?;
    Ok((pixels.dimensions, rgba))
}

fn decode_page(
    bytes: &[u8],
    header: &TiffHeader,
    page: &TiffPage,
    limits: TiffDecodeLimits,
) -> Result<TiffPixelData, TiffDecodeError> {
    let source = backend_source(bytes, header, page)?;
    let mut backend_limits = Limits::default();
    backend_limits.decoding_buffer_size =
        to_usize(limits.max_decompressed_bytes.min(limits.max_decoded_bytes))?;
    backend_limits.intermediate_buffer_size = to_usize(limits.max_temporary_bytes)?;
    backend_limits.ifd_value_size = to_usize(limits.max_ifd_value_bytes)?;
    let mut decoder = catch_unwind(AssertUnwindSafe(|| Decoder::new(Cursor::new(source))))
        .map_err(|_| TiffDecodeError::Backend("decoder panicked while opening TIFF".to_owned()))?
        .map_err(map_backend)?
        .with_limits(backend_limits);
    let (width, height) = decoder.dimensions().map_err(map_backend)?;
    if (width, height) != (page.dimensions.width(), page.dimensions.height()) {
        return Err(TiffDecodeError::Malformed(
            "backend dimensions differ from selected IFD".to_owned(),
        ));
    }
    let mut result = DecodingResult::U8(Vec::new());
    let layout = catch_unwind(AssertUnwindSafe(|| {
        decoder.read_image_to_buffer(&mut result)
    }))
    .map_err(|_| TiffDecodeError::Backend("decoder panicked while decoding TIFF".to_owned()))?
    .map_err(map_backend)?;
    if page.storage == TiffStorageLayout::Planar
        && layout.planes != usize::from(page.samples_per_pixel)
    {
        return Err(TiffDecodeError::Malformed(
            "backend did not return every planar sample plane".to_owned(),
        ));
    }
    let samples = match result {
        DecodingResult::U8(values) => TiffSampleData::U8(values),
        DecodingResult::U16(values) => TiffSampleData::U16(values),
        DecodingResult::U32(values) => TiffSampleData::U32(values),
        DecodingResult::I8(values) => TiffSampleData::I8(values),
        DecodingResult::I16(values) => TiffSampleData::I16(values),
        DecodingResult::I32(values) => TiffSampleData::I32(values),
        DecodingResult::F16(values) => {
            TiffSampleData::F16(values.into_iter().map(half::f16::to_bits).collect())
        }
        DecodingResult::F32(values) => TiffSampleData::F32(values),
        DecodingResult::U64(_) | DecodingResult::I64(_) | DecodingResult::F64(_) => {
            return Err(TiffDecodeError::Malformed(
                "backend returned an unrequested 64-bit sample type".to_owned(),
            ));
        }
    };
    let expected = expected_samples(page.dimensions, page.samples_per_pixel)?;
    if samples.len() != expected {
        return Err(TiffDecodeError::Malformed(format!(
            "decoded sample count {} differs from expected {expected}",
            samples.len()
        )));
    }
    Ok(TiffPixelData {
        dimensions: page.dimensions,
        samples_per_pixel: page.samples_per_pixel,
        storage: page.storage,
        samples,
    })
}

fn backend_source(
    bytes: &[u8],
    header: &TiffHeader,
    page: &TiffPage,
) -> Result<Vec<u8>, TiffDecodeError> {
    let mut source = bytes.to_vec();
    match header.container {
        TiffContainer::Classic => write_u32(
            &mut source,
            4,
            u32::try_from(page.ifd_offset).map_err(|_| TiffDecodeError::ArithmeticOverflow)?,
            header.byte_order,
        )?,
        TiffContainer::BigTiff => write_u64(&mut source, 8, page.ifd_offset, header.byte_order)?,
    }
    // The backend transforms WhiteIsZero and rejects palette/CIELab before
    // exposing samples. Substitute an equivalent channel-count interpretation
    // so RustTable receives exact stored samples and retains the real descriptor.
    let substitute = match page.photometric {
        TiffPhotometric::WhiteIsZero | TiffPhotometric::Palette | TiffPhotometric::Cfa => {
            Some(1_u16)
        }
        TiffPhotometric::CieLab => Some(2),
        _ => None,
    };
    if let Some(value) = substitute {
        patch_short_tag(
            &mut source,
            header.container,
            header.byte_order,
            page.ifd_offset,
            262,
            value,
        )?;
    }
    Ok(source)
}

fn patch_short_tag(
    bytes: &mut [u8],
    container: TiffContainer,
    order: TiffByteOrder,
    ifd_offset: u64,
    wanted: u16,
    value: u16,
) -> Result<(), TiffDecodeError> {
    let start = to_usize(ifd_offset)?;
    let (count, count_bytes, entry_bytes, value_offset) = match container {
        TiffContainer::Classic => (
            u64::from(read_u16(bytes, start, order)?),
            2_usize,
            12_usize,
            8_usize,
        ),
        TiffContainer::BigTiff => (read_u64(bytes, start, order)?, 8, 20, 12),
    };
    for index in 0..to_usize(count)? {
        let entry = start
            .checked_add(count_bytes)
            .and_then(|current| current.checked_add(index.checked_mul(entry_bytes)?))
            .ok_or(TiffDecodeError::ArithmeticOverflow)?;
        if read_u16(bytes, entry, order)? == wanted {
            if read_u16(bytes, entry + 2, order)? != 3 {
                return Err(TiffDecodeError::Malformed(
                    "PhotometricInterpretation is not SHORT".to_owned(),
                ));
            }
            return write_u16(bytes, entry + value_offset, value, order);
        }
    }
    Err(TiffDecodeError::Malformed(
        "selected IFD lost PhotometricInterpretation".to_owned(),
    ))
}

fn crop(mut pixels: TiffPixelData, roi: Roi) -> Result<TiffPixelData, TiffDecodeError> {
    let width = usize::try_from(pixels.dimensions.width())
        .map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    let channels = usize::from(pixels.samples_per_pixel);
    let x = usize::try_from(roi.x()).map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    let y = usize::try_from(roi.y()).map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    let roi_width =
        usize::try_from(roi.width()).map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    let roi_height =
        usize::try_from(roi.height()).map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    let source_pixels = usize::try_from(
        pixels
            .dimensions
            .pixel_count()
            .map_err(|_| TiffDecodeError::ArithmeticOverflow)?,
    )
    .map_err(|_| TiffDecodeError::ArithmeticOverflow)?;
    macro_rules! crop_variant {
        ($values:expr) => {
            crop_values(
                $values,
                pixels.storage,
                width,
                source_pixels,
                channels,
                x,
                y,
                roi_width,
                roi_height,
            )?
        };
    }
    pixels.samples = match pixels.samples {
        TiffSampleData::U8(values) => TiffSampleData::U8(crop_variant!(&values)),
        TiffSampleData::U16(values) => TiffSampleData::U16(crop_variant!(&values)),
        TiffSampleData::U32(values) => TiffSampleData::U32(crop_variant!(&values)),
        TiffSampleData::I8(values) => TiffSampleData::I8(crop_variant!(&values)),
        TiffSampleData::I16(values) => TiffSampleData::I16(crop_variant!(&values)),
        TiffSampleData::I32(values) => TiffSampleData::I32(crop_variant!(&values)),
        TiffSampleData::F16(values) => TiffSampleData::F16(crop_variant!(&values)),
        TiffSampleData::F32(values) => TiffSampleData::F32(crop_variant!(&values)),
    };
    pixels.dimensions = rusttable_image::ImageDimensions::new(roi.width(), roi.height())
        .map_err(|_| TiffDecodeError::InvalidRegion)?;
    Ok(pixels)
}

#[allow(clippy::too_many_arguments)]
fn crop_values<T: Copy>(
    values: &[T],
    storage: TiffStorageLayout,
    width: usize,
    source_pixels: usize,
    channels: usize,
    x: usize,
    y: usize,
    roi_width: usize,
    roi_height: usize,
) -> Result<Vec<T>, TiffDecodeError> {
    let output_len = roi_width
        .checked_mul(roi_height)
        .and_then(|current| current.checked_mul(channels))
        .ok_or(TiffDecodeError::ArithmeticOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(output_len)
        .map_err(|_| TiffDecodeError::AllocationFailure)?;
    match storage {
        TiffStorageLayout::Chunky => {
            for row in y..y + roi_height {
                let start = row
                    .checked_mul(width)
                    .and_then(|current| current.checked_add(x))
                    .and_then(|current| current.checked_mul(channels))
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                let len = roi_width
                    .checked_mul(channels)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                output.extend_from_slice(
                    values.get(start..start + len).ok_or_else(|| {
                        TiffDecodeError::Malformed("ROI exceeds samples".to_owned())
                    })?,
                );
            }
        }
        TiffStorageLayout::Planar => {
            for channel in 0..channels {
                let plane = channel
                    .checked_mul(source_pixels)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                for row in y..y + roi_height {
                    let start = plane
                        .checked_add(
                            row.checked_mul(width)
                                .ok_or(TiffDecodeError::ArithmeticOverflow)?,
                        )
                        .and_then(|current| current.checked_add(x))
                        .ok_or(TiffDecodeError::ArithmeticOverflow)?;
                    output.extend_from_slice(values.get(start..start + roi_width).ok_or_else(
                        || TiffDecodeError::Malformed("ROI exceeds planar samples".to_owned()),
                    )?);
                }
            }
        }
    }
    Ok(output)
}

fn to_rgba8(pixels: &TiffPixelData, page: &TiffPage) -> Result<Vec<u8>, ImageInputError> {
    if page.alpha.contains(&TiffAlphaSample::Premultiplied) {
        return Err(malformed_input(
            "legacy RGBA8 cannot represent premultiplied TIFF alpha",
        ));
    }
    if !matches!(
        page.photometric,
        TiffPhotometric::WhiteIsZero
            | TiffPhotometric::BlackIsZero
            | TiffPhotometric::Palette
            | TiffPhotometric::Rgb
    ) {
        return Err(malformed_input(
            "legacy RGBA8 requires explicit TIFF color conversion",
        ));
    }
    let pixel_count = usize::try_from(
        pixels
            .dimensions
            .pixel_count()
            .map_err(|_| ImageInputError::ArithmeticOverflow)?,
    )
    .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(
            pixel_count
                .checked_mul(4)
                .ok_or(ImageInputError::ArithmeticOverflow)?,
        )
        .map_err(|_| ImageInputError::AllocationFailure)?;
    for pixel in 0..pixel_count {
        let sample = |channel| sample_u8(pixels, pixel_count, pixel, channel);
        let alpha_channel = usize::from(page.photometric.color_samples());
        let alpha = if page.alpha.is_empty() {
            255
        } else {
            sample(alpha_channel)?
        };
        let rgb = match page.photometric {
            TiffPhotometric::WhiteIsZero => {
                let gray = 255 - sample(0)?;
                [gray, gray, gray]
            }
            TiffPhotometric::BlackIsZero => {
                let gray = sample(0)?;
                [gray, gray, gray]
            }
            TiffPhotometric::Rgb => [sample(0)?, sample(1)?, sample(2)?],
            TiffPhotometric::Palette => {
                let index = usize::from(sample(0)?);
                let map = page
                    .color_map
                    .as_ref()
                    .ok_or_else(|| malformed_input("palette has no ColorMap"))?;
                let entries = map.len() / 3;
                if index >= entries {
                    return Err(malformed_input("palette sample is out of range"));
                }
                [
                    (map[index] >> 8) as u8,
                    (map[entries + index] >> 8) as u8,
                    (map[2 * entries + index] >> 8) as u8,
                ]
            }
            _ => unreachable!("photometric was checked above"),
        };
        output.extend_from_slice(&[rgb[0], rgb[1], rgb[2], alpha]);
    }
    Ok(output)
}

fn sample_u8(
    pixels: &TiffPixelData,
    pixel_count: usize,
    pixel: usize,
    channel: usize,
) -> Result<u8, ImageInputError> {
    let index = match pixels.storage {
        TiffStorageLayout::Chunky => pixel
            .checked_mul(usize::from(pixels.samples_per_pixel))
            .and_then(|current| current.checked_add(channel)),
        TiffStorageLayout::Planar => channel
            .checked_mul(pixel_count)
            .and_then(|current| current.checked_add(pixel)),
    }
    .ok_or(ImageInputError::ArithmeticOverflow)?;
    match &pixels.samples {
        TiffSampleData::U8(values) => values.get(index).copied(),
        TiffSampleData::U16(values) => values.get(index).map(|value| (value >> 8) as u8),
        TiffSampleData::U32(values) => values.get(index).map(|value| (value >> 24) as u8),
        _ => None,
    }
    .ok_or_else(|| malformed_input("legacy RGBA8 requires unsigned integer samples"))
}

fn logical_bytes(page: &TiffPage) -> Result<u64, TiffDecodeError> {
    page.dimensions
        .pixel_count()
        .map_err(|_| TiffDecodeError::ArithmeticOverflow)?
        .checked_mul(u64::from(page.samples_per_pixel))
        .and_then(|current| {
            current.checked_mul(u64::from(page.bits_per_sample[0].div_ceil(8).max(1)))
        })
        .ok_or(TiffDecodeError::ArithmeticOverflow)
}

fn expected_samples(
    dimensions: rusttable_image::ImageDimensions,
    channels: u16,
) -> Result<usize, TiffDecodeError> {
    let count = dimensions
        .pixel_count()
        .map_err(|_| TiffDecodeError::ArithmeticOverflow)?
        .checked_mul(u64::from(channels))
        .ok_or(TiffDecodeError::ArithmeticOverflow)?;
    usize::try_from(count).map_err(|_| TiffDecodeError::ArithmeticOverflow)
}

fn map_backend(error: tiff::TiffError) -> TiffDecodeError {
    match error {
        tiff::TiffError::LimitsExceeded | tiff::TiffError::IntSizeError => TiffDecodeError::Limit {
            kind: "backend bytes",
            actual: u64::MAX,
            limit: u64::MAX - 1,
        },
        tiff::TiffError::UnsupportedError(error) => {
            TiffDecodeError::Backend(format!("unsupported by pinned backend: {error}"))
        }
        other => TiffDecodeError::Backend(other.to_string()),
    }
}

fn map_error(error: TiffDecodeError) -> ImageInputError {
    match error {
        TiffDecodeError::Source(RawSourceError::TooLarge { actual, limit }) => {
            ImageInputError::SourceTooLarge { actual, limit }
        }
        TiffDecodeError::Limit {
            kind: "width",
            actual,
            limit,
        } => ImageInputError::WidthLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        TiffDecodeError::Limit {
            kind: "height",
            actual,
            limit,
        } => ImageInputError::HeightLimit {
            actual: u32::try_from(actual).unwrap_or(u32::MAX),
            limit: u32::try_from(limit).unwrap_or(u32::MAX),
        },
        TiffDecodeError::Limit {
            kind: "pixel count",
            actual,
            limit,
        } => ImageInputError::PixelLimit { actual, limit },
        TiffDecodeError::Limit {
            kind: "decoded bytes",
            actual,
            limit,
        } => ImageInputError::DecodedByteLimit { actual, limit },
        TiffDecodeError::ArithmeticOverflow => ImageInputError::ArithmeticOverflow,
        TiffDecodeError::AllocationFailure => ImageInputError::AllocationFailure,
        other => malformed_input(&other.to_string()),
    }
}

fn malformed_input(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Tiff,
        message: message.to_owned(),
    }
}

fn check_cancel(request: &TiffDecodeRequest) -> Result<(), TiffDecodeError> {
    if request.cancellation.is_cancelled() {
        Err(TiffDecodeError::Cancelled)
    } else {
        Ok(())
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
        request: &TiffDecodeRequest,
    ) -> Result<Self, TiffDecodeError> {
        let length = source.len().map_err(TiffDecodeError::Source)?;
        if length == 0 {
            return Err(TiffDecodeError::Source(RawSourceError::Empty));
        }
        if length > request.limits.max_source_bytes {
            return Err(TiffDecodeError::Source(RawSourceError::TooLarge {
                actual: length,
                limit: request.limits.max_source_bytes,
            }));
        }
        let revision = source.revision().map_err(TiffDecodeError::Source)?;
        let length_usize = usize::try_from(length)
            .map_err(|_| TiffDecodeError::Source(RawSourceError::LengthConversion))?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length_usize)
            .map_err(|_| TiffDecodeError::Source(RawSourceError::AllocationFailure))?;
        bytes.resize(length_usize, 0);
        for (index, chunk) in bytes.chunks_mut(COPY_CHUNK_BYTES).enumerate() {
            check_cancel(request)?;
            let offset = u64::try_from(index)
                .ok()
                .and_then(|current| current.checked_mul(COPY_CHUNK_BYTES as u64))
                .ok_or(TiffDecodeError::ArithmeticOverflow)?;
            source
                .read_exact_at(offset, chunk)
                .map_err(TiffDecodeError::Source)?;
        }
        if source.revision().map_err(TiffDecodeError::Source)? != revision {
            return Err(TiffDecodeError::Source(RawSourceError::Changed));
        }
        let sha256: [u8; 32] = Sha256::digest(&bytes).into();
        Ok(Self {
            bytes,
            source_bytes: length,
            sha256,
        })
    }
}

fn to_usize(value: u64) -> Result<usize, TiffDecodeError> {
    usize::try_from(value).map_err(|_| TiffDecodeError::ArithmeticOverflow)
}

fn read_u16(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u16, TiffDecodeError> {
    let value: [u8; 2] = bytes
        .get(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?
        .try_into()
        .map_err(|_| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?;
    Ok(match order {
        TiffByteOrder::Little => u16::from_le_bytes(value),
        TiffByteOrder::Big => u16::from_be_bytes(value),
    })
}

fn read_u64(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u64, TiffDecodeError> {
    let value: [u8; 8] = bytes
        .get(
            offset
                ..offset
                    .checked_add(8)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?
        .try_into()
        .map_err(|_| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?;
    Ok(match order {
        TiffByteOrder::Little => u64::from_le_bytes(value),
        TiffByteOrder::Big => u64::from_be_bytes(value),
    })
}

fn write_u16(
    bytes: &mut [u8],
    offset: usize,
    value: u16,
    order: TiffByteOrder,
) -> Result<(), TiffDecodeError> {
    let encoded = match order {
        TiffByteOrder::Little => value.to_le_bytes(),
        TiffByteOrder::Big => value.to_be_bytes(),
    };
    bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(2)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?
        .copy_from_slice(&encoded);
    Ok(())
}

fn write_u32(
    bytes: &mut [u8],
    offset: usize,
    value: u32,
    order: TiffByteOrder,
) -> Result<(), TiffDecodeError> {
    let encoded = match order {
        TiffByteOrder::Little => value.to_le_bytes(),
        TiffByteOrder::Big => value.to_be_bytes(),
    };
    bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(4)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?
        .copy_from_slice(&encoded);
    Ok(())
}

fn write_u64(
    bytes: &mut [u8],
    offset: usize,
    value: u64,
    order: TiffByteOrder,
) -> Result<(), TiffDecodeError> {
    let encoded = match order {
        TiffByteOrder::Little => value.to_le_bytes(),
        TiffByteOrder::Big => value.to_be_bytes(),
    };
    bytes
        .get_mut(
            offset
                ..offset
                    .checked_add(8)
                    .ok_or(TiffDecodeError::ArithmeticOverflow)?,
        )
        .ok_or_else(|| TiffDecodeError::Malformed("TIFF value is truncated".to_owned()))?
        .copy_from_slice(&encoded);
    Ok(())
}
