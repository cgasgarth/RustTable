use std::fs::File;
use std::io::{Cursor, Read};
use std::path::Path;

use image::{ImageFormat, ImageReader};
use rusttable_image::{
    DecodeLimits, DecodedImage, ImageDimensions, ImageInput, ImageInputError, ImageProbe,
    InputFormat,
};

pub struct FileImageInput {
    limits: DecodeLimits,
}

impl FileImageInput {
    #[must_use]
    pub const fn new(limits: DecodeLimits) -> Self {
        Self { limits }
    }

    #[must_use]
    pub const fn limits(&self) -> DecodeLimits {
        self.limits
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
        let actual = u64::try_from(bytes.len()).map_err(|_| ImageInputError::ArithmeticOverflow)?;
        if actual > self.limits.max_source_bytes() {
            return Err(ImageInputError::SourceTooLarge {
                limit: self.limits.max_source_bytes(),
                actual,
            });
        }
        Ok(bytes)
    }

    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        let format = detect_format(bytes)?;
        let dimensions = ImageReader::with_format(Cursor::new(bytes), image_format(format))
            .into_dimensions()
            .map_err(|error| malformed(format, &error))?;
        let dimensions = ImageDimensions::new(dimensions.0, dimensions.1)
            .map_err(|_| ImageInputError::ArithmeticOverflow)?;
        enforce_limits(self.limits, dimensions)?;
        Ok(ImageProbe::new(format, dimensions))
    }

    fn decode_bytes(&self, bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        let probe = self.probe_bytes(bytes)?;
        let decoded = ImageReader::with_format(Cursor::new(bytes), image_format(probe.format()))
            .decode()
            .map_err(|error| malformed(probe.format(), &error))?
            .to_rgba8();
        DecodedImage::new(probe.dimensions(), decoded.into_raw()).map_err(|error| match error {
            rusttable_image::DecodedImageError::ArithmeticOverflow => {
                ImageInputError::ArithmeticOverflow
            }
            rusttable_image::DecodedImageError::ByteLengthMismatch { expected, actual } => {
                ImageInputError::DecodedBufferInvariant { expected, actual }
            }
        })
    }
}

impl ImageInput for FileImageInput {
    fn probe_path(&self, path: &Path) -> Result<ImageProbe, ImageInputError> {
        self.probe_bytes(&self.read_source(path)?)
    }

    fn decode_path(&self, path: &Path) -> Result<DecodedImage, ImageInputError> {
        self.decode_bytes(&self.read_source(path)?)
    }
}

fn detect_format(bytes: &[u8]) -> Result<InputFormat, ImageInputError> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Ok(InputFormat::Jpeg);
    }
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Ok(InputFormat::Png);
    }
    Err(ImageInputError::UnsupportedSignature {
        signature: bytes.iter().copied().take(8).collect(),
    })
}

fn image_format(format: InputFormat) -> ImageFormat {
    match format {
        InputFormat::Jpeg => ImageFormat::Jpeg,
        InputFormat::Png => ImageFormat::Png,
    }
}

fn enforce_limits(
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

fn malformed(format: InputFormat, error: &image::ImageError) -> ImageInputError {
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
