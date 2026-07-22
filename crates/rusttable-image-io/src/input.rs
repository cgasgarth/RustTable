use std::fs::File;
use std::io::Read;
use std::path::Path;

use rusttable_image::{
    DecodeLimits, DecodedFrame, DecodedImage, ImageDimensions, ImageInput, ImageInputError,
    ImageProbe,
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

    fn decode_frame_bytes_inner(&self, bytes: &[u8]) -> Result<DecodedFrame, ImageInputError> {
        self.enforce_source_limit(bytes)?;
        if crate::raw::is_raw(bytes) {
            return crate::raw::decode_raw_legacy_frame(bytes, self.limits);
        }
        self.registry.decode_frame_bytes(bytes, self.limits)
    }

    /// Decodes the canonical scene-linear RAW frame used by preview and export.
    ///
    /// The [`ImageInput`] compatibility method retains its historical RGBA8
    /// projection for existing catalog/UI callers. New renderers use this
    /// method so preview and export share one checked F32 RAW input.
    ///
    /// # Errors
    ///
    /// Returns the selected decoder's typed probe, decode, limit, or contract
    /// failure.
    pub fn decode_linear_frame_bytes(&self, bytes: &[u8]) -> Result<DecodedFrame, ImageInputError> {
        self.enforce_source_limit(bytes)?;
        self.registry.decode_frame_bytes(bytes, self.limits)
    }
}

impl ImageInput for FileImageInput {
    fn probe_bytes(&self, bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        self.probe_bytes_inner(bytes)
    }

    fn decode_bytes(&self, bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        self.decode_bytes_inner(bytes)
    }

    fn decode_frame_bytes(&self, bytes: &[u8]) -> Result<DecodedFrame, ImageInputError> {
        self.decode_frame_bytes_inner(bytes)
    }

    fn probe_path(&self, path: &Path) -> Result<ImageProbe, ImageInputError> {
        self.probe_bytes_inner(&self.read_source(path)?)
    }

    fn decode_path(&self, path: &Path) -> Result<DecodedImage, ImageInputError> {
        self.decode_bytes_inner(&self.read_source(path)?)
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

fn io_error(error: &std::io::Error) -> ImageInputError {
    if error.kind() == std::io::ErrorKind::OutOfMemory {
        ImageInputError::AllocationFailure
    } else {
        ImageInputError::Io {
            message: error.to_string(),
        }
    }
}
