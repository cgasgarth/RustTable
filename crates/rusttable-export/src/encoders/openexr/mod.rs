#![allow(clippy::too_many_lines)]

mod inspect;
mod metadata;
mod settings;

pub use inspect::{Inspection, InspectionError, inspect};
pub use settings::{Compression, LineOrder, NonFinitePolicy, Settings, SettingsError, Storage};

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, Write};
use std::path::Path;

use exr::prelude::{Blocks, Encoding, Image, Layer, SpecificChannels, Vec2, WritableImage};
use exr::prelude::{ReadChannels, ReadLayers};
use rusttable_image::{AlphaMode, ByteOrder, ChannelLayout, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::encoders::raster::{self, RasterError};
use crate::encoders::resource::checked_memory;
use crate::{CanonicalArtifact, EncodeBudget, EncodeCancellation, EncoderCapabilityDescriptor};

pub const ENCODER_ID: &str = "openexr";
pub const CODEC_VERSION: &str = "exr-1.74.2";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: rusttable_image::ImageDimensions,
    pub channels: ChannelLayout,
    pub sample_type: SampleType,
    pub compression: Compression,
    pub storage: Storage,
    pub line_order: LineOrder,
    pub attribute_manifest: Vec<String>,
    pub codec_version: &'static str,
    pub settings_sha256: [u8; 32],
    pub metadata_sha256: Option<[u8; 32]>,
    pub output_sha256: [u8; 32],
    pub encoded_bytes: u64,
    pub chunk_count: u64,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(SettingsError),
    Unsupported(&'static str),
    Raster(&'static str),
    QueueBudget(&'static str),
    Cancelled,
    OutputLimit { limit: u64, actual: u64 },
    Codec(String),
    Io(io::Error),
    Inspection(InspectionError),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "OpenEXR export failed: {self:?}")
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
pub fn capabilities() -> EncoderCapabilityDescriptor {
    use crate::capabilities::MetadataField;
    let mut descriptor = EncoderCapabilityDescriptor::new(ENCODER_ID)
        .with_format(rusttable_image::OutputFormat::OpenExr)
        .with_channel_layout(crate::ChannelLayout::Gray)
        .with_channel_layout(crate::ChannelLayout::Rgb)
        .with_channel_layout(crate::ChannelLayout::Rgba)
        .supports_float16()
        .supports_float32()
        .supports_profiles()
        .supports_alpha();
    for field in [
        MetadataField::Exif,
        MetadataField::Iptc,
        MetadataField::Xmp,
        MetadataField::IccAndCicp,
        MetadataField::SoftwareAndVersion,
        MetadataField::UserFields,
    ] {
        descriptor = descriptor.with_metadata(field);
    }
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

    /// Encodes one validated artifact into an owned `OpenEXR` buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when settings, raster data, metadata, cancellation,
    /// output limits, codec verification, or independent inspection fail.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.encode_to_vec_with_budget(artifact, EncodeBudget::default(), &crate::NeverCancel)
    }

    /// Encodes one artifact while enforcing the caller's memory budget.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, cancellation, bounded encoding, or
    /// independent verification fails.
    pub fn encode_to_vec_with_budget<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        budget: EncodeBudget,
        cancellation: &C,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.validate(artifact, budget, cancellation)?;
        let mut output = Cursor::new(Vec::new());
        let attributes = metadata::build(artifact).map_err(Error::Unsupported)?;
        write_image(&mut output, artifact, self.settings, attributes)?;
        let bytes = output.into_inner();
        let receipt = self.finish(artifact, &bytes)?;
        Ok((bytes, receipt))
    }

    /// Streams one artifact to a caller-owned path and verifies the result.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, path I/O, bounded encoding, or
    /// independent verification fails. A failed path encode is removed.
    pub fn encode_to_path<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
        budget: EncodeBudget,
        cancellation: &C,
    ) -> Result<Receipt, Error> {
        self.validate(artifact, budget, cancellation)?;
        let result = (|| {
            let mut file = File::create(path)?;
            let attributes = metadata::build(artifact).map_err(Error::Unsupported)?;
            write_image(&mut file, artifact, self.settings, attributes)?;
            file.sync_all()?;
            let bytes = read_bounded(path, self.settings.max_output_bytes)?;
            self.finish(artifact, &bytes)
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    fn validate<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        budget: EncodeBudget,
        cancellation: &C,
    ) -> Result<(), Error> {
        self.settings.validate()?;
        if cancellation.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let shape = raster::shape(artifact)?;
        let descriptor = artifact.image().descriptor();
        if descriptor.format().storage() != StorageLayout::Interleaved {
            return Err(Error::Unsupported("planar storage"));
        }
        if shape.channels != self.settings.channels.channels()
            || shape.sample_type != self.settings.sample_type
        {
            return Err(Error::Unsupported("settings do not match canonical raster"));
        }
        if artifact.image().descriptor().format().alpha() == AlphaMode::Premultiplied
            && !matches!(
                self.settings.channels,
                ChannelLayout::GrayA | ChannelLayout::Rgba
            )
        {
            return Err(Error::Unsupported("alpha channel is not representable"));
        }
        checked_memory(
            descriptor.dimensions(),
            shape.channels,
            shape.sample_bytes,
            budget,
        )
        .map_err(Error::QueueBudget)?;
        if self.settings.non_finite == NonFinitePolicy::Reject {
            let view = artifact
                .view()
                .map_err(|_| Error::Unsupported("image view"))?;
            let sample_bytes = self.settings.sample_type.bytes();
            let row_bytes = usize::try_from(descriptor.dimensions().width())
                .map_err(|_| Error::Unsupported("row width"))?
                .checked_mul(shape.channels)
                .and_then(|value| value.checked_mul(sample_bytes))
                .ok_or(Error::Unsupported("row size"))?;
            for y in 0..descriptor.dimensions().height() {
                if cancellation.is_cancelled() {
                    return Err(Error::Cancelled);
                }
                let row = view
                    .row(0, y)
                    .map_err(|_| Error::Unsupported("image row"))?;
                for sample in row[..row_bytes].chunks_exact(sample_bytes) {
                    if sample_is_non_finite(
                        sample,
                        descriptor.format().byte_order(),
                        self.settings.sample_type,
                    ) {
                        return Err(Error::Unsupported("non-finite sample"));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(&self, artifact: &CanonicalArtifact<'_>, bytes: &[u8]) -> Result<Receipt, Error> {
        let actual = u64::try_from(bytes.len()).map_err(|_| Error::OutputLimit {
            limit: self.settings.max_output_bytes,
            actual: u64::MAX,
        })?;
        if actual > self.settings.max_output_bytes {
            return Err(Error::OutputLimit {
                limit: self.settings.max_output_bytes,
                actual,
            });
        }
        let inspection = inspect(bytes)?;
        let decoded = exr::prelude::read()
            .no_deep_data()
            .largest_resolution_level()
            .all_channels()
            .first_valid_layer()
            .all_attributes()
            .from_buffered(Cursor::new(bytes))
            .map_err(|error| Error::Codec(error.to_string()))?;
        if decoded.layer_data.size != Vec2(inspection.width as usize, inspection.height as usize) {
            return Err(Error::Inspection(InspectionError::Invalid(
                "decoded dimensions",
            )));
        }
        Ok(Receipt {
            dimensions: artifact.image().descriptor().dimensions(),
            channels: self.settings.channels,
            sample_type: self.settings.sample_type,
            compression: self.settings.compression,
            storage: self.settings.storage,
            line_order: self.settings.line_order,
            attribute_manifest: inspection.attribute_names,
            codec_version: CODEC_VERSION,
            settings_sha256: Sha256::digest(self.settings.canonical_bytes()).into(),
            metadata_sha256: metadata_digest(artifact),
            output_sha256: Sha256::digest(bytes).into(),
            encoded_bytes: actual,
            chunk_count: inspection.chunk_count,
        })
    }
}

macro_rules! write_typed {
    ($writer:expr, $image:expr) => {
        $image
            .write()
            .non_parallel()
            .to_buffered($writer)
            .map_err(|error| Error::Codec(error.to_string()))
    };
}

fn write_image<W: Write + Seek>(
    writer: &mut W,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
    attributes: metadata::Attributes,
) -> Result<(), Error> {
    let dimensions = artifact.image().descriptor().dimensions();
    let size = (
        usize::try_from(dimensions.width()).map_err(|_| Error::Unsupported("width"))?,
        usize::try_from(dimensions.height()).map_err(|_| Error::Unsupported("height"))?,
    );
    let encoding = exr_encoding(settings);
    let image_attributes = attributes.image;
    let layer_attributes = attributes.layer;
    let view = artifact
        .view()
        .map_err(|_| Error::Unsupported("image view"))?;
    let format = artifact.image().descriptor().format();
    let order = format.byte_order();
    match (settings.channels, settings.sample_type) {
        (ChannelLayout::Gray, SampleType::F32) => {
            let channels = SpecificChannels::build()
                .with_channel::<f32>("Y")
                .with_pixel_fn(move |position| (sample_f32(view, position, 0, order),));
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::GrayA, SampleType::F32) => {
            let channels = SpecificChannels::build()
                .with_channel::<f32>("Y")
                .with_channel::<f32>("A")
                .with_pixel_fn(move |position| {
                    (
                        sample_f32(view, position, 0, order),
                        sample_f32(view, position, 1, order),
                    )
                });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::Rgb, SampleType::F32) => {
            let channels = SpecificChannels::build()
                .with_channel::<f32>("R")
                .with_channel::<f32>("G")
                .with_channel::<f32>("B")
                .with_pixel_fn(move |position| {
                    (
                        sample_f32(view, position, 0, order),
                        sample_f32(view, position, 1, order),
                        sample_f32(view, position, 2, order),
                    )
                });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::Rgba, SampleType::F32) => {
            let channels = SpecificChannels::rgba(move |position| {
                (
                    sample_f32(view, position, 0, order),
                    sample_f32(view, position, 1, order),
                    sample_f32(view, position, 2, order),
                    sample_f32(view, position, 3, order),
                )
            });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::Gray, SampleType::F16) => {
            let channels = SpecificChannels::build()
                .with_channel::<exr::prelude::f16>("Y")
                .with_pixel_fn(move |position| (sample_f16(view, position, 0, order),));
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::GrayA, SampleType::F16) => {
            let channels = SpecificChannels::build()
                .with_channel::<exr::prelude::f16>("Y")
                .with_channel::<exr::prelude::f16>("A")
                .with_pixel_fn(move |position| {
                    (
                        sample_f16(view, position, 0, order),
                        sample_f16(view, position, 1, order),
                    )
                });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::Rgb, SampleType::F16) => {
            let channels = SpecificChannels::build()
                .with_channel::<exr::prelude::f16>("R")
                .with_channel::<exr::prelude::f16>("G")
                .with_channel::<exr::prelude::f16>("B")
                .with_pixel_fn(move |position| {
                    (
                        sample_f16(view, position, 0, order),
                        sample_f16(view, position, 1, order),
                        sample_f16(view, position, 2, order),
                    )
                });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        (ChannelLayout::Rgba, SampleType::F16) => {
            let channels = SpecificChannels::rgba(move |position| {
                (
                    sample_f16(view, position, 0, order),
                    sample_f16(view, position, 1, order),
                    sample_f16(view, position, 2, order),
                    sample_f16(view, position, 3, order),
                )
            });
            write_typed!(
                writer,
                Image::new(
                    image_attributes,
                    Layer::new(size, layer_attributes, encoding, channels)
                )
            )?;
        }
        _ => return Err(Error::Unsupported("OpenEXR sample/layout")),
    }
    Ok(())
}

fn exr_encoding(settings: Settings) -> Encoding {
    Encoding {
        compression: match settings.compression {
            Compression::None => exr::prelude::Compression::Uncompressed,
            Compression::Rle => exr::prelude::Compression::RLE,
            Compression::Zip1 => exr::prelude::Compression::ZIP1,
            Compression::Zip16 => exr::prelude::Compression::ZIP16,
            Compression::Piz => exr::prelude::Compression::PIZ,
            Compression::Pxr24 => exr::prelude::Compression::PXR24,
            Compression::B44 => exr::prelude::Compression::B44,
            Compression::B44A => exr::prelude::Compression::B44A,
        },
        blocks: match settings.storage {
            Storage::ScanLines => Blocks::ScanLines,
            Storage::Tiles { width, height } => {
                Blocks::Tiles(Vec2(width as usize, height as usize))
            }
        },
        line_order: match settings.line_order {
            LineOrder::Increasing => exr::prelude::LineOrder::Increasing,
            LineOrder::Decreasing => exr::prelude::LineOrder::Decreasing,
        },
    }
}

fn sample_f32(
    view: rusttable_image::ImageView<'_>,
    position: Vec2<usize>,
    channel: usize,
    order: ByteOrder,
) -> f32 {
    let format = view.descriptor().format();
    let row = view
        .row(0, u32::try_from(position.y()).unwrap_or(u32::MAX))
        .unwrap_or(&[]);
    let offset = position
        .x()
        .saturating_mul(format.channels().channels())
        .saturating_add(channel)
        .saturating_mul(4);
    let bytes = row.get(offset..offset.saturating_add(4)).unwrap_or(&[0; 4]);
    f32::from_bits(match order {
        ByteOrder::Big => u32::from_be_bytes(bytes.try_into().unwrap_or([0; 4])),
        ByteOrder::Little => u32::from_le_bytes(bytes.try_into().unwrap_or([0; 4])),
        ByteOrder::Native => u32::from_ne_bytes(bytes.try_into().unwrap_or([0; 4])),
    })
}

fn sample_f16(
    view: rusttable_image::ImageView<'_>,
    position: Vec2<usize>,
    channel: usize,
    order: ByteOrder,
) -> exr::prelude::f16 {
    let format = view.descriptor().format();
    let row = view
        .row(0, u32::try_from(position.y()).unwrap_or(u32::MAX))
        .unwrap_or(&[]);
    let offset = position
        .x()
        .saturating_mul(format.channels().channels())
        .saturating_add(channel)
        .saturating_mul(2);
    let bytes = row.get(offset..offset.saturating_add(2)).unwrap_or(&[0; 2]);
    exr::prelude::f16::from_bits(match order {
        ByteOrder::Big => u16::from_be_bytes(bytes.try_into().unwrap_or([0; 2])),
        ByteOrder::Little => u16::from_le_bytes(bytes.try_into().unwrap_or([0; 2])),
        ByteOrder::Native => u16::from_ne_bytes(bytes.try_into().unwrap_or([0; 2])),
    })
}

fn sample_is_non_finite(bytes: &[u8], order: ByteOrder, sample: SampleType) -> bool {
    match sample {
        SampleType::F16 => !exr::prelude::f16::from_bits(match order {
            ByteOrder::Big => u16::from_be_bytes(bytes.try_into().unwrap_or([0; 2])),
            ByteOrder::Little => u16::from_le_bytes(bytes.try_into().unwrap_or([0; 2])),
            ByteOrder::Native => u16::from_ne_bytes(bytes.try_into().unwrap_or([0; 2])),
        })
        .to_f32()
        .is_finite(),
        SampleType::F32 => !f32::from_bits(match order {
            ByteOrder::Big => u32::from_be_bytes(bytes.try_into().unwrap_or([0; 4])),
            ByteOrder::Little => u32::from_le_bytes(bytes.try_into().unwrap_or([0; 4])),
            ByteOrder::Native => u32::from_ne_bytes(bytes.try_into().unwrap_or([0; 4])),
        })
        .is_finite(),
        _ => false,
    }
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
    if let Some(packet) = artifact.metadata().packet() {
        bytes.extend_from_slice(&packet.canonical_bytes());
    }
    (!bytes.is_empty()).then(|| Sha256::digest(bytes).into())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, Error> {
    let mut file = File::open(path)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    let actual = u64::try_from(bytes.len()).map_err(|_| Error::OutputLimit {
        limit,
        actual: u64::MAX,
    })?;
    if actual > limit {
        return Err(Error::OutputLimit { limit, actual });
    }
    Ok(bytes)
}
