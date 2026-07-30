//! AVIF still-image export with RustTable-owned YUV conversion and validation.
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::too_many_lines)]

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use rusttable_image::{AlphaMode, ChannelLayout, ColorEncoding, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::CanonicalArtifact;
use crate::encoders::resource::{EncodeBudget, EncodeCancellation, NeverCancel, checked_memory};
use crate::encoders::yuv::{self, BitDepth, Input, Matrix, Range, Subsampling};

pub const ENCODER_ID: &str = "avif";
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
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub lossless: bool,
    pub quality: f32,
    pub alpha_quality: f32,
    pub speed: u8,
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
            lossless: false,
            quality: 90.0,
            alpha_quality: 100.0,
            speed: 6,
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
pub const fn capabilities() -> Capabilities {
    Capabilities {
        encoder_id: ENCODER_ID,
        codec_version: rusttable_avif_native::CODEC_VERSION,
        still_image: true,
        channels: &[ChannelLayout::Gray, ChannelLayout::Rgb, ChannelLayout::Rgba],
        samples: &[SampleType::U8, SampleType::U16],
        bit_depths: &[OutputBitDepth::Eight, OutputBitDepth::Ten],
        chroma: &[ChromaSubsampling::FourFourFour],
        metadata: &["exif"],
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
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
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(value) => write!(formatter, "invalid AVIF settings: {value}"),
            Self::UnsupportedLayout(value) => {
                write!(formatter, "unsupported AVIF layout: {value:?}")
            }
            Self::UnsupportedSample(value) => {
                write!(formatter, "unsupported AVIF sample: {value:?}")
            }
            Self::UnsupportedAlpha(value) => write!(formatter, "unsupported AVIF alpha: {value:?}"),
            Self::UnsupportedStorage(value) => {
                write!(formatter, "unsupported AVIF storage: {value:?}")
            }
            Self::UnsupportedOrientation => formatter.write_str("AVIF requires orientation 1"),
            Self::UnsupportedProfile(value) => {
                write!(formatter, "unsupported AVIF profile: {value:?}")
            }
            Self::UnsupportedMetadata(value) => {
                write!(formatter, "unsupported AVIF metadata: {value}")
            }
            Self::EmptyProfile => formatter.write_str("AVIF ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => write!(
                formatter,
                "AVIF metadata is {actual} bytes; limit is {limit}"
            ),
            Self::QueueBudget(value) => write!(formatter, "AVIF queue budget rejected: {value}"),
            Self::Cancelled => formatter.write_str("AVIF export cancelled"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "AVIF output is {actual} bytes; limit is {limit}")
            }
            Self::ImageView => formatter.write_str("AVIF image view is invalid"),
            Self::Codec(value) => write!(formatter, "AVIF codec failed: {value}"),
            Self::Io(value) => write!(formatter, "AVIF I/O failed: {value}"),
            Self::Malformed(value) => write!(formatter, "malformed AVIF output: {value}"),
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
            rusttable_avif_native::MetadataRefs {
                exif: metadata.exif(),
            }
        } else {
            rusttable_avif_native::MetadataRefs { exif: None }
        };
        let bytes = match self.settings.bit_depth {
            OutputBitDepth::Eight => {
                let color = frame
                    .y
                    .iter()
                    .zip(&frame.cb)
                    .zip(&frame.cr)
                    .map(|((y, cb), cr)| {
                        [
                            u8::try_from(*y).unwrap_or(u8::MAX),
                            u8::try_from(*cb).unwrap_or(u8::MAX),
                            u8::try_from(*cr).unwrap_or(u8::MAX),
                        ]
                    })
                    .collect::<Vec<_>>();
                let alpha = frame.alpha.as_deref().map(|values| {
                    values
                        .iter()
                        .map(|value| u8::try_from(*value).unwrap_or(u8::MAX))
                        .collect::<Vec<_>>()
                });
                rusttable_avif_native::encode_8(
                    view.descriptor().dimensions().width(),
                    view.descriptor().dimensions().height(),
                    &color,
                    alpha.as_deref(),
                    native_options(self.settings, budget),
                    refs,
                )
            }
            OutputBitDepth::Ten => {
                let color = frame
                    .y
                    .iter()
                    .zip(&frame.cb)
                    .zip(&frame.cr)
                    .map(|((y, cb), cr)| [*y, *cb, *cr])
                    .collect::<Vec<_>>();
                rusttable_avif_native::encode_10(
                    view.descriptor().dimensions().width(),
                    view.descriptor().dimensions().height(),
                    &color,
                    frame.alpha.as_deref(),
                    native_options(self.settings, budget),
                    refs,
                )
            }
        }
        .map_err(map_codec_error)?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let boxes = inspect(
            &bytes,
            view.descriptor().dimensions(),
            frame.alpha.is_some(),
        )?;
        let receipt = Receipt {
            dimensions: view.descriptor().dimensions(),
            encoded_bytes: u64::try_from(bytes.len()).map_err(|_| Error::Malformed("size"))?,
            codec_version: rusttable_avif_native::CODEC_VERSION,
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
        if !self.quality.is_finite()
            || !self.alpha_quality.is_finite()
            || !(0.0..=100.0).contains(&self.quality)
            || !(0.0..=100.0).contains(&self.alpha_quality)
        {
            return Err(Error::InvalidSettings("quality"));
        }
        if !(1..=10).contains(&self.speed) {
            return Err(Error::InvalidSettings("speed"));
        }
        if self.lossless && (self.quality < 100.0 || self.alpha_quality < 100.0) {
            return Err(Error::InvalidSettings("lossless requires quality 100"));
        }
        if self.subsampling != ChromaSubsampling::FourFourFour {
            return Err(Error::InvalidSettings("ravif only advertises 4:4:4"));
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
    if !matches!(descriptor.color_encoding(), ColorEncoding::SrgbD65) {
        return Err(Error::UnsupportedProfile(descriptor.color_encoding()));
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
            "ICC requires a libavif profile API",
        ));
    }
    if artifact.metadata().xmp().is_some() {
        return Err(Error::UnsupportedMetadata(
            "XMP requires a libavif metadata-item API",
        ));
    }
    if artifact.metadata().iptc().is_some()
        || artifact.metadata().density().is_some()
        || !artifact.metadata().text().is_empty()
    {
        return Err(Error::UnsupportedMetadata(
            "IPTC, density, and text are not in the AVIF contract",
        ));
    }
    if settings.metadata == MetadataPolicy::All {
        let actual = artifact.metadata().exif().map_or(0, <[u8]>::len);
        if actual > settings.max_metadata_bytes {
            return Err(Error::MetadataLimit {
                limit: settings.max_metadata_bytes,
                actual,
            });
        }
    }
    Ok(())
}

fn native_options(settings: Settings, budget: EncodeBudget) -> rusttable_avif_native::Options {
    rusttable_avif_native::Options {
        quality: settings.quality,
        alpha_quality: settings.alpha_quality,
        speed: settings.speed,
        depth: match settings.bit_depth {
            OutputBitDepth::Eight => rusttable_avif_native::Depth::Eight,
            OutputBitDepth::Ten => rusttable_avif_native::Depth::Ten,
        },
        matrix: native_matrix(settings.matrix),
        full_range: settings.full_range,
        threads: budget.threads(),
        max_output_bytes: settings.max_output_bytes,
    }
}

fn native_matrix(value: Matrix) -> rusttable_avif_native::Matrix {
    match value {
        Matrix::Bt601 => rusttable_avif_native::Matrix::Bt601,
        Matrix::Bt709 => rusttable_avif_native::Matrix::Bt709,
        Matrix::Bt2020 => rusttable_avif_native::Matrix::Bt2020,
        Matrix::Identity => rusttable_avif_native::Matrix::Identity,
    }
}
fn bit_depth(value: OutputBitDepth) -> BitDepth {
    match value {
        OutputBitDepth::Eight => BitDepth::Eight,
        OutputBitDepth::Ten => BitDepth::Ten,
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

fn map_codec_error(error: rusttable_avif_native::Error) -> Error {
    match error {
        rusttable_avif_native::Error::OutputLimit { limit, actual } => {
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

fn inspect(
    bytes: &[u8],
    dimensions: rusttable_image::ImageDimensions,
    has_alpha: bool,
) -> Result<Vec<String>, Error> {
    if bytes.len() < 16 || &bytes[4..8] != b"ftyp" {
        return Err(Error::Malformed("ftyp signature"));
    }
    let mut boxes = Vec::new();
    let mut cursor = 0;
    while cursor + 8 <= bytes.len() {
        let size = u32::from_be_bytes(
            bytes[cursor..cursor + 4]
                .try_into()
                .map_err(|_| Error::Malformed("box size"))?,
        ) as usize;
        if size < 8 || cursor.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err(Error::Malformed("box extent"));
        }
        let name = std::str::from_utf8(&bytes[cursor + 4..cursor + 8])
            .map_err(|_| Error::Malformed("box name"))?;
        boxes.push(name.to_owned());
        cursor += size;
    }
    if cursor != bytes.len()
        || !boxes.iter().any(|name| name == "meta")
        || !boxes.iter().any(|name| name == "mdat")
    {
        return Err(Error::Malformed("missing AVIF metadata or payload"));
    }
    if has_alpha && !boxes.iter().any(|name| name == "meta") {
        return Err(Error::Malformed("missing alpha association"));
    }
    let mut input = bytes;
    let parsed = avif_parse::read_avif(&mut input).map_err(|_| Error::Malformed("AV1 item"))?;
    let metadata = parsed
        .primary_item_metadata()
        .map_err(|_| Error::Malformed("AV1 metadata"))?;
    if metadata.max_frame_width.get() != dimensions.width()
        || metadata.max_frame_height.get() != dimensions.height()
    {
        return Err(Error::Malformed("dimensions"));
    }
    Ok(boxes)
}

fn settings_hash(settings: Settings) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(SETTINGS_SCHEMA_VERSION.to_be_bytes());
    hasher.update([u8::from(settings.lossless)]);
    hasher.update(settings.quality.to_bits().to_be_bytes());
    hasher.update(settings.alpha_quality.to_bits().to_be_bytes());
    hasher.update([
        settings.speed,
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
