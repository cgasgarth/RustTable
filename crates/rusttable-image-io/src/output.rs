use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::{ExtendedColorType, ImageEncoder};
use rusttable_image::{
    DecodedImage, DurableImageOutput, DurableImageOutputError, DurableOutputReceipt,
    DurableOutputStage, ImageOutput, ImageOutputError, JpegQuality, OutputFormat, OutputLimits,
    OutputOptions, OutputReceipt,
};
use rusttable_metadata::{
    CanonicalExifOutput, MetadataImageOutput, MetadataImageOutputError, MetadataOutput,
    MetadataOutputLimits,
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

impl MetadataImageOutput for FileImageOutput {
    fn write_new_with_metadata(
        &self,
        image: &DecodedImage,
        metadata: &rusttable_metadata::ImageMetadata,
        destination: &Path,
        options: OutputOptions,
        metadata_limits: MetadataOutputLimits,
    ) -> Result<OutputReceipt, MetadataImageOutputError> {
        if options.format() == OutputFormat::Tiff {
            return Err(MetadataImageOutputError::UnsupportedMetadataOutputFormat {
                format: OutputFormat::Tiff,
            });
        }
        if metadata.is_empty() {
            return self
                .write_new(image, destination, options)
                .map_err(image_output_error);
        }
        validate_destination(destination).map_err(image_output_error)?;
        if destination.exists() {
            return Err(image_output_error(ImageOutputError::DestinationExists {
                path: destination.to_owned(),
            }));
        }
        let encoded = encode(image, options, self.limits).map_err(image_output_error)?;
        let exif = CanonicalExifOutput::new(metadata_limits)
            .encode_exif(metadata)
            .map_err(|source| MetadataImageOutputError::MetadataSerializationFailure { source })?
            .ok_or(MetadataImageOutputError::MetadataSerializationFailure {
                source: rusttable_metadata::MetadataOutputError::InternalInvariant {
                    context: "nonempty metadata produced no EXIF payload",
                },
            })?;
        let encoded = insert_metadata(&encoded, options.format(), exif.as_bytes(), self.limits)?;
        let temporary = create_temporary(destination).map_err(image_output_error)?;
        publish(&temporary, destination, &encoded).map_err(image_output_error)?;
        OutputReceipt::new(
            destination.to_owned(),
            options.format(),
            image.dimensions(),
            u64::try_from(encoded.len()).unwrap_or(u64::MAX),
        )
        .map_err(|_| {
            image_output_error(ImageOutputError::EncodeFailure {
                format: options.format(),
            })
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
        #[cfg(not(windows))]
        let directory = {
            let parent = destination.parent().unwrap_or_else(|| Path::new("."));
            File::open(parent).map_err(|_| DurableImageOutputError::DurabilityUnsupported {
                destination: destination.to_owned(),
            })?
        };
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
        #[cfg(not(windows))]
        if sync_directory(&directory).is_err() {
            return Err(DurableImageOutputError::PublishedDirectorySyncFailure { receipt });
        }
        Ok(DurableOutputReceipt::new(receipt))
    }
}

#[cfg(not(windows))]
fn sync_directory(directory: &File) -> io::Result<()> {
    directory.sync_all()
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
        OutputOptions::Tiff => TiffEncoder::new(&mut writer)
            .write_image(
                image.pixels(),
                image.dimensions().width(),
                image.dimensions().height(),
                ExtendedColorType::Rgba8,
            )
            .map_err(|error| (OutputFormat::Tiff, error)),
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

fn image_output_error(source: ImageOutputError) -> MetadataImageOutputError {
    MetadataImageOutputError::BeforePublication { source }
}

fn insert_metadata(
    encoded: &[u8],
    format: OutputFormat,
    exif: &[u8],
    limits: OutputLimits,
) -> Result<Vec<u8>, MetadataImageOutputError> {
    let extra = match format {
        OutputFormat::Jpeg => {
            let actual = u64::try_from(exif.len())
                .ok()
                .and_then(|length| length.checked_add(8))
                .unwrap_or(u64::MAX);
            if actual > u64::from(u16::MAX) {
                return Err(MetadataImageOutputError::ExifFramingLimit {
                    format,
                    limit: u64::from(u16::MAX),
                    actual,
                });
            }
            10usize
                .checked_add(exif.len())
                .ok_or_else(|| image_output_error(ImageOutputError::AllocationFailure))?
        }
        OutputFormat::Png => 12usize
            .checked_add(exif.len())
            .ok_or_else(|| image_output_error(ImageOutputError::AllocationFailure))?,
        OutputFormat::Tiff | OutputFormat::JpegXl | OutputFormat::Webp => {
            return Err(image_output_error(ImageOutputError::EncodeFailure {
                format,
            }));
        }
    };
    let final_length = encoded
        .len()
        .checked_add(extra)
        .ok_or_else(|| image_output_error(ImageOutputError::AllocationFailure))?;
    let actual = u64::try_from(final_length).unwrap_or(u64::MAX);
    if actual > limits.max_encoded_bytes() {
        return Err(image_output_error(
            ImageOutputError::EncodedOutputTooLarge {
                limit: limits.max_encoded_bytes(),
                actual,
            },
        ));
    }
    match format {
        OutputFormat::Jpeg => insert_jpeg(encoded, exif, final_length),
        OutputFormat::Png => insert_png(encoded, exif, final_length),
        OutputFormat::Tiff => Err(image_output_error(ImageOutputError::EncodeFailure {
            format,
        })),
        OutputFormat::JpegXl | OutputFormat::Webp => {
            Err(image_output_error(ImageOutputError::EncodeFailure {
                format,
            }))
        }
    }
}

fn insert_jpeg(
    encoded: &[u8],
    exif: &[u8],
    final_length: usize,
) -> Result<Vec<u8>, MetadataImageOutputError> {
    validate_jpeg(encoded).map_err(|reason| {
        MetadataImageOutputError::MalformedEncodedContainer {
            format: OutputFormat::Jpeg,
            reason,
        }
    })?;
    let segment_length = u16::try_from(
        exif.len()
            .checked_add(8)
            .ok_or_else(|| image_output_error(ImageOutputError::AllocationFailure))?,
    )
    .map_err(|_| MetadataImageOutputError::ExifFramingLimit {
        format: OutputFormat::Jpeg,
        limit: u64::from(u16::MAX),
        actual: u64::try_from(exif.len()).unwrap_or(u64::MAX),
    })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(final_length)
        .map_err(|_| image_output_error(ImageOutputError::AllocationFailure))?;
    output.extend_from_slice(&encoded[..2]);
    output.extend_from_slice(&[0xff, 0xe1]);
    output.extend_from_slice(&segment_length.to_be_bytes());
    output.extend_from_slice(b"Exif\0\0");
    output.extend_from_slice(exif);
    output.extend_from_slice(&encoded[2..]);
    Ok(output)
}

fn validate_jpeg(encoded: &[u8]) -> Result<(), &'static str> {
    if encoded.len() < 4 || !encoded.starts_with(&[0xff, 0xd8]) {
        return Err("missing JPEG SOI");
    }
    let mut cursor = 2usize;
    while cursor < encoded.len() {
        if encoded[cursor] != 0xff {
            return Err("JPEG marker prefix missing");
        }
        while cursor < encoded.len() && encoded[cursor] == 0xff {
            cursor += 1;
        }
        let marker = *encoded.get(cursor).ok_or("truncated JPEG marker")?;
        cursor += 1;
        if marker == 0x00 {
            return Err("unexpected JPEG stuffed byte");
        }
        if marker == 0xd9 {
            return (cursor == encoded.len())
                .then_some(())
                .ok_or("JPEG data follows EOI");
        }
        if marker == 0xda {
            let end = jpeg_segment_end(encoded, cursor)?;
            if end > encoded.len() || !encoded.ends_with(&[0xff, 0xd9]) {
                return Err("JPEG scan is truncated");
            }
            return Ok(());
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            continue;
        }
        let end = jpeg_segment_end(encoded, cursor)?;
        let payload = &encoded[cursor + 2..end];
        if marker == 0xe1 && payload.starts_with(b"Exif\0\0") {
            return Err("unexpected pre-existing EXIF APP1");
        }
        cursor = end;
    }
    Err("JPEG EOI is missing")
}

fn jpeg_segment_end(encoded: &[u8], length_start: usize) -> Result<usize, &'static str> {
    let length_bytes = encoded
        .get(length_start..length_start + 2)
        .ok_or("truncated JPEG segment length")?;
    let length = usize::from(u16::from_be_bytes([length_bytes[0], length_bytes[1]]));
    if length < 2 {
        return Err("invalid JPEG segment length");
    }
    length_start
        .checked_add(length)
        .filter(|end| *end <= encoded.len())
        .ok_or("JPEG segment exceeds output")
}

fn insert_png(
    encoded: &[u8],
    exif: &[u8],
    final_length: usize,
) -> Result<Vec<u8>, MetadataImageOutputError> {
    let ihdr_end = validate_png(encoded).map_err(|reason| {
        MetadataImageOutputError::MalformedEncodedContainer {
            format: OutputFormat::Png,
            reason,
        }
    })?;
    let length =
        u32::try_from(exif.len()).map_err(|_| MetadataImageOutputError::ExifFramingLimit {
            format: OutputFormat::Png,
            limit: u64::from(u32::MAX),
            actual: u64::try_from(exif.len()).unwrap_or(u64::MAX),
        })?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(final_length)
        .map_err(|_| image_output_error(ImageOutputError::AllocationFailure))?;
    output.extend_from_slice(&encoded[..ihdr_end]);
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(b"eXIf");
    output.extend_from_slice(exif);
    output.extend_from_slice(&crc32_parts(b"eXIf", exif).to_be_bytes());
    output.extend_from_slice(&encoded[ihdr_end..]);
    Ok(output)
}

fn validate_png(encoded: &[u8]) -> Result<usize, &'static str> {
    const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if !encoded.starts_with(SIGNATURE) {
        return Err("missing PNG signature");
    }
    let mut cursor = 8usize;
    let mut ihdr_end = None;
    let mut saw_iend = false;
    while cursor < encoded.len() {
        let header = encoded
            .get(cursor..cursor + 8)
            .ok_or("truncated PNG chunk header")?;
        let length = usize::try_from(u32::from_be_bytes([
            header[0], header[1], header[2], header[3],
        ]))
        .map_err(|_| "PNG chunk length does not fit host size")?;
        let data_start = cursor.checked_add(8).ok_or("PNG chunk offset overflow")?;
        let data_end = data_start
            .checked_add(length)
            .ok_or("PNG chunk length overflow")?;
        let end = data_end.checked_add(4).ok_or("PNG chunk end overflow")?;
        if end > encoded.len() {
            return Err("PNG chunk exceeds output");
        }
        let kind = &header[4..8];
        let expected_crc = u32::from_be_bytes([
            encoded[data_end],
            encoded[data_end + 1],
            encoded[data_end + 2],
            encoded[data_end + 3],
        ]);
        if crc32(&encoded[cursor + 4..data_end]) != expected_crc {
            return Err("PNG chunk CRC mismatch");
        }
        match kind {
            b"IHDR" => {
                if cursor != 8 || ihdr_end.is_some() || length != 13 {
                    return Err("invalid PNG IHDR structure");
                }
                ihdr_end = Some(end);
            }
            b"eXIf" => return Err("unexpected pre-existing EXIF chunk"),
            b"IEND" => {
                if ihdr_end.is_none() || length != 0 || saw_iend || end != encoded.len() {
                    return Err("invalid PNG IEND structure");
                }
                saw_iend = true;
            }
            _ if ihdr_end.is_none() || saw_iend => return Err("invalid PNG chunk order"),
            _ => {}
        }
        cursor = end;
        if saw_iend {
            break;
        }
    }
    if !saw_iend {
        return Err("PNG IEND is missing");
    }
    ihdr_end.ok_or("PNG IHDR is missing")
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn crc32_parts(first: &[u8], second: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in first.iter().chain(second) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn rgba_to_opaque_rgb(
    image: &DecodedImage,
    _quality: JpegQuality,
) -> Result<Vec<u8>, ImageOutputError> {
    let pixels = image.pixels();
    for (index, pixel) in pixels.as_chunks::<4>().0.iter().enumerate() {
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
    for pixel in pixels.as_chunks::<4>().0 {
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
    position: u64,
    length: u64,
    limit_exceeded: Option<u64>,
    allocation_failed: bool,
}

impl LimitedWriter {
    fn new(limit: u64) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
            position: 0,
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
            .position
            .checked_add(incoming)
            .ok_or_else(|| io::Error::other("write length overflow"))?;
        if next > self.limit {
            self.limit_exceeded = Some(next);
            return Err(io::Error::other("encoded output limit exceeded"));
        }
        let end = usize::try_from(next).map_err(|_| io::Error::other("write length overflow"))?;
        if end > self.bytes.len() {
            self.bytes
                .try_reserve(end - self.bytes.len())
                .map_err(|_| {
                    self.allocation_failed = true;
                    io::Error::other("output allocation failed")
                })?;
            self.bytes.resize(end, 0);
        }
        let start = usize::try_from(self.position)
            .map_err(|_| io::Error::other("write position overflow"))?;
        self.bytes[start..end].copy_from_slice(bytes);
        self.position = next;
        self.length = self.length.max(next);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Seek for LimitedWriter {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let end = i128::try_from(self.bytes.len())
            .map_err(|_| io::Error::other("seek position overflow"))?;
        let current = i128::from(self.position);
        let target = match from {
            SeekFrom::Start(position) => i128::from(position),
            SeekFrom::Current(offset) => current + i128::from(offset),
            SeekFrom::End(offset) => end + i128::from(offset),
        };
        if target < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "seek before output start",
            ));
        }
        let target =
            u64::try_from(target).map_err(|_| io::Error::other("seek position overflow"))?;
        if target > self.limit {
            self.limit_exceeded = Some(target);
            return Err(io::Error::other("encoded output limit exceeded"));
        }
        self.position = target;
        Ok(target)
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

    #[test]
    fn metadata_container_failures_are_typed_and_bounded() {
        let limit = OutputLimits::new(u64::MAX).expect("nonzero limit");
        let oversized = vec![0; usize::from(u16::MAX) - 7];
        assert!(matches!(
            insert_metadata(
                &[0xff, 0xd8, 0xff, 0xd9],
                OutputFormat::Jpeg,
                &oversized,
                limit
            ),
            Err(MetadataImageOutputError::ExifFramingLimit {
                format: OutputFormat::Jpeg,
                limit: 65_535,
                actual: 65_536,
            })
        ));

        assert!(matches!(
            insert_metadata(
                &[0xff, 0xd8, 0xff, 0xe0, 0, 2],
                OutputFormat::Jpeg,
                &[1],
                limit,
            ),
            Err(MetadataImageOutputError::MalformedEncodedContainer {
                format: OutputFormat::Jpeg,
                ..
            })
        ));

        assert!(matches!(
            insert_metadata(b"\x89PNG\r\n\x1a\n", OutputFormat::Png, &[1], limit,),
            Err(MetadataImageOutputError::MalformedEncodedContainer {
                format: OutputFormat::Png,
                ..
            })
        ));
    }
}
