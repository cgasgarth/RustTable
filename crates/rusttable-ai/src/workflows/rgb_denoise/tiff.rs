#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "TIFF integer encoding intentionally clamps normalized floats to the sample range"
)]

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{self, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use tiff::encoder::colortype::{ColorType, RGB8, RGB16, RGB32Float, RGBA8, RGBA16, RGBA32Float};
use tiff::encoder::{Compression, DeflateLevel, TiffEncoder, TiffValue};
use tiff::tags::Tag;

use rusttable_image::ImageDimensions;

use super::ports::{PublishError, PublishedArtifact, RgbDenoisePublisher};
use super::{AlphaOutput, OutputBitDepth, RgbProfile, TiffCompression, TiffRecipe};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub struct FileTiffPublisher;

impl RgbDenoisePublisher for FileTiffPublisher {
    fn publish(
        &self,
        destination: &Path,
        recipe: &TiffRecipe,
        collision: super::CollisionPolicy,
        profile: &RgbProfile,
        pixels: &[[f32; 4]],
        dimensions: ImageDimensions,
        artifact_key: [u8; 32],
    ) -> Result<PublishedArtifact, PublishError> {
        let destination = resolve_destination(destination, collision, &artifact_key)?;
        if same_artifact(&destination, &artifact_key) {
            let encoded_bytes = fs::metadata(&destination)
                .map_err(|error| PublishError::Io(error.to_string()))?
                .len();
            return Ok(PublishedArtifact {
                destination,
                encoded_bytes,
            });
        }
        let parent = destination.parent().unwrap_or_else(|| Path::new("."));
        if !parent.is_dir() {
            return Err(PublishError::InvalidDestination);
        }
        let temporary = temporary_path(&destination);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| PublishError::Io(error.to_string()))?;
        let mut writer = LimitedFile::new(file, recipe.max_encoded_bytes());
        let result = encode_tiff(
            &mut writer,
            recipe,
            profile,
            pixels,
            dimensions,
            artifact_key,
        );
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
        writer
            .file
            .sync_all()
            .map_err(|error| cleanup_error(&temporary, PublishError::Io(error.to_string())))?;
        let encoded_bytes = writer.written;
        drop(writer);
        verify_tiff(&temporary, dimensions, profile, artifact_key)?;
        fs::rename(&temporary, &destination)
            .map_err(|error| cleanup_error(&temporary, PublishError::Io(error.to_string())))?;
        sync_parent(parent).map_err(|error| PublishError::Io(error.to_string()))?;
        Ok(PublishedArtifact {
            destination,
            encoded_bytes,
        })
    }
}

fn encode_tiff(
    writer: &mut LimitedFile,
    recipe: &TiffRecipe,
    profile: &RgbProfile,
    pixels: &[[f32; 4]],
    dimensions: ImageDimensions,
    artifact_key: [u8; 32],
) -> Result<(), PublishError> {
    let description = format!("RustTable rgb-denoise v1 key={}", hex_key(artifact_key));
    match (recipe.bit_depth(), recipe.alpha()) {
        (OutputBitDepth::Eight, AlphaOutput::Opaque) => encode_typed::<RGB8>(
            writer,
            recipe,
            profile,
            &quantize_u8(pixels, false),
            dimensions,
            &description,
            false,
        ),
        (OutputBitDepth::Eight, AlphaOutput::PreserveStraight) => encode_typed::<RGBA8>(
            writer,
            recipe,
            profile,
            &quantize_u8(pixels, true),
            dimensions,
            &description,
            true,
        ),
        (OutputBitDepth::Sixteen, AlphaOutput::Opaque) => encode_typed::<RGB16>(
            writer,
            recipe,
            profile,
            &quantize_u16(pixels, false),
            dimensions,
            &description,
            false,
        ),
        (OutputBitDepth::Sixteen, AlphaOutput::PreserveStraight) => encode_typed::<RGBA16>(
            writer,
            recipe,
            profile,
            &quantize_u16(pixels, true),
            dimensions,
            &description,
            true,
        ),
        (OutputBitDepth::ThirtyTwoFloat, AlphaOutput::Opaque) => encode_typed::<RGB32Float>(
            writer,
            recipe,
            profile,
            &quantize_f32(pixels, false),
            dimensions,
            &description,
            false,
        ),
        (OutputBitDepth::ThirtyTwoFloat, AlphaOutput::PreserveStraight) => {
            encode_typed::<RGBA32Float>(
                writer,
                recipe,
                profile,
                &quantize_f32(pixels, true),
                dimensions,
                &description,
                true,
            )
        }
    }
}

fn encode_typed<C: ColorType>(
    writer: &mut LimitedFile,
    recipe: &TiffRecipe,
    profile: &RgbProfile,
    data: &[C::Inner],
    dimensions: ImageDimensions,
    description: &str,
    _alpha: bool,
) -> Result<(), PublishError>
where
    [C::Inner]: TiffValue,
{
    let compression = match recipe.compression() {
        TiffCompression::Uncompressed => Compression::Uncompressed,
        TiffCompression::DeflateBalanced => Compression::Deflate(DeflateLevel::Balanced),
        TiffCompression::PackBits => Compression::Packbits,
    };
    let mut encoder = TiffEncoder::new(writer)
        .map_err(encode_error)?
        .with_compression(compression);
    let mut image = encoder
        .new_image::<C>(dimensions.width(), dimensions.height())
        .map_err(encode_error)?;
    image
        .encoder()
        .write_tag(Tag::IccProfile, profile.icc_profile())
        .map_err(encode_error)?;
    image
        .encoder()
        .write_tag(Tag::ImageDescription, description)
        .map_err(encode_error)?;
    image.write_data(data).map_err(encode_error)
}

fn quantize_u8(pixels: &[[f32; 4]], alpha: bool) -> Vec<u8> {
    let channels = if alpha { 4 } else { 3 };
    let mut output = Vec::with_capacity(pixels.len() * channels);
    for pixel in pixels {
        output.extend(pixel[..3].iter().map(|value| quantize(*value, 255.0) as u8));
        if alpha {
            output.push(quantize(pixel[3], 255.0) as u8);
        }
    }
    output
}

fn quantize_u16(pixels: &[[f32; 4]], alpha: bool) -> Vec<u16> {
    let channels = if alpha { 4 } else { 3 };
    let mut output = Vec::with_capacity(pixels.len() * channels);
    for pixel in pixels {
        output.extend(
            pixel[..3]
                .iter()
                .map(|value| quantize(*value, 65_535.0) as u16),
        );
        if alpha {
            output.push(quantize(pixel[3], 65_535.0) as u16);
        }
    }
    output
}

fn quantize_f32(pixels: &[[f32; 4]], alpha: bool) -> Vec<f32> {
    let channels = if alpha { 4 } else { 3 };
    let mut output = Vec::with_capacity(pixels.len() * channels);
    for pixel in pixels {
        output.extend_from_slice(&pixel[..3]);
        if alpha {
            output.push(pixel[3]);
        }
    }
    output
}

fn quantize(value: f32, maximum: f32) -> f32 {
    value.clamp(0.0, 1.0) * maximum
}

fn resolve_destination(
    destination: &Path,
    collision: super::CollisionPolicy,
    artifact_key: &[u8; 32],
) -> Result<PathBuf, PublishError> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if destination.file_name().is_none() {
        return Err(PublishError::InvalidDestination);
    }
    for suffix in 0..10_000_u32 {
        let candidate = if suffix == 0 {
            destination.to_owned()
        } else {
            let stem = destination
                .file_stem()
                .ok_or(PublishError::InvalidDestination)?
                .to_string_lossy();
            let extension = destination
                .extension()
                .map(|value| format!(".{}", value.to_string_lossy()))
                .unwrap_or_default();
            parent.join(format!("{stem}-{suffix}{extension}"))
        };
        if !candidate.exists() {
            return Ok(candidate);
        }
        if same_artifact(&candidate, artifact_key) {
            return Ok(candidate);
        }
        if matches!(collision, super::CollisionPolicy::Fail) {
            return Err(PublishError::DestinationExists);
        }
    }
    Err(PublishError::DestinationExists)
}

fn same_artifact(path: &Path, key: &[u8; 32]) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let Ok(mut decoder) = tiff::decoder::Decoder::new(file) else {
        return false;
    };
    let Ok(description) = decoder.get_tag_ascii_string(Tag::ImageDescription) else {
        return false;
    };
    description.contains(&format!("key={}", hex_key(*key)))
}

fn verify_tiff(
    path: &Path,
    dimensions: ImageDimensions,
    profile: &RgbProfile,
    key: [u8; 32],
) -> Result<(), PublishError> {
    let file = File::open(path)
        .map_err(|error| cleanup_error(path, PublishError::Io(error.to_string())))?;
    let mut decoder = tiff::decoder::Decoder::new(file)
        .map_err(|error| cleanup_error(path, PublishError::Probe(error.to_string())))?;
    let actual = decoder
        .dimensions()
        .map_err(|error| cleanup_error(path, PublishError::Probe(error.to_string())))?;
    if actual != (dimensions.width(), dimensions.height()) {
        return Err(cleanup_error(
            path,
            PublishError::Probe("TIFF dimensions differ from the rendered image".to_owned()),
        ));
    }
    let description = decoder
        .get_tag_ascii_string(Tag::ImageDescription)
        .map_err(|error| cleanup_error(path, PublishError::Probe(error.to_string())))?;
    if !description.contains(&format!("key={}", hex_key(key))) {
        return Err(cleanup_error(
            path,
            PublishError::Probe("TIFF artifact identity is missing".to_owned()),
        ));
    }
    let embedded = decoder
        .get_tag_u8_vec(Tag::IccProfile)
        .map_err(|error| cleanup_error(path, PublishError::Probe(error.to_string())))?;
    if embedded != profile.icc_profile() {
        return Err(cleanup_error(
            path,
            PublishError::Probe("TIFF ICC profile does not match the recipe".to_owned()),
        ));
    }
    if decoder.more_images() {
        return Err(cleanup_error(
            path,
            PublishError::Probe("TIFF has more than one page".to_owned()),
        ));
    }
    Ok(())
}

fn temporary_path(destination: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = destination.file_name().map_or_else(
        || "output.tiff".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    );
    destination
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".{name}.rusttable-ai-{sequence}.tmp"))
}

fn cleanup_error(path: &Path, error: PublishError) -> PublishError {
    let _ = fs::remove_file(path);
    error
}

#[cfg(not(windows))]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(windows)]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn encode_error(error: impl std::fmt::Display) -> PublishError {
    PublishError::Encode(error.to_string())
}

fn hex_key(key: [u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in key {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

struct LimitedFile {
    file: File,
    written: u64,
    limit: u64,
}

impl LimitedFile {
    const fn new(file: File, limit: u64) -> Self {
        Self {
            file,
            written: 0,
            limit,
        }
    }
}

impl Write for LimitedFile {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let length = u64::try_from(bytes.len()).map_err(|_| io::Error::other("write overflow"))?;
        let next = self
            .written
            .checked_add(length)
            .ok_or_else(|| io::Error::other("write overflow"))?;
        if next > self.limit {
            return Err(io::Error::other(
                "encoded TIFF exceeds configured byte limit",
            ));
        }
        let written = self.file.write(bytes)?;
        self.written = self
            .written
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

impl std::io::Seek for LimitedFile {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}
