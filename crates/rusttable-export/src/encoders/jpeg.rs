//! Safe, streaming JPEG export over the canonical artifact boundary.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use mozjpeg_rs::{Encoder as CodecEncoder, PixelDensity, Preset, Subsampling};
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDimensions, SampleType, StorageLayout,
};
use sha2::{Digest, Sha256};

use crate::{CanonicalArtifact, Density, DensityUnit};

pub const ENCODER_ID: &str = "jpeg";
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_SEGMENT_BYTES: usize = 65_533;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSubsampling {
    FourFourFour,
    FourTwoTwo,
    FourTwoZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbnailPolicy {
    Include,
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub quality: u8,
    pub subsampling: ChromaSubsampling,
    pub progressive: bool,
    pub optimize_huffman: bool,
    pub restart_interval: u16,
    pub density: Option<Density>,
    pub metadata: MetadataPolicy,
    pub thumbnail: ThumbnailPolicy,
    pub max_metadata_bytes: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            quality: 90,
            subsampling: ChromaSubsampling::FourFourFour,
            progressive: true,
            optimize_huffman: true,
            restart_interval: 0,
            density: None,
            metadata: MetadataPolicy::All,
            thumbnail: ThumbnailPolicy::Include,
            max_metadata_bytes: MAX_METADATA_BYTES,
        }
    }
}

impl Settings {
    /// Validates settings before any destination or codec state is touched.
    ///
    /// # Errors
    ///
    /// Returns an error when quality or the metadata bound is outside the contract.
    pub fn validate(self) -> Result<(), Error> {
        if !(1..=100).contains(&self.quality) {
            return Err(Error::InvalidSettings("quality must be 1..=100"));
        }
        if self.max_metadata_bytes == 0 || self.max_metadata_bytes > MAX_METADATA_BYTES {
            return Err(Error::InvalidSettings("metadata limit"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub layouts: &'static [ChannelLayout],
    pub controls: &'static [&'static str],
    pub metadata: &'static [&'static str],
}

#[must_use]
pub const fn capabilities() -> Capabilities {
    Capabilities {
        layouts: &[ChannelLayout::Gray, ChannelLayout::Rgb],
        controls: &[
            "quality",
            "subsampling",
            "progressive",
            "optimize_huffman",
            "restart_interval",
            "density",
        ],
        metadata: &["icc", "exif", "xmp", "iptc"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: ImageDimensions,
    pub encoded_bytes: u64,
    pub artifact_sha256: [u8; 32],
    pub marker_manifest: Vec<String>,
    pub codec: &'static str,
    pub quality: u8,
    pub subsampling: ChromaSubsampling,
    pub progressive: bool,
    pub optimize_huffman: bool,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedByteOrder(ByteOrder),
    UnsupportedOrientation,
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    MetadataTooLarge(&'static str),
    MetadataTextUnsupported,
    InvalidDensity,
    InvalidDimensions(ImageDimensions),
    ImageView,
    Codec(String),
    Io(io::Error),
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(message) => write!(formatter, "invalid JPEG settings: {message}"),
            Self::UnsupportedLayout(layout) => {
                write!(formatter, "unsupported JPEG layout: {layout:?}")
            }
            Self::UnsupportedSample(sample) => {
                write!(formatter, "unsupported JPEG sample type: {sample:?}")
            }
            Self::UnsupportedAlpha(alpha) => {
                write!(formatter, "unsupported JPEG alpha mode: {alpha:?}")
            }
            Self::UnsupportedStorage(storage) => {
                write!(formatter, "unsupported JPEG storage: {storage:?}")
            }
            Self::UnsupportedByteOrder(order) => {
                write!(formatter, "unsupported JPEG byte order: {order:?}")
            }
            Self::UnsupportedOrientation => {
                formatter.write_str("JPEG input orientation is not canonical")
            }
            Self::EmptyProfile => formatter.write_str("JPEG ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "JPEG metadata is {actual} bytes, limit is {limit}"
            ),
            Self::MetadataTooLarge(kind) => {
                write!(formatter, "JPEG {kind} metadata exceeds marker limits")
            }
            Self::MetadataTextUnsupported => {
                formatter.write_str("JPEG text metadata needs a canonical packet")
            }
            Self::InvalidDensity => formatter.write_str("JPEG density cannot be represented"),
            Self::InvalidDimensions(dimensions) => write!(
                formatter,
                "JPEG dimensions {}x{} exceed the JPEG limit",
                dimensions.width(),
                dimensions.height()
            ),
            Self::ImageView => formatter.write_str("canonical artifact image view is invalid"),
            Self::Codec(message) => write!(formatter, "JPEG codec failed: {message}"),
            Self::Io(error) => write!(formatter, "JPEG I/O failed: {error}"),
            Self::Malformed(message) => write!(formatter, "malformed JPEG: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Encoder {
    settings: Settings,
}

impl Encoder {
    #[must_use]
    pub const fn new(settings: Settings) -> Self {
        Self { settings }
    }

    #[must_use]
    pub const fn settings(self) -> Settings {
        self.settings
    }

    /// Encodes into an owned buffer and validates the JPEG marker contract.
    ///
    /// # Errors
    ///
    /// Returns an error when the artifact, settings, codec, or marker contract is invalid.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        let mut bytes = Vec::new();
        let receipt = self.encode_into(artifact, &mut bytes)?;
        inspect(&bytes, receipt)
    }

    /// Streams a complete JPEG into an already staged writer.
    ///
    /// # Errors
    ///
    /// Returns an error when the artifact, settings, codec, or writer is invalid.
    pub fn encode_to_writer<W: Write>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        self.encode_into(artifact, writer)
    }

    /// Streams a JPEG with a row-independent cancellation checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, cancellation, codec, or writer handling fails.
    pub fn encode_to_writer_with_cancel<W: Write, C: FnMut() -> bool>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
        mut cancelled: C,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        if cancelled() {
            return Err(Error::Codec("cancelled before JPEG encoding".to_owned()));
        }
        self.encode_into_with_cancel(artifact, writer, &mut cancelled)
    }

    /// Encodes to a queue staging path and removes partial output on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, encoding, verification, or staging I/O fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        let result = (|| {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            let receipt = self.encode_into(artifact, &mut writer)?;
            writer.flush()?;
            writer
                .into_inner()
                .map_err(|error| Error::Io(error.into_error()))?
                .sync_all()?;
            let bytes = fs::read(path)?;
            inspect(&bytes, receipt).map(|(_, receipt)| receipt)
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    fn encode_into<W: Write>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
    ) -> Result<Receipt, Error> {
        let mut cancelled = || false;
        self.encode_into_with_cancel(artifact, writer, &mut cancelled)
    }

    fn encode_into_with_cancel<W: Write, C: FnMut() -> bool>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
        cancelled: &mut C,
    ) -> Result<Receipt, Error> {
        let view = artifact.view().map_err(|_| Error::ImageView)?;
        let dimensions = view.descriptor().dimensions();
        let mut pixels = Vec::new();
        let width = usize::try_from(dimensions.width()).map_err(|_| Error::ImageView)?;
        let height = usize::try_from(dimensions.height()).map_err(|_| Error::ImageView)?;
        let channels = match view.descriptor().format().channels() {
            ChannelLayout::Gray => 1,
            ChannelLayout::Rgb => 3,
            layout => return Err(Error::UnsupportedLayout(layout)),
        };
        let row_bytes = width.checked_mul(channels).ok_or(Error::ImageView)?;
        let image_bytes = row_bytes.checked_mul(height).ok_or(Error::ImageView)?;
        pixels.reserve(image_bytes);
        for y in 0..height {
            if cancelled() {
                return Err(Error::Codec(
                    "cancelled during JPEG scan preparation".to_owned(),
                ));
            }
            let row = view
                .row(0, u32::try_from(y).map_err(|_| Error::ImageView)?)
                .map_err(|_| Error::ImageView)?;
            pixels.extend_from_slice(&row[..row_bytes]);
        }
        if cancelled() {
            return Err(Error::Codec("cancelled before JPEG scan".to_owned()));
        }
        let encoder = codec(self.settings, artifact)?;
        let mut sink = DigestWriter::new(writer);
        let result = match channels {
            1 => encoder.encode_gray_to_writer(
                &pixels,
                dimensions.width(),
                dimensions.height(),
                &mut sink,
            ),
            3 => encoder.encode_rgb_to_writer(
                &pixels,
                dimensions.width(),
                dimensions.height(),
                &mut sink,
            ),
            _ => unreachable!("validated JPEG channel layout"),
        };
        result.map_err(|error| Error::Codec(error.to_string()))?;
        let (encoded_bytes, artifact_sha256) = sink.finish();
        Ok(Receipt {
            dimensions,
            encoded_bytes,
            artifact_sha256,
            marker_manifest: vec![
                "SOI".to_owned(),
                "APP0".to_owned(),
                "SOF".to_owned(),
                "DQT".to_owned(),
                "DHT".to_owned(),
                "SOS".to_owned(),
                "EOI".to_owned(),
            ],
            codec: "mozjpeg-rs-0.9.2",
            quality: self.settings.quality,
            subsampling: self.settings.subsampling,
            progressive: self.settings.progressive,
            optimize_huffman: self.settings.optimize_huffman,
        })
    }
}

fn validate_artifact(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<(), Error> {
    let descriptor = artifact.image().descriptor();
    let dimensions = descriptor.dimensions();
    if dimensions.width() == 0
        || dimensions.height() == 0
        || dimensions.width() > u32::from(u16::MAX)
        || dimensions.height() > u32::from(u16::MAX)
    {
        return Err(Error::InvalidDimensions(dimensions));
    }
    if descriptor.orientation() != rusttable_image::Orientation::Normal {
        return Err(Error::UnsupportedOrientation);
    }
    let format = descriptor.format();
    if format.storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage(format.storage()));
    }
    if !matches!(format.channels(), ChannelLayout::Gray | ChannelLayout::Rgb) {
        return Err(Error::UnsupportedLayout(format.channels()));
    }
    if format.alpha() != AlphaMode::None {
        return Err(Error::UnsupportedAlpha(format.alpha()));
    }
    if format.sample_type() != SampleType::U8 {
        return Err(Error::UnsupportedSample(format.sample_type()));
    }
    if format.byte_order() != ByteOrder::Native {
        return Err(Error::UnsupportedByteOrder(format.byte_order()));
    }
    let metadata = artifact.metadata();
    let actual = metadata.icc_profile().map_or(0, <[u8]>::len)
        + metadata.exif().map_or(0, <[u8]>::len)
        + metadata.xmp().map_or(0, <[u8]>::len)
        + metadata.iptc().map_or(0, <[u8]>::len);
    if actual > settings.max_metadata_bytes {
        return Err(Error::MetadataLimit {
            limit: settings.max_metadata_bytes,
            actual,
        });
    }
    if metadata.icc_profile().is_some_and(<[u8]>::is_empty) {
        return Err(Error::EmptyProfile);
    }
    if settings.metadata == MetadataPolicy::All && !metadata.text().is_empty() {
        return Err(Error::MetadataTextUnsupported);
    }
    Ok(())
}

fn codec(settings: Settings, artifact: &CanonicalArtifact<'_>) -> Result<CodecEncoder, Error> {
    let preset = if settings.progressive {
        Preset::ProgressiveBalanced
    } else {
        Preset::BaselineBalanced
    };
    let mut encoder = CodecEncoder::new(preset)
        .quality(settings.quality)
        .subsampling(sampling(settings.subsampling))
        .progressive(settings.progressive)
        .optimize_huffman(settings.optimize_huffman)
        .restart_interval(settings.restart_interval);
    if let Some(density) = settings.density.or_else(|| artifact.metadata().density()) {
        encoder = encoder.pixel_density(pixel_density(density)?);
    }
    if settings.metadata == MetadataPolicy::All {
        encoder = add_metadata(encoder, artifact, settings)?;
    }
    Ok(encoder)
}

fn add_metadata(
    mut encoder: CodecEncoder,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<CodecEncoder, Error> {
    let metadata = artifact.metadata();
    if let Some(profile) = metadata.icc_profile() {
        encoder = encoder.icc_profile(profile.to_vec());
    }
    if let Some(exif) = metadata.exif() {
        if exif.len() > MAX_SEGMENT_BYTES.saturating_sub(6) {
            return Err(Error::MetadataTooLarge("EXIF"));
        }
        encoder = encoder.exif_data(exif.to_vec());
    }
    if let Some(xmp) = metadata.xmp() {
        encoder = add_xmp(encoder, xmp)?;
    }
    if let Some(iptc) = metadata.iptc() {
        encoder = add_chunked_app(encoder, 13, b"Photoshop 3.0\0", iptc)?;
    }
    let _ = settings.thumbnail;
    Ok(encoder)
}

fn add_xmp(mut encoder: CodecEncoder, xmp: &[u8]) -> Result<CodecEncoder, Error> {
    const STANDARD: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
    const EXTENDED: &[u8] = b"http://ns.adobe.com/xmp/extension/\0";
    if STANDARD
        .len()
        .checked_add(xmp.len())
        .is_some_and(|len| len <= MAX_SEGMENT_BYTES)
    {
        let mut data = STANDARD.to_vec();
        data.extend_from_slice(xmp);
        encoder = encoder.add_marker(1, data);
        return Ok(encoder);
    }
    let digest = Sha256::digest(xmp);
    let guid = digest
        .iter()
        .fold(String::with_capacity(64), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02X}");
            output
        });
    let chunk_size = MAX_SEGMENT_BYTES.saturating_sub(EXTENDED.len() + 64 + 4 + 4);
    if chunk_size == 0 || xmp.len() > usize::try_from(u32::MAX).unwrap_or(usize::MAX) {
        return Err(Error::MetadataTooLarge("extended XMP"));
    }
    for (offset, chunk) in xmp.chunks(chunk_size).enumerate() {
        let offset = offset
            .checked_mul(chunk_size)
            .ok_or(Error::MetadataTooLarge("extended XMP"))?;
        let mut data = Vec::with_capacity(EXTENDED.len() + 32 + 8 + chunk.len());
        data.extend_from_slice(EXTENDED);
        data.extend_from_slice(guid.as_bytes());
        data.extend_from_slice(
            &u32::try_from(xmp.len())
                .map_err(|_| Error::MetadataTooLarge("extended XMP"))?
                .to_be_bytes(),
        );
        data.extend_from_slice(
            &u32::try_from(offset)
                .map_err(|_| Error::MetadataTooLarge("extended XMP"))?
                .to_be_bytes(),
        );
        data.extend_from_slice(chunk);
        encoder = encoder.add_marker(1, data);
    }
    Ok(encoder)
}

fn add_chunked_app(
    mut encoder: CodecEncoder,
    marker: u8,
    prefix: &[u8],
    payload: &[u8],
) -> Result<CodecEncoder, Error> {
    let chunk_size = MAX_SEGMENT_BYTES.saturating_sub(prefix.len());
    if chunk_size == 0 {
        return Err(Error::MetadataTooLarge("APP segment"));
    }
    for (index, chunk) in payload.chunks(chunk_size).enumerate() {
        let mut data = Vec::with_capacity(prefix.len() + chunk.len());
        if index == 0 {
            data.extend_from_slice(prefix);
        }
        data.extend_from_slice(chunk);
        encoder = encoder.add_marker(marker, data);
    }
    Ok(encoder)
}

fn sampling(value: ChromaSubsampling) -> Subsampling {
    match value {
        ChromaSubsampling::FourFourFour => Subsampling::S444,
        ChromaSubsampling::FourTwoTwo => Subsampling::S422,
        ChromaSubsampling::FourTwoZero => Subsampling::S420,
    }
}

fn pixel_density(density: Density) -> Result<PixelDensity, Error> {
    let x = u16::try_from(density.x()).map_err(|_| Error::InvalidDensity)?;
    let y = u16::try_from(density.y()).map_err(|_| Error::InvalidDensity)?;
    let density = match density.unit() {
        DensityUnit::Inch => PixelDensity::dpi(x, y),
        DensityUnit::Centimeter => PixelDensity::dpcm(x, y),
        DensityUnit::Meter => return Err(Error::InvalidDensity),
    };
    Ok(density)
}

struct DigestWriter<'a, W> {
    writer: &'a mut W,
    hasher: Sha256,
    bytes: u64,
}

impl<'a, W> DigestWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    fn finish(self) -> (u64, [u8; 32]) {
        (self.bytes, self.hasher.finalize().into())
    }
}

impl<W: Write> Write for DigestWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.bytes = self
            .bytes
            .checked_add(written as u64)
            .ok_or_else(|| io::Error::other("JPEG byte count overflow"))?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn inspect(bytes: &[u8], receipt: Receipt) -> Result<(Vec<u8>, Receipt), Error> {
    if bytes.len() < 4 || bytes[0..2] != [0xFF, 0xD8] || bytes[bytes.len() - 2..] != [0xFF, 0xD9] {
        return Err(Error::Malformed("SOI/EOI"));
    }
    let mut index = 2usize;
    let mut dimensions = None;
    while index + 1 < bytes.len() {
        if bytes[index] != 0xFF {
            index += 1;
            continue;
        }
        while index < bytes.len() && bytes[index] == 0xFF {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let marker = bytes[index];
        index += 1;
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if index + 2 > bytes.len() {
            return Err(Error::Malformed("marker length"));
        }
        let length = usize::from(u16::from_be_bytes([bytes[index], bytes[index + 1]]));
        if length < 2 || index + length > bytes.len() {
            return Err(Error::Malformed("marker payload"));
        }
        if (0xC0..=0xC3).contains(&marker)
            || (0xC5..=0xC7).contains(&marker)
            || (0xC9..=0xCB).contains(&marker)
            || (0xCD..=0xCF).contains(&marker)
        {
            if length < 8 {
                return Err(Error::Malformed("frame header"));
            }
            let height = u32::from(u16::from_be_bytes([bytes[index + 3], bytes[index + 4]]));
            let width = u32::from(u16::from_be_bytes([bytes[index + 5], bytes[index + 6]]));
            dimensions = Some(
                ImageDimensions::new(width, height).map_err(|_| Error::Malformed("dimensions"))?,
            );
        }
        index += length;
    }
    if dimensions != Some(receipt.dimensions) {
        return Err(Error::Malformed("dimensions do not match receipt"));
    }
    Ok((bytes.to_vec(), receipt))
}
