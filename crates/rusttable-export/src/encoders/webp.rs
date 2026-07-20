#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use rusttable_image::{AlphaMode, ChannelLayout, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::CanonicalArtifact;
use crate::encoders::resource::{EncodeBudget, EncodeCancellation, NeverCancel, checked_memory};

const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 1 << 30;
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const ENCODER_ID: &str = "webp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub lossless: bool,
    pub quality: f32,
    pub alpha_quality: u8,
    pub near_lossless: u8,
    pub method: u8,
    pub exact: bool,
    pub sharp_yuv: bool,
    pub metadata: MetadataPolicy,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lossless: false,
            quality: 90.0,
            alpha_quality: 100,
            near_lossless: 100,
            method: 4,
            exact: true,
            sharp_yuv: true,
            metadata: MetadataPolicy::All,
            max_metadata_bytes: MAX_METADATA_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    pub encoder_id: &'static str,
    pub codec_version: &'static str,
    pub still_image: bool,
    pub channels: &'static [ChannelLayout],
    pub samples: &'static [SampleType],
    pub metadata: &'static [&'static str],
}

#[must_use]
pub const fn capabilities() -> Capabilities {
    Capabilities {
        encoder_id: ENCODER_ID,
        codec_version: rusttable_webp_native::CODEC_VERSION,
        still_image: true,
        channels: &[ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba],
        samples: &[SampleType::U8],
        metadata: &["icc", "exif", "xmp"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: rusttable_image::ImageDimensions,
    pub encoded_bytes: u64,
    pub codec_version: &'static str,
    pub settings_hash: [u8; 32],
    pub threads: u16,
    pub chunks: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedOrientation,
    UnsupportedMetadata(&'static str),
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    QueueBudget(&'static str),
    Cancelled,
    OutputLimit { limit: u64, actual: usize },
    ImageView,
    Codec(String),
    Io(io::Error),
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(value) => write!(formatter, "invalid WebP settings: {value}"),
            Self::UnsupportedLayout(value) => {
                write!(formatter, "unsupported WebP layout: {value:?}")
            }
            Self::UnsupportedSample(value) => {
                write!(formatter, "unsupported WebP sample: {value:?}")
            }
            Self::UnsupportedAlpha(value) => write!(formatter, "unsupported WebP alpha: {value:?}"),
            Self::UnsupportedStorage(value) => {
                write!(formatter, "unsupported WebP storage: {value:?}")
            }
            Self::UnsupportedOrientation => formatter.write_str("WebP requires orientation 1"),
            Self::UnsupportedMetadata(value) => {
                write!(formatter, "unsupported WebP metadata: {value}")
            }
            Self::EmptyProfile => formatter.write_str("WebP ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "WebP metadata is {actual} bytes; limit is {limit}"
            ),
            Self::QueueBudget(value) => write!(formatter, "WebP queue budget rejected: {value}"),
            Self::Cancelled => formatter.write_str("WebP export cancelled"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "WebP output is {actual} bytes; limit is {limit}")
            }
            Self::ImageView => formatter.write_str("WebP image view is invalid"),
            Self::Codec(value) => write!(formatter, "WebP codec failed: {value}"),
            Self::Io(value) => write!(formatter, "WebP I/O failed: {value}"),
            Self::Malformed(value) => write!(formatter, "malformed WebP output: {value}"),
        }
    }
}

impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
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

    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.encode_to_vec_with_budget(artifact, EncodeBudget::default(), &NeverCancel)
    }

    pub fn encode_to_vec_with_budget<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        budget: EncodeBudget,
        cancellation: &C,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let view = artifact.view().map_err(|_| Error::ImageView)?;
        let format = view.descriptor().format();
        checked_memory(
            view.descriptor().dimensions(),
            if format.channels() == ChannelLayout::Gray {
                3
            } else {
                format.channels().channels()
            },
            1,
            budget,
        )
        .map_err(Error::QueueBudget)?;
        let pixels = tight_pixels(&view).ok_or(Error::ImageView)?;
        let (pixels, alpha) = match format.channels() {
            ChannelLayout::Gray => (gray_to_rgb(&pixels), false),
            ChannelLayout::Rgb => (pixels, false),
            ChannelLayout::Rgba => (pixels, true),
            channels => return Err(Error::UnsupportedLayout(channels)),
        };
        let metadata = artifact.metadata();
        let refs = if self.settings.metadata == MetadataPolicy::All {
            rusttable_webp_native::MetadataRefs {
                icc: metadata.icc_profile(),
                exif: metadata.exif(),
                xmp: metadata.xmp(),
            }
        } else {
            rusttable_webp_native::MetadataRefs {
                icc: None,
                exif: None,
                xmp: None,
            }
        };
        let bytes = if alpha {
            rusttable_webp_native::encode_rgba(
                &pixels,
                view.descriptor().dimensions().width(),
                view.descriptor().dimensions().height(),
                native_options(self.settings, budget),
                refs,
            )
        } else {
            rusttable_webp_native::encode_rgb(
                &pixels,
                view.descriptor().dimensions().width(),
                view.descriptor().dimensions().height(),
                native_options(self.settings, budget),
                refs,
            )
        }
        .map_err(map_codec_error)?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let chunks = inspect(&bytes)?;
        let receipt = Receipt {
            dimensions: view.descriptor().dimensions(),
            encoded_bytes: u64::try_from(bytes.len()).map_err(|_| Error::Malformed("size"))?,
            codec_version: rusttable_webp_native::CODEC_VERSION,
            settings_hash: settings_hash(self.settings),
            threads: budget.threads(),
            chunks,
        };
        Ok((bytes, receipt))
    }

    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        let (bytes, receipt) = self.encode_to_vec(artifact)?;
        let result = (|| {
            let mut file = io::BufWriter::new(std::fs::File::create(path)?);
            file.write_all(&bytes)?;
            file.flush()?;
            file.into_inner()
                .map_err(|error| Error::Io(error.into_error()))?
                .sync_all()?;
            Ok(receipt)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }
}

impl Settings {
    pub fn validate(self) -> Result<(), Error> {
        if self.max_metadata_bytes == 0 || self.max_metadata_bytes > MAX_METADATA_BYTES {
            return Err(Error::InvalidSettings("metadata limit"));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidSettings("output limit"));
        }
        if !self.quality.is_finite() || !(0.0..=100.0).contains(&self.quality) {
            return Err(Error::InvalidSettings("quality"));
        }
        if self.alpha_quality > 100 || self.near_lossless > 100 || self.method > 6 {
            return Err(Error::InvalidSettings(
                "alpha quality, near-lossless, or method",
            ));
        }
        Ok(())
    }
}

fn validate_artifact(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<(), Error> {
    let descriptor = artifact.image().descriptor();
    let format = descriptor.format();
    if descriptor.orientation() != rusttable_image::Orientation::Normal {
        return Err(Error::UnsupportedOrientation);
    }
    if format.storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage(format.storage()));
    }
    if !matches!(
        format.channels(),
        ChannelLayout::Gray | ChannelLayout::Rgb | ChannelLayout::Rgba
    ) {
        return Err(Error::UnsupportedLayout(format.channels()));
    }
    if !matches!(format.alpha(), AlphaMode::None | AlphaMode::Straight) {
        return Err(Error::UnsupportedAlpha(format.alpha()));
    }
    if format.sample_type() != SampleType::U8 {
        return Err(Error::UnsupportedSample(format.sample_type()));
    }
    if artifact
        .metadata()
        .icc_profile()
        .is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::EmptyProfile);
    }
    if artifact.metadata().iptc().is_some()
        || artifact.metadata().density().is_some()
        || !artifact.metadata().text().is_empty()
    {
        return Err(Error::UnsupportedMetadata(
            "IPTC, density, and text are not WebP chunks",
        ));
    }
    if settings.metadata == MetadataPolicy::All {
        let actual = artifact.metadata().icc_profile().map_or(0, <[u8]>::len)
            + artifact.metadata().exif().map_or(0, <[u8]>::len)
            + artifact.metadata().xmp().map_or(0, <[u8]>::len);
        if actual > settings.max_metadata_bytes {
            return Err(Error::MetadataLimit {
                limit: settings.max_metadata_bytes,
                actual,
            });
        }
    }
    Ok(())
}

fn native_options(
    settings: Settings,
    budget: EncodeBudget,
) -> rusttable_webp_native::EncodeOptions {
    rusttable_webp_native::EncodeOptions {
        lossless: settings.lossless,
        quality: settings.quality,
        alpha_quality: settings.alpha_quality,
        near_lossless: settings.near_lossless,
        method: settings.method,
        exact: settings.exact,
        sharp_yuv: settings.sharp_yuv,
        threads: budget.threads(),
        max_output_bytes: settings.max_output_bytes,
    }
}

fn map_codec_error(error: rusttable_webp_native::Error) -> Error {
    match error {
        rusttable_webp_native::Error::OutputLimit { limit, actual } => {
            Error::OutputLimit { limit, actual }
        }
        other => Error::Codec(other.to_string()),
    }
}

fn tight_pixels(view: &rusttable_image::ImageView<'_>) -> Option<Vec<u8>> {
    let format = view.descriptor().format();
    let row_bytes = format
        .bytes_per_pixel()
        .checked_mul(usize::try_from(view.descriptor().dimensions().width()).ok()?)?;
    let mut output = Vec::with_capacity(
        row_bytes.checked_mul(usize::try_from(view.descriptor().dimensions().height()).ok()?)?,
    );
    for row in 0..view.descriptor().dimensions().height() {
        output.extend_from_slice(view.row(0, row).ok()?.get(..row_bytes)?);
    }
    Some(output)
}

fn gray_to_rgb(gray: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(gray.len().saturating_mul(3));
    for value in gray {
        rgb.extend_from_slice(&[*value, *value, *value]);
    }
    rgb
}

fn inspect(bytes: &[u8]) -> Result<Vec<String>, Error> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return Err(Error::Malformed("RIFF signature"));
    }
    let declared = u32::from_le_bytes(
        bytes[4..8]
            .try_into()
            .map_err(|_| Error::Malformed("RIFF size"))?,
    ) as usize;
    if declared.checked_add(8) != Some(bytes.len()) {
        return Err(Error::Malformed("RIFF size"));
    }
    let mut cursor = 12;
    let mut chunks = Vec::new();
    while cursor + 8 <= bytes.len() {
        let name = std::str::from_utf8(&bytes[cursor..cursor + 4])
            .map_err(|_| Error::Malformed("chunk name"))?;
        let size = u32::from_le_bytes(
            bytes[cursor + 4..cursor + 8]
                .try_into()
                .map_err(|_| Error::Malformed("chunk size"))?,
        ) as usize;
        let payload_end = cursor
            .checked_add(8)
            .and_then(|value| value.checked_add(size))
            .ok_or(Error::Malformed("chunk overflow"))?;
        let end = payload_end
            .checked_add(size % 2)
            .ok_or(Error::Malformed("chunk padding"))?;
        if end > bytes.len() {
            return Err(Error::Malformed("chunk extent"));
        }
        if name == "ANIM" || name == "ANMF" {
            return Err(Error::Malformed("animation output"));
        }
        chunks.push(name.to_owned());
        cursor = end;
    }
    if cursor != bytes.len()
        || !chunks
            .iter()
            .any(|name| matches!(name.as_str(), "VP8 " | "VP8L"))
    {
        return Err(Error::Malformed("missing image chunk"));
    }
    Ok(chunks)
}

fn settings_hash(settings: Settings) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SETTINGS_SCHEMA_VERSION.to_be_bytes());
    hasher.update([u8::from(settings.lossless)]);
    hasher.update(settings.quality.to_bits().to_be_bytes());
    hasher.update([
        settings.alpha_quality,
        settings.near_lossless,
        settings.method,
        u8::from(settings.exact),
        u8::from(settings.sharp_yuv),
        settings.metadata as u8,
    ]);
    hasher.update((settings.max_metadata_bytes as u64).to_be_bytes());
    hasher.update(settings.max_output_bytes.to_be_bytes());
    hasher.finalize().into()
}
