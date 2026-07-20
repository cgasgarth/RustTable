//! Safe JPEG 2000 Part 1 export with an independently checked J2K/JP2 boundary.
#![allow(
    clippy::missing_errors_doc,
    clippy::struct_excessive_bools,
    clippy::too_many_lines
)]

mod boxes;
mod inspect;
mod settings;

pub use inspect::{Inspection, InspectionError, inspect};
pub use settings::{Container, ProgressionOrder, Settings, SettingsError, Transform};

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use rusttable_image::{AlphaMode, ChannelLayout, SampleType};
use sha2::{Digest, Sha256};

use crate::encoders::raster::{self, RasterError};
use crate::encoders::resource::checked_memory;
use crate::{CanonicalArtifact, EncodeBudget, EncodeCancellation, NeverCancel};

pub const ENCODER_ID: &str = "j2k";
pub const JP2_ENCODER_ID: &str = "jp2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub container: Container,
    pub dimensions: rusttable_image::ImageDimensions,
    pub components: u8,
    pub bit_depth: u8,
    pub alpha: bool,
    pub transform: Transform,
    pub progression: ProgressionOrder,
    pub resolution_levels: u8,
    pub code_block: (u16, u16),
    pub codec_version: &'static str,
    pub threads: u16,
    pub metadata_sha256: Option<[u8; 32]>,
    pub settings_sha256: [u8; 32],
    pub encoded_bytes: u64,
    pub output_sha256: [u8; 32],
    pub markers: Vec<String>,
    pub boxes: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(SettingsError),
    Unsupported(&'static str),
    Raster(&'static str),
    QueueBudget(&'static str),
    Cancelled,
    OutputLimit { limit: u64, actual: usize },
    Codec(String),
    Io(io::Error),
    Malformed(&'static str),
    Inspection(InspectionError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "JPEG 2000 export failed: {self:?}")
    }
}
impl std::error::Error for Error {}
impl From<SettingsError> for Error {
    fn from(value: SettingsError) -> Self {
        Self::InvalidSettings(value)
    }
}
impl From<RasterError> for Error {
    fn from(_: RasterError) -> Self {
        Self::Raster("invalid raster")
    }
}
impl From<InspectionError> for Error {
    fn from(value: InspectionError) -> Self {
        Self::Inspection(value)
    }
}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Encoder {
    settings: Settings,
}

#[must_use]
pub fn capabilities() -> crate::EncoderCapabilityDescriptor {
    use crate::capabilities::MetadataField;
    let mut descriptor = crate::EncoderCapabilityDescriptor::new(ENCODER_ID)
        .with_format(rusttable_image::OutputFormat::Jpeg2000)
        .with_format(rusttable_image::OutputFormat::Jp2)
        .with_channel_layout(crate::ChannelLayout::Gray)
        .with_channel_layout(crate::ChannelLayout::Rgb)
        .with_channel_layout(crate::ChannelLayout::Rgba)
        .with_bit_depth(crate::BitDepth::Eight)
        .with_bit_depth(crate::BitDepth::Twelve)
        .with_bit_depth(crate::BitDepth::Sixteen)
        .supports_alpha();
    descriptor = descriptor.with_metadata(MetadataField::IccAndCicp);
    descriptor
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
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        validate_artifact(artifact, self.settings, budget)?;
        if !self.settings.engine_supported() {
            return Err(Error::Unsupported(
                "codec setting is not supported by the pinned engine",
            ));
        }
        let view = artifact
            .view()
            .map_err(|_| Error::Malformed("image view"))?;
        let descriptor = view.descriptor();
        let format = descriptor.format();
        let width = descriptor.dimensions().width();
        let height = descriptor.dimensions().height();
        let mut pixels = Vec::new();
        let row_bytes = usize::try_from(width)
            .map_err(|_| Error::Malformed("width"))?
            .checked_mul(format.bytes_per_pixel())
            .ok_or(Error::Malformed("row size"))?;
        pixels
            .try_reserve(
                row_bytes
                    .checked_mul(usize::try_from(height).map_err(|_| Error::Malformed("height"))?)
                    .ok_or(Error::Malformed("pixel size"))?,
            )
            .map_err(|_| Error::QueueBudget("pixel allocation"))?;
        for y in 0..height {
            if cancellation.is_cancelled() {
                return Err(Error::Cancelled);
            }
            let row = view.row(0, y).map_err(|_| Error::Malformed("image row"))?;
            pixels.extend_from_slice(row.get(..row_bytes).ok_or(Error::Malformed("short row"))?);
        }
        let options = rusttable_jpeg2000_native::Options {
            reversible: self.settings.lossless,
            decomposition_levels: self.settings.resolution_levels.min(32),
            code_block_width_exp: log2(self.settings.code_block_width).saturating_sub(2),
            code_block_height_exp: log2(self.settings.code_block_height).saturating_sub(2),
        };
        let components = u8::try_from(format.channels().channels())
            .map_err(|_| Error::Malformed("component count"))?;
        let encoded = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            rusttable_jpeg2000_native::encode(
                &pixels,
                width,
                height,
                components,
                self.settings.bit_depth,
                options,
            )
        }))
        .map_err(|_| Error::Codec("codec panicked on bounded input".to_owned()))?
        .map_err(|error| Error::Codec(error.to_string()))?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let color_space = if format.channels() == ChannelLayout::Gray {
            boxes::ColorSpace::Gray
        } else {
            boxes::ColorSpace::Srgb
        };
        let output = if self.settings.container == Container::Jp2 {
            boxes::wrap(
                &encoded,
                width,
                height,
                u8::try_from(format.channels().channels())
                    .map_err(|_| Error::Malformed("component count"))?,
                self.settings.bit_depth,
                color_space,
                artifact.metadata().icc_profile(),
            )
            .map_err(|_| Error::Malformed("JP2 boxes"))?
        } else {
            encoded
        };
        if u64::try_from(output.len()).map_err(|_| Error::OutputLimit {
            limit: self.settings.max_output_bytes,
            actual: usize::MAX,
        })? > self.settings.max_output_bytes
        {
            return Err(Error::OutputLimit {
                limit: self.settings.max_output_bytes,
                actual: output.len(),
            });
        }
        let inspection = inspect::inspect(&output, self.settings.container)?;
        if self.settings.lossless {
            let decoded = rusttable_jpeg2000_native::decode(
                boxes::codestream(&output, self.settings.container)
                    .map_err(|_| Error::Malformed("codestream"))?,
            )
            .map_err(|error| Error::Codec(error.to_string()))?;
            if decoded.pixels != pixels || decoded.width != width || decoded.height != height {
                return Err(Error::Malformed("lossless independent decode mismatch"));
            }
        }
        let metadata_sha256 = metadata_digest(artifact);
        let receipt = Receipt {
            container: self.settings.container,
            dimensions: descriptor.dimensions(),
            components: inspection.components,
            bit_depth: inspection.bit_depth,
            alpha: inspection.alpha,
            transform: inspection.transform,
            progression: inspection.progression,
            resolution_levels: inspection.resolution_levels,
            code_block: (inspection.code_block_width, inspection.code_block_height),
            codec_version: rusttable_jpeg2000_native::CODEC_VERSION,
            threads: self.settings.threads.min(budget.threads()),
            metadata_sha256,
            settings_sha256: settings_digest(self.settings),
            encoded_bytes: u64::try_from(output.len())
                .map_err(|_| Error::Malformed("output length"))?,
            output_sha256: Sha256::digest(&output).into(),
            markers: inspection.markers,
            boxes: inspection.boxes,
        };
        Ok((output, receipt))
    }

    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        let result = (|| {
            let (bytes, receipt) = self.encode_to_vec(artifact)?;
            let mut file = std::fs::File::create(path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok(receipt)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }
}

fn validate_artifact(
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
    budget: EncodeBudget,
) -> Result<(), Error> {
    let shape = raster::shape(artifact)?;
    let descriptor = artifact.image().descriptor();
    if !matches!(shape.sample_type, SampleType::U8 | SampleType::U16) {
        return Err(Error::Unsupported("JPEG 2000 requires integer samples"));
    }
    if shape.channels == 2 {
        return Err(Error::Unsupported(
            "gray alpha is not in the JPEG 2000 contract",
        ));
    }
    if shape.channels != descriptor.format().channels().channels() {
        return Err(Error::Malformed("channel descriptor"));
    }
    if shape.sample_bytes != usize::from(if settings.bit_depth <= 8 { 1u8 } else { 2u8 }) {
        return Err(Error::Unsupported(
            "settings depth does not match sample storage",
        ));
    }
    if descriptor.format().alpha() != AlphaMode::None && shape.channels != 4 {
        return Err(Error::Unsupported("alpha layout"));
    }
    if settings.container == Container::J2k
        && (artifact.metadata().icc_profile().is_some()
            || artifact.metadata().exif().is_some()
            || artifact.metadata().xmp().is_some()
            || artifact.metadata().iptc().is_some()
            || !artifact.metadata().text().is_empty())
    {
        return Err(Error::Unsupported(
            "raw codestream cannot carry required metadata",
        ));
    }
    if shape.channels == 4 && settings.container == Container::J2k && !settings.allow_raw_alpha {
        return Err(Error::Unsupported(
            "raw codestream alpha requires explicit opt-in",
        ));
    }
    checked_memory(
        descriptor.dimensions(),
        shape.channels,
        shape.sample_bytes,
        budget,
    )
    .map_err(Error::QueueBudget)
}

fn metadata_digest(artifact: &CanonicalArtifact<'_>) -> Option<[u8; 32]> {
    let mut bytes = Vec::new();
    for value in [
        artifact.metadata().icc_profile(),
        artifact.metadata().exif(),
        artifact.metadata().xmp(),
        artifact.metadata().iptc(),
    ]
    .into_iter()
    .flatten()
    {
        bytes.extend_from_slice(value);
    }
    (!bytes.is_empty()).then(|| Sha256::digest(bytes).into())
}

fn settings_digest(settings: Settings) -> [u8; 32] {
    Sha256::digest(format!(
        "v{}:{settings:?}",
        settings::SETTINGS_SCHEMA_VERSION
    ))
    .into()
}

fn log2(value: u8) -> u8 {
    u8::try_from(u32::from(value).ilog2()).unwrap_or(0)
}
