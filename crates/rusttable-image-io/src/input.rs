use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use image::{ImageFormat, ImageReader};
use rusttable_image::{
    DecodeLimits, DecodedImage, ImageDimensions, ImageInput, ImageInputError, ImageProbe,
    InputFormat, UnsupportedImageFeature,
};

use crate::ImageDecoderRegistry;

pub struct FileImageInput {
    limits: DecodeLimits,
    registry: ImageDecoderRegistry,
}

impl FileImageInput {
    #[must_use]
    pub const fn new(limits: DecodeLimits) -> Self {
        Self::with_registry(limits, ImageDecoderRegistry::standard())
    }

    #[must_use]
    pub const fn with_registry(limits: DecodeLimits, registry: ImageDecoderRegistry) -> Self {
        Self { limits, registry }
    }

    #[must_use]
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
    }

    #[must_use]
    pub const fn registry(&self) -> ImageDecoderRegistry {
        self.registry
    }

    fn read_source(&self, path: &Path) -> Result<Vec<u8>, ImageInputError> {
        let read_limit = self
            .limits
            .max_source_bytes()
            .checked_add(1)
            .ok_or(ImageInputError::ArithmeticOverflow)?;
        let mut file = File::open(path).map_err(|error| io_error(&error))?;
        let mut bytes = Vec::new();
        file.by_ref()
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|error| io_error(&error))?;
        self.enforce_source_limit(bytes.as_slice())?;
        Ok(bytes)
    }

    fn enforce_source_limit(&self, bytes: &[u8]) -> Result<(), ImageInputError> {
        let actual = u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
        if actual > self.limits.max_source_bytes() {
            return Err(ImageInputError::SourceTooLarge {
                limit: self.limits.max_source_bytes(),
                actual,
            });
        }
        Ok(())
    }

    fn probe_bytes_inner(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        self.enforce_source_limit(bytes)?;
        self.registry.probe_bytes(bytes, self.limits)
    }

    fn decode_bytes_inner(&self, bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        self.enforce_source_limit(bytes)?;
        self.registry.decode_bytes(bytes, self.limits)
    }
}

impl ImageInput for FileImageInput {
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        self.probe_bytes_inner(bytes)
    }

    fn decode_bytes(&self, bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        self.decode_bytes_inner(bytes)
    }

    fn probe_path(&self, path: &Path) -> Result<ImageProbe, ImageInputError> {
        self.probe_bytes_inner(&self.read_source(path)?)
    }

    fn decode_path(&self, path: &Path) -> Result<DecodedImage, ImageInputError> {
        self.decode_bytes_inner(&self.read_source(path)?)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TiffByteOrder {
    Little,
    Big,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TiffImageSpec {
    width: u32,
    height: u32,
}

pub(crate) fn probe_tiff(
    bytes: &[u8],
    limits: DecodeLimits,
) -> Result<ImageProbe, ImageInputError> {
    let spec = inspect_tiff(bytes)?;
    let dimensions = ImageDimensions::new(spec.width, spec.height)
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    enforce_limits(limits, dimensions)?;
    let decoded_dimensions = ImageReader::with_format(Cursor::new(bytes), ImageFormat::Tiff)
        .into_dimensions()
        .map_err(|error| malformed(InputFormat::Tiff, &error))?;
    if decoded_dimensions != (spec.width, spec.height) {
        return Err(malformed_tiff("decoder dimensions differ from TIFF IFD"));
    }
    Ok(ImageProbe::new(InputFormat::Tiff, dimensions))
}

#[derive(Debug)]
struct TiffDirectory {
    width: Option<u32>,
    height: Option<u32>,
    bits: Option<Vec<u64>>,
    samples: Option<u64>,
    photometric: Option<u64>,
    sample_format: Option<Vec<u64>>,
    planar: Option<u64>,
    strip_offsets: Option<Vec<u64>>,
    strip_byte_counts: Option<Vec<u64>>,
    next_offset: u32,
}

fn inspect_tiff(bytes: &[u8]) -> Result<TiffImageSpec, ImageInputError> {
    let order = match bytes.get(0..2) {
        Some([b'I', b'I']) => TiffByteOrder::Little,
        Some([b'M', b'M']) => TiffByteOrder::Big,
        _ => return Err(malformed_tiff("missing TIFF byte-order marker")),
    };
    if read_u16(bytes, 2, order)? != 42 {
        return Err(malformed_tiff("invalid classic TIFF magic"));
    }
    let ifd_offset = read_u32(bytes, 4, order)?;
    let directory = read_tiff_directory(bytes, ifd_offset, order)?;
    if directory.next_offset != 0 {
        return Err(ImageInputError::UnsupportedFeature {
            format: InputFormat::Tiff,
            reason: UnsupportedImageFeature::MultipleImages,
        });
    }

    let width = directory
        .width
        .ok_or_else(|| malformed_tiff("missing ImageWidth"))?;
    let height = directory
        .height
        .ok_or_else(|| malformed_tiff("missing ImageLength"))?;
    let bits = directory
        .bits
        .ok_or_else(|| malformed_tiff("missing BitsPerSample"))?;
    let samples = directory.samples.unwrap_or(1);
    let photometric = directory
        .photometric
        .ok_or_else(|| malformed_tiff("missing PhotometricInterpretation"))?;
    if bits.iter().any(|value| *value != 8) {
        return Err(unsupported_tiff(UnsupportedImageFeature::BitDepth));
    }
    if directory
        .sample_format
        .is_some_and(|values| values.iter().any(|value| *value != 1))
    {
        return Err(unsupported_tiff(UnsupportedImageFeature::SampleFormat));
    }
    if directory.planar.is_some_and(|value| value != 1) {
        return Err(unsupported_tiff(
            UnsupportedImageFeature::PlanarConfiguration,
        ));
    }
    validate_strip_ranges(
        bytes,
        directory.strip_offsets.as_deref(),
        directory.strip_byte_counts.as_deref(),
    )?;
    let supported_color_model = (photometric == 1 && (samples == 1 || samples == 2))
        || (photometric == 2 && (samples == 3 || samples == 4));
    if !supported_color_model || bits.len() != usize::try_from(samples).unwrap_or(usize::MAX) {
        return Err(unsupported_tiff(UnsupportedImageFeature::ColorModel));
    }
    Ok(TiffImageSpec { width, height })
}

fn read_tiff_directory(
    bytes: &[u8],
    ifd_offset: u32,
    order: TiffByteOrder,
) -> Result<TiffDirectory, ImageInputError> {
    let entry_count = usize::from(read_u16(
        bytes,
        usize::try_from(ifd_offset)
            .map_err(|_| malformed_tiff("IFD offset does not fit host size"))?,
        order,
    )?);
    let entries_start = usize::try_from(ifd_offset)
        .map_err(|_| malformed_tiff("IFD offset does not fit host size"))?
        .checked_add(2)
        .ok_or_else(|| malformed_tiff("IFD offset arithmetic overflowed"))?;
    let entries_bytes = entry_count
        .checked_mul(12)
        .ok_or_else(|| malformed_tiff("IFD entry arithmetic overflowed"))?;
    let entries_end = entries_start
        .checked_add(entries_bytes)
        .ok_or_else(|| malformed_tiff("IFD entry arithmetic overflowed"))?;
    if entries_end
        .checked_add(4)
        .is_none_or(|end| end > bytes.len())
    {
        return Err(malformed_tiff("IFD entries exceed the source"));
    }

    let mut directory = TiffDirectory {
        width: None,
        height: None,
        bits: None,
        samples: None,
        photometric: None,
        sample_format: None,
        planar: None,
        strip_offsets: None,
        strip_byte_counts: None,
        next_offset: 0,
    };
    for index in 0..entry_count {
        let entry_start = entries_start + index * 12;
        let tag = read_u16(bytes, entry_start, order)?;
        let field_type = read_u16(bytes, entry_start + 2, order)?;
        let count = read_u32(bytes, entry_start + 4, order)?;
        let values = match tag {
            256 | 257 | 258 | 262 | 273 | 277 | 279 | 284 | 339 => {
                read_tiff_values(bytes, entry_start + 8, field_type, count, order)?
            }
            _ => continue,
        };
        match tag {
            256 => directory.width = Some(single_dimension(&values, "ImageWidth")?),
            257 => directory.height = Some(single_dimension(&values, "ImageLength")?),
            258 => directory.bits = Some(values),
            262 => {
                directory.photometric = Some(single_value(&values, "PhotometricInterpretation")?);
            }
            273 => directory.strip_offsets = Some(values),
            277 => directory.samples = Some(single_value(&values, "SamplesPerPixel")?),
            279 => directory.strip_byte_counts = Some(values),
            284 => directory.planar = Some(single_value(&values, "PlanarConfiguration")?),
            339 => directory.sample_format = Some(values),
            _ => {}
        }
    }
    directory.next_offset = read_u32(bytes, entries_end, order)?;
    Ok(directory)
}

fn validate_strip_ranges(
    bytes: &[u8],
    offsets: Option<&[u64]>,
    byte_counts: Option<&[u64]>,
) -> Result<(), ImageInputError> {
    if let (Some(offsets), Some(byte_counts)) = (offsets, byte_counts) {
        if offsets.len() != byte_counts.len()
            || offsets.iter().zip(byte_counts).any(|(offset, count)| {
                usize::try_from(*offset)
                    .ok()
                    .and_then(|offset| {
                        usize::try_from(*count)
                            .ok()
                            .and_then(|count| offset.checked_add(count))
                    })
                    .is_none_or(|end| end > bytes.len())
            })
        {
            return Err(malformed_tiff("TIFF strip range exceeds the source"));
        }
    } else if offsets.is_some() || byte_counts.is_some() {
        return Err(malformed_tiff("TIFF strip tables are incomplete"));
    }
    Ok(())
}

fn read_tiff_values(
    bytes: &[u8],
    inline_start: usize,
    field_type: u16,
    count: u32,
    order: TiffByteOrder,
) -> Result<Vec<u64>, ImageInputError> {
    let width = match field_type {
        1 => 1usize,
        3 => 2,
        4 => 4,
        _ => return Err(malformed_tiff("unsupported TIFF field type")),
    };
    let count = usize::try_from(count)
        .map_err(|_| malformed_tiff("TIFF field count does not fit host size"))?;
    let byte_count = width
        .checked_mul(count)
        .ok_or_else(|| malformed_tiff("TIFF field size arithmetic overflowed"))?;
    let data = if byte_count <= 4 {
        let inline_end = inline_start
            .checked_add(byte_count)
            .ok_or_else(|| malformed_tiff("inline TIFF field arithmetic overflowed"))?;
        bytes
            .get(inline_start..inline_end)
            .ok_or_else(|| malformed_tiff("inline TIFF field is truncated"))?
    } else {
        let offset = usize::try_from(read_u32(bytes, inline_start, order)?)
            .map_err(|_| malformed_tiff("TIFF field offset does not fit host size"))?;
        let end = offset
            .checked_add(byte_count)
            .ok_or_else(|| malformed_tiff("TIFF field range arithmetic overflowed"))?;
        bytes
            .get(offset..end)
            .ok_or_else(|| malformed_tiff("TIFF field data is truncated"))?
    };
    (0..count)
        .map(|index| match field_type {
            1 => Ok(u64::from(data[index])),
            3 => Ok(u64::from(read_u16(data, index * 2, order)?)),
            4 => Ok(u64::from(read_u32(data, index * 4, order)?)),
            _ => unreachable!("field type was checked above"),
        })
        .collect()
}

fn read_u16(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u16, ImageInputError> {
    let bytes = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| malformed_tiff("TIFF field is truncated"))?;
    Ok(match order {
        TiffByteOrder::Little => u16::from_le_bytes([bytes[0], bytes[1]]),
        TiffByteOrder::Big => u16::from_be_bytes([bytes[0], bytes[1]]),
    })
}

fn read_u32(bytes: &[u8], offset: usize, order: TiffByteOrder) -> Result<u32, ImageInputError> {
    let bytes = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| malformed_tiff("TIFF field is truncated"))?;
    Ok(match order {
        TiffByteOrder::Little => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        TiffByteOrder::Big => u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    })
}

fn single_dimension(values: &[u64], name: &str) -> Result<u32, ImageInputError> {
    let value = single_value(values, name)?;
    u32::try_from(value).map_err(|_| malformed_tiff("TIFF dimension exceeds u32"))
}

fn single_value(values: &[u64], name: &str) -> Result<u64, ImageInputError> {
    values
        .first()
        .copied()
        .filter(|_| values.len() == 1)
        .ok_or_else(|| malformed_tiff(name))
}

fn unsupported_tiff(reason: UnsupportedImageFeature) -> ImageInputError {
    ImageInputError::UnsupportedFeature {
        format: InputFormat::Tiff,
        reason,
    }
}

fn malformed_tiff(message: &str) -> ImageInputError {
    ImageInputError::MalformedInput {
        format: InputFormat::Tiff,
        message: message.to_owned(),
    }
}

pub(crate) fn enforce_limits(
    limits: DecodeLimits,
    dimensions: ImageDimensions,
) -> Result<(), ImageInputError> {
    if dimensions.width() > limits.max_width() {
        return Err(ImageInputError::WidthLimit {
            actual: dimensions.width(),
            limit: limits.max_width(),
        });
    }
    if dimensions.height() > limits.max_height() {
        return Err(ImageInputError::HeightLimit {
            actual: dimensions.height(),
            limit: limits.max_height(),
        });
    }
    let pixels = dimensions
        .pixel_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if pixels > limits.max_pixel_count() {
        return Err(ImageInputError::PixelLimit {
            actual: pixels,
            limit: limits.max_pixel_count(),
        });
    }
    let decoded_bytes = dimensions
        .decoded_byte_count()
        .map_err(|_| ImageInputError::ArithmeticOverflow)?;
    if decoded_bytes > limits.max_decoded_bytes() {
        return Err(ImageInputError::DecodedByteLimit {
            actual: decoded_bytes,
            limit: limits.max_decoded_bytes(),
        });
    }
    Ok(())
}

pub(crate) fn malformed(format: InputFormat, error: &image::ImageError) -> ImageInputError {
    ImageInputError::MalformedInput {
        format,
        message: error.to_string(),
    }
}

fn io_error(error: &std::io::Error) -> ImageInputError {
    if error.kind() == std::io::ErrorKind::OutOfMemory {
        ImageInputError::AllocationFailure
    } else {
        ImageInputError::Io {
            message: error.to_string(),
        }
    }
}
