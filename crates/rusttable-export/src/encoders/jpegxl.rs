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
pub const ENCODER_ID: &str = "jpegxl";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub lossless: bool,
    pub distance: f32,
    pub effort: u8,
    pub decoding_speed: u8,
    pub container: bool,
    pub metadata: MetadataPolicy,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            lossless: false,
            distance: 1.0,
            effort: 7,
            decoding_speed: 0,
            container: false,
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
        codec_version: rusttable_jxl_native::CODEC_VERSION,
        still_image: true,
        channels: &[ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba],
        samples: &[SampleType::U8, SampleType::U16, SampleType::F32],
        metadata: &["exif", "xmp"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: rusttable_image::ImageDimensions,
    pub encoded_bytes: u64,
    pub codec_version: &'static str,
    pub settings_hash: [u8; 32],
    pub threads: u16,
    pub container: bool,
    pub boxes: Vec<String>,
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
            Self::InvalidSettings(value) => write!(formatter, "invalid JPEG XL settings: {value}"),
            Self::UnsupportedLayout(value) => {
                write!(formatter, "unsupported JPEG XL layout: {value:?}")
            }
            Self::UnsupportedSample(value) => {
                write!(formatter, "unsupported JPEG XL sample: {value:?}")
            }
            Self::UnsupportedAlpha(value) => {
                write!(formatter, "unsupported JPEG XL alpha: {value:?}")
            }
            Self::UnsupportedStorage(value) => {
                write!(formatter, "unsupported JPEG XL storage: {value:?}")
            }
            Self::UnsupportedOrientation => formatter.write_str("JPEG XL requires orientation 1"),
            Self::UnsupportedMetadata(value) => {
                write!(formatter, "unsupported JPEG XL metadata: {value}")
            }
            Self::EmptyProfile => formatter.write_str("JPEG XL ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "JPEG XL metadata is {actual} bytes; limit is {limit}"
            ),
            Self::QueueBudget(value) => write!(formatter, "JPEG XL queue budget rejected: {value}"),
            Self::Cancelled => formatter.write_str("JPEG XL export cancelled"),
            Self::OutputLimit { limit, actual } => {
                write!(
                    formatter,
                    "JPEG XL output is {actual} bytes; limit is {limit}"
                )
            }
            Self::ImageView => formatter.write_str("JPEG XL image view is invalid"),
            Self::Codec(value) => write!(formatter, "JPEG XL codec failed: {value}"),
            Self::Io(value) => write!(formatter, "JPEG XL I/O failed: {value}"),
            Self::Malformed(value) => write!(formatter, "malformed JPEG XL output: {value}"),
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
            format.channels().channels(),
            format.sample_type().bytes(),
            budget,
        )
        .map_err(Error::QueueBudget)?;
        let pixels = tight_pixels(&view).ok_or(Error::ImageView)?;
        let metadata = artifact.metadata();
        let container =
            self.settings.container || metadata.exif().is_some() || metadata.xmp().is_some();
        let bytes = rusttable_jxl_native::encode(
            rusttable_jxl_native::Input {
                bytes: &pixels,
                sample: native_sample(format.sample_type())?,
                channels: u8::try_from(format.channels().channels())
                    .map_err(|_| Error::Malformed("channel count"))?,
            },
            view.descriptor().dimensions().width(),
            view.descriptor().dimensions().height(),
            rusttable_jxl_native::EncodeOptions {
                lossless: self.settings.lossless,
                quality: self.settings.distance,
                effort: self.settings.effort,
                decoding_speed: self.settings.decoding_speed,
                threads: budget.threads(),
                use_container: container,
                max_output_bytes: self.settings.max_output_bytes,
            },
            rusttable_jxl_native::MetadataRefs {
                exif: self.settings.metadata.then(metadata.exif()),
                xmp: self.settings.metadata.then(metadata.xmp()),
            },
        )
        .map_err(map_codec_error)?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let boxes = inspect(&bytes, container)?;
        let receipt = Receipt {
            dimensions: view.descriptor().dimensions(),
            encoded_bytes: u64::try_from(bytes.len()).map_err(|_| Error::Malformed("size"))?,
            codec_version: rusttable_jxl_native::CODEC_VERSION,
            settings_hash: settings_hash(self.settings),
            threads: budget.threads(),
            container,
            boxes,
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
        if !self.distance.is_finite() || !(0.0..=15.0).contains(&self.distance) {
            return Err(Error::InvalidSettings("distance"));
        }
        if self.effort == 0 || self.effort > 10 || self.decoding_speed > 4 {
            return Err(Error::InvalidSettings("effort or decoding speed"));
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
    if matches!(format.sample_type(), SampleType::F16) {
        return Err(Error::UnsupportedSample(format.sample_type()));
    }
    if artifact
        .metadata()
        .icc_profile()
        .is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::EmptyProfile);
    }
    if artifact.metadata().icc_profile().is_some() {
        return Err(Error::UnsupportedMetadata(
            "ICC requires a JXL color-profile API",
        ));
    }
    if artifact.metadata().iptc().is_some()
        || artifact.metadata().density().is_some()
        || !artifact.metadata().text().is_empty()
    {
        return Err(Error::UnsupportedMetadata(
            "IPTC, density, and text are not JXL boxes",
        ));
    }
    if settings.metadata == MetadataPolicy::All {
        let actual = artifact.metadata().exif().map_or(0, <[u8]>::len)
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

fn native_sample(sample: SampleType) -> Result<rusttable_jxl_native::SampleType, Error> {
    match sample {
        SampleType::U8 => Ok(rusttable_jxl_native::SampleType::U8),
        SampleType::U16 => Ok(rusttable_jxl_native::SampleType::U16),
        SampleType::F32 => Ok(rusttable_jxl_native::SampleType::F32),
        SampleType::F16 => Err(Error::UnsupportedSample(sample)),
    }
}

fn map_codec_error(error: rusttable_jxl_native::Error) -> Error {
    match error {
        rusttable_jxl_native::Error::OutputLimit { limit, actual } => {
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
        let bytes = view.row(0, row).ok()?;
        output.extend_from_slice(bytes.get(..row_bytes)?);
    }
    Some(output)
}

fn inspect(bytes: &[u8], container: bool) -> Result<Vec<String>, Error> {
    if container {
        if bytes.len() < 12 || bytes.get(4..8) != Some(b"JXL ") {
            return Err(Error::Malformed("container signature"));
        }
        let mut cursor = 12;
        let mut boxes = Vec::new();
        while cursor + 8 <= bytes.len() {
            let size = u32::from_be_bytes(
                bytes[cursor..cursor + 4]
                    .try_into()
                    .map_err(|_| Error::Malformed("box size"))?,
            ) as usize;
            let end = cursor
                .checked_add(size)
                .ok_or(Error::Malformed("box overflow"))?;
            if size < 8 || end > bytes.len() {
                return Err(Error::Malformed("box extent"));
            }
            let name = std::str::from_utf8(&bytes[cursor + 4..cursor + 8])
                .map_err(|_| Error::Malformed("box name"))?;
            boxes.push(name.to_owned());
            cursor = end;
        }
        if !boxes
            .iter()
            .any(|name| matches!(name.as_str(), "jxlc" | "jxlp"))
        {
            return Err(Error::Malformed("missing codestream box"));
        }
        Ok(boxes)
    } else if bytes.starts_with(&[0xff, 0x0a]) {
        Ok(Vec::new())
    } else {
        Err(Error::Malformed("codestream signature"))
    }
}

fn settings_hash(settings: Settings) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SETTINGS_SCHEMA_VERSION.to_be_bytes());
    hasher.update([u8::from(settings.lossless)]);
    hasher.update(settings.distance.to_bits().to_be_bytes());
    hasher.update([
        settings.effort,
        settings.decoding_speed,
        u8::from(settings.container),
    ]);
    hasher.update([settings.metadata as u8]);
    hasher.update((settings.max_metadata_bytes as u64).to_be_bytes());
    hasher.update(settings.max_output_bytes.to_be_bytes());
    hasher.finalize().into()
}

trait MetadataMode {
    fn then<T>(self, value: Option<T>) -> Option<T>;
}

impl MetadataMode for MetadataPolicy {
    fn then<T>(self, value: Option<T>) -> Option<T> {
        (self == Self::All).then_some(value).flatten()
    }
}
