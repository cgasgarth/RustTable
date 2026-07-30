//! HEIF/HEIC still-image export through one explicit HEVC encoder contract.
#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use rusttable_image::{AlphaMode, ChannelLayout, ColorEncoding, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::CanonicalArtifact;
use crate::encoders::resource::{EncodeBudget, EncodeCancellation, NeverCancel, checked_memory};
use crate::encoders::yuv::{self, BitDepth, Input, Matrix, Range, Subsampling};

pub const ENCODER_ID_HEIF: &str = "heif";
pub const ENCODER_ID_HEIC: &str = "heic";
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: u64 = 1 << 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaSubsampling {
    FourFourFour,
    FourTwoTwo,
    FourTwoZero,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputBitDepth {
    Eight,
    Ten,
    Twelve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncoderId {
    Heif,
    Heic,
}

impl EncoderId {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Heif => ENCODER_ID_HEIF,
            Self::Heic => ENCODER_ID_HEIC,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub encoder: EncoderId,
    pub lossless: bool,
    pub quality: u8,
    pub bit_depth: OutputBitDepth,
    pub subsampling: ChromaSubsampling,
    pub matrix: Matrix,
    pub full_range: bool,
    pub metadata: MetadataPolicy,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            encoder: EncoderId::Heic,
            lossless: false,
            quality: 90,
            bit_depth: OutputBitDepth::Ten,
            subsampling: ChromaSubsampling::FourFourFour,
            matrix: Matrix::Bt709,
            full_range: true,
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
    pub bit_depths: &'static [OutputBitDepth],
    pub chroma: &'static [ChromaSubsampling],
    pub metadata: &'static [&'static str],
}

#[must_use]
pub const fn capabilities(id: EncoderId) -> Capabilities {
    Capabilities {
        encoder_id: id.as_str(),
        codec_version: rusttable_heif_native::CODEC_VERSION,
        still_image: true,
        channels: &[ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba],
        samples: &[SampleType::U8, SampleType::U16],
        bit_depths: &[
            OutputBitDepth::Eight,
            OutputBitDepth::Ten,
            OutputBitDepth::Twelve,
        ],
        chroma: &[
            ChromaSubsampling::FourFourFour,
            ChromaSubsampling::FourTwoTwo,
            ChromaSubsampling::FourTwoZero,
        ],
        metadata: &["icc", "exif", "xmp"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub encoder_id: &'static str,
    pub dimensions: rusttable_image::ImageDimensions,
    pub encoded_bytes: u64,
    pub codec_version: &'static str,
    pub settings_hash: [u8; 32],
    pub threads: u16,
    pub depth: OutputBitDepth,
    pub chroma: ChromaSubsampling,
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
    UnsupportedProfile(ColorEncoding),
    UnsupportedMetadata(&'static str),
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    QueueBudget(&'static str),
    Cancelled,
    OutputLimit { limit: u64, actual: usize },
    ImageView,
    Codec(String),
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(value) => write!(formatter, "invalid HEIF settings: {value}"),
            Self::UnsupportedLayout(value) => {
                write!(formatter, "unsupported HEIF layout: {value:?}")
            }
            Self::UnsupportedSample(value) => {
                write!(formatter, "unsupported HEIF sample: {value:?}")
            }
            Self::UnsupportedAlpha(value) => write!(formatter, "unsupported HEIF alpha: {value:?}"),
            Self::UnsupportedStorage(value) => {
                write!(formatter, "unsupported HEIF storage: {value:?}")
            }
            Self::UnsupportedOrientation => formatter.write_str("HEIF requires orientation 1"),
            Self::UnsupportedProfile(value) => {
                write!(formatter, "unsupported HEIF profile: {value:?}")
            }
            Self::UnsupportedMetadata(value) => {
                write!(formatter, "unsupported HEIF metadata: {value}")
            }
            Self::EmptyProfile => formatter.write_str("HEIF ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "HEIF metadata is {actual} bytes; limit is {limit}"
            ),
            Self::QueueBudget(value) => write!(formatter, "HEIF queue budget rejected: {value}"),
            Self::Cancelled => formatter.write_str("HEIF export cancelled"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "HEIF output is {actual} bytes; limit is {limit}")
            }
            Self::ImageView => formatter.write_str("HEIF image view is invalid"),
            Self::Codec(value) => write!(formatter, "HEIF codec failed: {value}"),
            Self::Io(value) => write!(formatter, "HEIF I/O failed: {value}"),
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
        let frame = yuv::convert(
            Input {
                bytes: &pixels,
                layout: format.channels(),
                sample: format.sample_type(),
                byte_order: format.byte_order(),
            },
            view.descriptor().dimensions().width(),
            view.descriptor().dimensions().height(),
            bit_depth(self.settings.bit_depth),
            self.settings.matrix,
            range(self.settings),
            subsampling(self.settings.subsampling),
        )
        .map_err(|error| Error::Codec(error.to_string()))?;
        let _roundtrip = yuv::roundtrip_rgb(&frame, self.settings.matrix, range(self.settings));
        let metadata = artifact.metadata();
        let refs = if self.settings.metadata == MetadataPolicy::All {
            rusttable_heif_native::MetadataRefs {
                icc: metadata.icc_profile(),
                exif: metadata.exif(),
                xmp: metadata.xmp(),
            }
        } else {
            rusttable_heif_native::MetadataRefs {
                icc: None,
                exif: None,
                xmp: None,
            }
        };
        let bytes = rusttable_heif_native::encode(
            rusttable_heif_native::Planes {
                width: frame.width,
                height: frame.height,
                y: &frame.y,
                cb: &frame.cb,
                cr: &frame.cr,
                alpha: frame.alpha.as_deref(),
                chroma_width: frame.chroma_width,
                chroma_height: frame.chroma_height,
            },
            native_options(self.settings, budget),
            refs,
        )
        .map_err(map_codec_error)?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let boxes = rusttable_heif_native::inspect(
            &bytes,
            frame.width,
            frame.height,
            frame.alpha.is_some(),
        )
        .map_err(map_codec_error)?;
        let receipt = Receipt {
            encoder_id: self.settings.encoder.as_str(),
            dimensions: view.descriptor().dimensions(),
            encoded_bytes: u64::try_from(bytes.len())
                .map_err(|_| Error::Codec("size overflow".to_owned()))?,
            codec_version: rusttable_heif_native::CODEC_VERSION,
            settings_hash: settings_hash(self.settings),
            threads: budget.threads(),
            depth: self.settings.bit_depth,
            chroma: self.settings.subsampling,
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
        if self.max_metadata_bytes == 0
            || self.max_metadata_bytes > MAX_METADATA_BYTES
            || self.max_output_bytes == 0
            || self.max_output_bytes > MAX_OUTPUT_BYTES
        {
            return Err(Error::InvalidSettings("metadata or output limit"));
        }
        if !(1..=100).contains(&self.quality) {
            return Err(Error::InvalidSettings("quality"));
        }
        if self.lossless && self.quality != 100 {
            return Err(Error::InvalidSettings("lossless requires quality 100"));
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
    if !matches!(format.sample_type(), SampleType::U8 | SampleType::U16) {
        return Err(Error::UnsupportedSample(format.sample_type()));
    }
    if !matches!(
        descriptor.color_encoding(),
        ColorEncoding::SrgbD65 | ColorEncoding::DisplayP3D65 | ColorEncoding::Rec2020D65
    ) {
        return Err(Error::UnsupportedProfile(descriptor.color_encoding()));
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
            "IPTC, density, and text are not HEIF metadata items",
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

fn native_options(settings: Settings, budget: EncodeBudget) -> rusttable_heif_native::Options {
    rusttable_heif_native::Options {
        quality: (!settings.lossless).then_some(settings.quality),
        chroma: match settings.subsampling {
            ChromaSubsampling::FourFourFour => rusttable_heif_native::ChromaMode::C444,
            ChromaSubsampling::FourTwoTwo => rusttable_heif_native::ChromaMode::C422,
            ChromaSubsampling::FourTwoZero => rusttable_heif_native::ChromaMode::C420,
        },
        depth: match settings.bit_depth {
            OutputBitDepth::Eight => rusttable_heif_native::Depth::Eight,
            OutputBitDepth::Ten => rusttable_heif_native::Depth::Ten,
            OutputBitDepth::Twelve => rusttable_heif_native::Depth::Twelve,
        },
        matrix: match settings.matrix {
            Matrix::Bt601 => rusttable_heif_native::Matrix::Bt601,
            Matrix::Bt709 => rusttable_heif_native::Matrix::Bt709,
            Matrix::Bt2020 => rusttable_heif_native::Matrix::Bt2020,
            Matrix::Identity => rusttable_heif_native::Matrix::Identity,
        },
        full_range: settings.full_range,
        threads: budget.threads(),
        max_output_bytes: settings.max_output_bytes,
    }
}

fn map_codec_error(error: rusttable_heif_native::Error) -> Error {
    match error {
        rusttable_heif_native::Error::OutputLimit { limit, actual } => {
            Error::OutputLimit { limit, actual }
        }
        other => Error::Codec(other.to_string()),
    }
}
fn bit_depth(value: OutputBitDepth) -> BitDepth {
    match value {
        OutputBitDepth::Eight => BitDepth::Eight,
        OutputBitDepth::Ten => BitDepth::Ten,
        OutputBitDepth::Twelve => BitDepth::Twelve,
    }
}
fn range(settings: Settings) -> Range {
    if settings.full_range {
        Range::Full
    } else {
        Range::Limited
    }
}
fn subsampling(value: ChromaSubsampling) -> Subsampling {
    match value {
        ChromaSubsampling::FourFourFour => Subsampling::FourFourFour,
        ChromaSubsampling::FourTwoTwo => Subsampling::FourTwoTwo,
        ChromaSubsampling::FourTwoZero => Subsampling::FourTwoZero,
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
fn settings_hash(settings: Settings) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SETTINGS_SCHEMA_VERSION.to_be_bytes());
    hasher.update([
        settings.encoder as u8,
        u8::from(settings.lossless),
        settings.quality,
        settings.bit_depth as u8,
        settings.subsampling as u8,
        settings.matrix as u8,
        u8::from(settings.full_range),
        settings.metadata as u8,
    ]);
    hasher.update((settings.max_metadata_bytes as u64).to_be_bytes());
    hasher.update(settings.max_output_bytes.to_be_bytes());
    hasher.finalize().into()
}
