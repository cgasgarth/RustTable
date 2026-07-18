use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::{ExtendedColorType, ImageEncoder};
use rusttable_image::{
    DecodedImage, DurableImageOutput, DurableImageOutputError, DurableOutputReceipt,
    DurableOutputStage, ImageOutput, ImageOutputError, JpegQuality, OutputFormat, OutputLimits,
    OutputOptions, OutputReceipt,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub struct FileImageOutput {
    limits: OutputLimits,
}

impl FileImageOutput {
    #[must_use]
    pub const fn new(limits: OutputLimits) -> Self {
        Self { limits }
    }

    #[must_use]
    pub const fn limits(&self) -> OutputLimits {
        self.limits
    }
}

impl ImageOutput for FileImageOutput {
    fn write_new(
        &self,
        image: &DecodedImage,
        destination: &Path,
        options: OutputOptions,
    ) -> Result<OutputReceipt, ImageOutputError> {
        validate_destination(destination)?;
        if destination.exists() {
            return Err(ImageOutputError::DestinationExists {
                path: destination.to_owned(),
            });
        }
        let encoded = encode(image, options, self.limits)?;
        let temporary = create_temporary(destination)?;
        publish(&temporary, destination, &encoded)?;
        OutputReceipt::new(
            destination.to_owned(),
            options.format(),
            image.dimensions(),
            u64::try_from(encoded.len()).unwrap_or(u64::MAX),
        )
        .map_err(|_| ImageOutputError::EncodeFailure {
            format: options.format(),
        })
    }
}

impl DurableImageOutput for FileImageOutput {
    fn write_new_durable(
        &self,
        image: &DecodedImage,
        destination: &Path,
        options: OutputOptions,
    ) -> Result<DurableOutputReceipt, DurableImageOutputError> {
        validate_destination(destination)
            .map_err(|source| DurableImageOutputError::BeforePublication { source })?;
        if destination.exists() {
            return Err(DurableImageOutputError::BeforePublication {
                source: ImageOutputError::DestinationExists {
                    path: destination.to_owned(),
                },
            });
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        let directory =
            File::open(parent).map_err(|_| DurableImageOutputError::DurabilityUnsupported {
                destination: destination.to_owned(),
            })?;
        let encoded = encode(image, options, self.limits)
            .map_err(|source| DurableImageOutputError::BeforePublication { source })?;
        let temporary = create_temporary(destination)
            .map_err(|source| DurableImageOutputError::BeforePublication { source })?;
        if let Err(source) = write_and_sync(&temporary, &encoded) {
            return Err(cleanup_before_publication(
                &temporary,
                destination,
                DurableOutputStage::Write,
                source,
            ));
        }
        if let Err(source) = fs::hard_link(&temporary, destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                ImageOutputError::DestinationExists {
                    path: destination.to_owned(),
                }
            } else {
                ImageOutputError::PublishFailure
            }
        }) {
            return Err(cleanup_before_publication(
                &temporary,
                destination,
                DurableOutputStage::Publication,
                source,
            ));
        }
        let receipt = OutputReceipt::new(
            destination.to_owned(),
            options.format(),
            image.dimensions(),
            u64::try_from(encoded.len()).unwrap_or(u64::MAX),
        )
        .map_err(|_| DurableImageOutputError::PublishedDirectorySyncFailure {
            receipt: OutputReceipt::new(
                destination.to_owned(),
                options.format(),
                image.dimensions(),
                1,
            )
            .expect("nonzero fallback receipt"),
        })?;
        if fs::remove_file(&temporary).is_err() {
            return Err(DurableImageOutputError::PublishedTemporaryCleanupFailure { receipt });
        }
        if directory.sync_all().is_err() {
            return Err(DurableImageOutputError::PublishedDirectorySyncFailure { receipt });
        }
        Ok(DurableOutputReceipt::new(receipt))
    }
}

fn validate_destination(destination: &Path) -> Result<(), ImageOutputError> {
    if destination.file_name().is_none() {
        return Err(ImageOutputError::InvalidDestination {
            path: destination.to_owned(),
        });
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(ImageOutputError::MissingDestinationParent {
            path: destination.to_owned(),
        });
    }
    Ok(())
}

fn encode(
    image: &DecodedImage,
    options: OutputOptions,
    limits: OutputLimits,
) -> Result<Vec<u8>, ImageOutputError> {
    let mut writer = LimitedWriter::new(limits.max_encoded_bytes());
    let result = match options {
        OutputOptions::Png => PngEncoder::new(&mut writer)
            .write_image(
                image.pixels(),
                image.dimensions().width(),
                image.dimensions().height(),
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| (OutputFormat::Png, error)),
        OutputOptions::Jpeg { quality } => {
            let rgb = rgba_to_opaque_rgb(image, quality)?;
            JpegEncoder::new_with_quality(&mut writer, quality.get())
                .write_image(
                    &rgb,
                    image.dimensions().width(),
                    image.dimensions().height(),
                    ExtendedColorType::Rgb8,
                )
                .map_err(|error| (OutputFormat::Jpeg, error))
        }
    };
    if let Err((format, error)) = result {
        return Err(map_encode_error(&writer, format, error));
    }
    if writer.length == 0 {
        return Err(ImageOutputError::EncodeFailure {
            format: options.format(),
        });
    }
    Ok(writer.bytes)
}

fn rgba_to_opaque_rgb(
    image: &DecodedImage,
    _quality: JpegQuality,
) -> Result<Vec<u8>, ImageOutputError> {
    let pixels = image.pixels();
    for (index, pixel) in pixels.chunks_exact(4).enumerate() {
        if pixel[3] != 255 {
            return Err(ImageOutputError::NonOpaqueJpegInput {
                pixel_index: u64::try_from(index).unwrap_or(u64::MAX),
            });
        }
    }
    let rgb_length = pixels
        .len()
        .checked_div(4)
        .and_then(|count| count.checked_mul(3))
        .ok_or(ImageOutputError::AllocationFailure)?;
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(rgb_length)
        .map_err(|_| ImageOutputError::AllocationFailure)?;
    for pixel in pixels.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    Ok(rgb)
}

fn map_encode_error(
    writer: &LimitedWriter,
    format: OutputFormat,
    _error: image::ImageError,
) -> ImageOutputError {
    if let Some(actual) = writer.limit_exceeded {
        return ImageOutputError::EncodedOutputTooLarge {
            limit: writer.limit,
            actual,
        };
    }
    if writer.allocation_failed {
        return ImageOutputError::AllocationFailure;
    }
    ImageOutputError::EncodeFailure { format }
}

struct LimitedWriter {
    bytes: Vec<u8>,
    limit: u64,
    length: u64,
    limit_exceeded: Option<u64>,
    allocation_failed: bool,
}

impl LimitedWriter {
    fn new(limit: u64) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            length: 0,
            limit_exceeded: None,
            allocation_failed: false,
        }
    }
}

impl Write for LimitedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let incoming =
            u64::try_from(bytes.len()).map_err(|_| io::Error::other("write length overflow"))?;
        let next = self
            .length
            .checked_add(incoming)
            .ok_or_else(|| io::Error::other("write length overflow"))?;
        if next > self.limit {
            self.limit_exceeded = Some(next);
            return Err(io::Error::other("encoded output limit exceeded"));
        }
        self.bytes.try_reserve(bytes.len()).map_err(|_| {
            self.allocation_failed = true;
            io::Error::other("output allocation failed")
        })?;
        self.bytes.extend_from_slice(bytes);
        self.length = next;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn create_temporary(destination: &Path) -> Result<PathBuf, ImageOutputError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".rusttable-output-{sequence:016x}.tmp");
        let path = parent.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(ImageOutputError::TemporaryFileCreationFailure),
        }
    }
    Err(ImageOutputError::TemporaryFileCreationFailure)
}

fn publish(temporary: &Path, destination: &Path, bytes: &[u8]) -> Result<(), ImageOutputError> {
    let result = (|| {
        let mut file = File::create(temporary).map_err(|_| ImageOutputError::WriteFailure)?;
        file.write_all(bytes)
            .map_err(|_| ImageOutputError::WriteFailure)?;
        file.flush().map_err(|_| ImageOutputError::WriteFailure)?;
        file.sync_all().map_err(|_| ImageOutputError::SyncFailure)?;
        fs::hard_link(temporary, destination).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                ImageOutputError::DestinationExists {
                    path: destination.to_owned(),
                }
            } else {
                ImageOutputError::PublishFailure
            }
        })?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}

fn write_and_sync(temporary: &Path, bytes: &[u8]) -> Result<(), ImageOutputError> {
    let mut file = File::create(temporary).map_err(|_| ImageOutputError::WriteFailure)?;
    file.write_all(bytes)
        .map_err(|_| ImageOutputError::WriteFailure)?;
    file.flush().map_err(|_| ImageOutputError::WriteFailure)?;
    file.sync_all().map_err(|_| ImageOutputError::SyncFailure)
}

fn cleanup_before_publication(
    temporary: &Path,
    destination: &Path,
    stage: DurableOutputStage,
    source: ImageOutputError,
) -> DurableImageOutputError {
    match fs::remove_file(temporary) {
        Ok(()) => DurableImageOutputError::BeforePublication { source },
        Err(_) => DurableImageOutputError::BeforePublicationCleanupFailure {
            destination: destination.to_owned(),
            stage,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_writer_accepts_exact_bound_and_rejects_next_append() {
        let mut writer = LimitedWriter::new(3);
        assert!(writer.write_all(&[1, 2, 3]).is_ok());
        assert!(writer.write_all(&[4]).is_err());
        assert_eq!(writer.limit_exceeded, Some(4));
    }

    #[test]
    fn durable_receipt_is_constructed_only_from_a_completed_output_receipt() {
        let output = OutputReceipt::new(
            PathBuf::from("published.png"),
            OutputFormat::Png,
            rusttable_image::ImageDimensions::new(1, 1).expect("dimensions"),
            4,
        )
        .expect("receipt");
        let durable = DurableOutputReceipt::new(output.clone());
        assert_eq!(durable.output(), &output);
        assert_eq!(
            durable.durability(),
            rusttable_image::DurableOutputTag::FileAndParentDirectorySynchronized
        );
    }

    #[test]
    fn temporary_cleanup_failure_is_not_downgraded_to_a_prepublication_error() {
        let error = cleanup_before_publication(
            Path::new("missing-temporary"),
            Path::new("destination.png"),
            DurableOutputStage::Write,
            ImageOutputError::WriteFailure,
        );
        assert!(matches!(
            error,
            DurableImageOutputError::BeforePublicationCleanupFailure { .. }
        ));
    }
}
