use std::borrow::Cow;
use std::fmt;
use std::io::{self, Cursor, Write};
use std::path::Path;

use rusttable_color::ColorEncoding;
use rusttable_image::{AlphaMode, ByteOrder, ChannelLayout, SampleType, StorageLayout};

use crate::{CanonicalArtifact, Density, DensityUnit};

const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 1 << 20;

/// PNG settings schema version.
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitDepth {
    Eight,
    Sixteen,
}
impl BitDepth {
    const fn bytes(self) -> usize {
        match self {
            Self::Eight => 1,
            Self::Sixteen => 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Fast,
    Balanced,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Adaptive,
    None,
    Sub,
    Up,
    Average,
    Paeth,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetadataPolicy {
    All,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub bit_depth: BitDepth,
    pub channels: ChannelLayout,
    pub compression: Compression,
    pub filter: Filter,
    pub interlace: bool,
    pub metadata: MetadataPolicy,
    pub max_metadata_bytes: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bit_depth: BitDepth::Sixteen,
            channels: ChannelLayout::Rgba,
            compression: Compression::Balanced,
            filter: Filter::Adaptive,
            interlace: false,
            metadata: MetadataPolicy::All,
            max_metadata_bytes: MAX_METADATA_BYTES,
        }
    }
}

impl Settings {
    /// Validates settings before an encoding attempt.
    ///
    /// # Errors
    ///
    /// Returns an error for an invalid metadata limit or mosaic channel layout.
    pub fn validate(self) -> Result<(), Error> {
        if self.max_metadata_bytes == 0 || self.max_metadata_bytes > MAX_METADATA_BYTES {
            return Err(Error::InvalidSettings("metadata limit"));
        }
        if self.channels.is_mosaic() {
            return Err(Error::UnsupportedLayout(self.channels));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: rusttable_image::ImageDimensions,
    pub encoded_bytes: u64,
    pub chunks: Vec<String>,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    SampleDepthMismatch,
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedOrientation,
    UnsupportedColor(ColorEncoding),
    UnsupportedMetadata(&'static str),
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    InvalidText,
    InvalidDensity,
    Codec(String),
    Io(io::Error),
    Malformed(&'static str),
    PixelMismatch,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(s) => write!(f, "invalid PNG settings: {s}"),
            Self::UnsupportedLayout(l) => write!(f, "unsupported PNG layout: {l:?}"),
            Self::UnsupportedSample(s) => write!(f, "unsupported PNG sample type: {s:?}"),
            Self::SampleDepthMismatch => f.write_str("PNG settings depth does not match samples"),
            Self::UnsupportedAlpha(a) => write!(f, "unsupported PNG alpha mode: {a:?}"),
            Self::UnsupportedStorage(s) => write!(f, "unsupported PNG storage: {s:?}"),
            Self::UnsupportedOrientation => {
                f.write_str("PNG export requires canonical orientation")
            }
            Self::UnsupportedColor(c) => write!(f, "unsupported PNG color declaration: {c:?}"),
            Self::UnsupportedMetadata(s) => write!(f, "unsupported PNG metadata: {s}"),
            Self::EmptyProfile => f.write_str("ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => {
                write!(f, "PNG metadata is {actual} bytes, limit is {limit}")
            }
            Self::InvalidText => f.write_str("PNG text metadata is invalid"),
            Self::InvalidDensity => f.write_str("PNG density cannot be represented"),
            Self::Codec(s) => write!(f, "PNG codec failed: {s}"),
            Self::Io(e) => write!(f, "PNG I/O failed: {e}"),
            Self::Malformed(s) => write!(f, "malformed PNG: {s}"),
            Self::PixelMismatch => {
                f.write_str("PNG decoded samples differ from the canonical artifact")
            }
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Stateless production PNG encoder.
#[derive(Debug, Clone, Copy, Default)]
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

    /// Encodes an artifact into a self-contained PNG buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the canonical image or metadata cannot be represented by PNG,
    /// or when encoding and validation fail.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        let mut bytes = Vec::new();
        self.encode_into(artifact, &mut bytes)?;
        let receipt = inspect(&bytes, artifact, self.settings)?;
        Ok((bytes, receipt))
    }

    /// Encodes an artifact to a path, removing a partial file on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the canonical image or metadata cannot be represented by PNG,
    /// or when file I/O, encoding, or validation fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        let result = (|| {
            let file = std::fs::File::create(path)?;
            let mut writer = io::BufWriter::new(file);
            self.encode_into(artifact, &mut writer)?;
            writer.flush()?;
            writer
                .into_inner()
                .map_err(|error| Error::Io(error.into_error()))?
                .sync_all()?;
            let bytes = std::fs::read(path)?;
            inspect(&bytes, artifact, self.settings)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    fn encode_into<W: Write>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        output: &mut W,
    ) -> Result<(), Error> {
        let view = artifact
            .view()
            .map_err(|_| Error::Malformed("image view"))?;
        let format = view.descriptor().format();
        let color = png_color(self.settings.channels);
        let mut info = png::Info::with_size(
            view.descriptor().dimensions().width(),
            view.descriptor().dimensions().height(),
        );
        info.color_type = color;
        info.bit_depth = match self.settings.bit_depth {
            BitDepth::Eight => png::BitDepth::Eight,
            BitDepth::Sixteen => png::BitDepth::Sixteen,
        };
        info.interlaced = self.settings.interlace;
        let cicp = if artifact.metadata().icc_profile().is_none()
            && self.settings.metadata == MetadataPolicy::All
        {
            cicp(artifact.image().descriptor().color_encoding())
        } else {
            None
        };
        let (srgb, gamma, chroma) = color_info(artifact.image().descriptor().color_encoding());
        if artifact.metadata().icc_profile().is_none() {
            info.srgb = srgb;
            if cicp.is_none() {
                info.source_gamma = gamma;
                info.source_chromaticities = chroma;
            }
        }
        if self.settings.metadata == MetadataPolicy::All {
            add_metadata(&mut info, artifact, self.settings)?;
        }
        let mut encoder = png::Encoder::with_info(output, info).map_err(codec)?;
        set_compression(&mut encoder, self.settings.compression);
        encoder.set_filter(png_filter(self.settings.filter));
        let mut writer = encoder.write_header().map_err(codec)?;
        if let Some(cicp) = cicp {
            writer.write_chunk(png::chunk::cICP, &cicp).map_err(codec)?;
        }
        if self.settings.interlace {
            let raw = full_png_data(&view, format, self.settings)?;
            writer.write_image_data(&raw).map_err(codec)?;
            writer.finish().map_err(codec)?;
        } else {
            {
                let mut stream = writer.stream_writer().map_err(codec)?;
                let row_len = usize::try_from(view.descriptor().dimensions().width())
                    .ok()
                    .and_then(|w| w.checked_mul(format.bytes_per_pixel()))
                    .ok_or(Error::Malformed("row length overflow"))?;
                let mut row = Vec::new();
                row.try_reserve_exact(row_len)
                    .map_err(|_| Error::Codec("row allocation".into()))?;
                for y in 0..view.descriptor().dimensions().height() {
                    pack_row(&view, y, self.settings, &mut row)?;
                    stream.write_all(&row)?;
                }
                stream.finish().map_err(codec)?;
            }
            writer.finish().map_err(codec)?;
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
    if format.channels() != settings.channels {
        return Err(Error::UnsupportedLayout(format.channels()));
    }
    if !matches!(format.alpha(), AlphaMode::None | AlphaMode::Straight) {
        return Err(Error::UnsupportedAlpha(format.alpha()));
    }
    let expected = match (format.sample_type(), settings.bit_depth) {
        (SampleType::U8, BitDepth::Eight) | (SampleType::U16, BitDepth::Sixteen) => true,
        (SampleType::U8 | SampleType::U16, _) => return Err(Error::SampleDepthMismatch),
        (sample, _) => return Err(Error::UnsupportedSample(sample)),
    };
    let _ = expected;
    if artifact
        .metadata()
        .icc_profile()
        .is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::EmptyProfile);
    }
    if artifact.metadata().iptc().is_some() {
        return Err(Error::UnsupportedMetadata("IPTC has no PNG standard chunk"));
    }
    if settings.metadata == MetadataPolicy::All {
        let actual = metadata_len(artifact);
        if actual > settings.max_metadata_bytes {
            return Err(Error::MetadataLimit {
                limit: settings.max_metadata_bytes,
                actual,
            });
        }
    }
    if matches!(
        artifact.image().descriptor().color_encoding(),
        ColorEncoding::External(_)
    ) && artifact.metadata().icc_profile().is_none()
    {
        return Err(Error::UnsupportedColor(
            artifact.image().descriptor().color_encoding(),
        ));
    }
    if artifact.metadata().icc_profile().is_none() {
        let encoding = artifact.image().descriptor().color_encoding();
        let (srgb, gamma, chromaticities) = color_info(encoding);
        if encoding != ColorEncoding::Unspecified
            && srgb.is_none()
            && gamma.is_none()
            && chromaticities.is_none()
            && cicp(encoding).is_none()
        {
            return Err(Error::UnsupportedColor(encoding));
        }
    }
    Ok(())
}

fn metadata_len(artifact: &CanonicalArtifact<'_>) -> usize {
    let m = artifact.metadata();
    m.icc_profile().map_or(0, <[u8]>::len)
        + m.exif().map_or(0, <[u8]>::len)
        + m.xmp().map_or(0, <[u8]>::len)
        + m.iptc().map_or(0, <[u8]>::len)
        + m.text()
            .iter()
            .map(|v| v.keyword().len() + v.value().len())
            .sum::<usize>()
}

fn png_color(layout: ChannelLayout) -> png::ColorType {
    match layout {
        ChannelLayout::Gray | ChannelLayout::Bayer | ChannelLayout::XTrans => {
            png::ColorType::Grayscale
        }
        ChannelLayout::GrayA => png::ColorType::GrayscaleAlpha,
        ChannelLayout::Rgb => png::ColorType::Rgb,
        ChannelLayout::Rgba => png::ColorType::Rgba,
    }
}

fn add_metadata(
    info: &mut png::Info<'static>,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<(), Error> {
    let metadata = artifact.metadata();
    if let Some(profile) = metadata.icc_profile() {
        info.icc_profile = Some(Cow::Owned(profile.to_vec()));
    }
    if let Some(exif) = metadata.exif() {
        info.exif_metadata = Some(Cow::Owned(exif.to_vec()));
    }
    if let Some(density) = metadata.density() {
        info.pixel_dims = Some(pixel_dimensions(density)?);
    }
    if let Some(xmp) = metadata.xmp() {
        info.utf8_text.push(png::text_metadata::ITXtChunk::new(
            "XML:com.adobe.xmp",
            String::from_utf8(xmp.to_vec()).map_err(|_| Error::InvalidText)?,
        ));
    }
    for text in metadata.text() {
        if text.keyword().len() > 79
            || text.keyword().is_empty()
            || text.value().len() > MAX_TEXT_BYTES
        {
            return Err(Error::InvalidText);
        }
        info.utf8_text.push(png::text_metadata::ITXtChunk::new(
            text.keyword().to_owned(),
            text.value().to_owned(),
        ));
    }
    let _ = settings;
    Ok(())
}

fn pixel_dimensions(density: Density) -> Result<png::PixelDimensions, Error> {
    let (x, y) = match density.unit() {
        DensityUnit::Meter => (density.x(), density.y()),
        DensityUnit::Centimeter => (
            density.x().checked_mul(100).ok_or(Error::InvalidDensity)?,
            density.y().checked_mul(100).ok_or(Error::InvalidDensity)?,
        ),
        DensityUnit::Inch => (
            u32::try_from(u64::from(density.x()) * 5000 / 127)
                .map_err(|_| Error::InvalidDensity)?,
            u32::try_from(u64::from(density.y()) * 5000 / 127)
                .map_err(|_| Error::InvalidDensity)?,
        ),
    };
    Ok(png::PixelDimensions {
        xppu: x,
        yppu: y,
        unit: png::Unit::Meter,
    })
}

fn set_compression(encoder: &mut png::Encoder<'_, impl Write>, compression: Compression) {
    encoder.set_deflate_compression(match compression {
        Compression::None => png::DeflateCompression::NoCompression,
        Compression::Fast => png::DeflateCompression::FdeflateUltraFast,
        Compression::Balanced => png::DeflateCompression::Level(6),
        Compression::Best => png::DeflateCompression::Level(9),
    });
}
fn png_filter(filter: Filter) -> png::Filter {
    match filter {
        Filter::Adaptive => png::Filter::Adaptive,
        Filter::None => png::Filter::NoFilter,
        Filter::Sub => png::Filter::Sub,
        Filter::Up => png::Filter::Up,
        Filter::Average => png::Filter::Avg,
        Filter::Paeth => png::Filter::Paeth,
    }
}

fn cicp(encoding: ColorEncoding) -> Option<[u8; 4]> {
    let (p, t) = match encoding {
        ColorEncoding::Rec2020D65 => (9, 14),
        ColorEncoding::LinearRec2020D65 => (9, 8),
        _ => return None,
    };
    Some([p, t, 0, 1])
}

type ColorInfo = (
    Option<png::SrgbRenderingIntent>,
    Option<png::ScaledFloat>,
    Option<png::SourceChromaticities>,
);

fn color_info(encoding: ColorEncoding) -> ColorInfo {
    let srgb =
        matches!(encoding, ColorEncoding::SrgbD65).then_some(png::SrgbRenderingIntent::Perceptual);
    let builtin = encoding.builtin();
    let gamma = encoding.transfer().map(|transfer| {
        png::ScaledFloat::new(match transfer {
            rusttable_color::TransferFunction::Linear => 1.0,
            _ => 0.45455,
        })
    });
    let chroma = builtin
        .and_then(rusttable_color::BuiltinSpace::primaries)
        .map(|primaries| {
            png::SourceChromaticities::new(
                primaries.white().xy(),
                (primaries.red().0.get(), primaries.red().1.get()),
                (primaries.green().0.get(), primaries.green().1.get()),
                (primaries.blue().0.get(), primaries.blue().1.get()),
            )
        });
    (srgb, gamma, chroma)
}

fn pack_row(
    view: &rusttable_image::ImageView<'_>,
    y: u32,
    settings: Settings,
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    out.clear();
    let row = view.row(0, y).map_err(|_| Error::Malformed("row"))?;
    let width = usize::try_from(view.descriptor().dimensions().width())
        .map_err(|_| Error::Malformed("width"))?;
    let samples = settings.channels.channels();
    let sample_bytes = settings.bit_depth.bytes();
    out.reserve(width * samples * sample_bytes);
    let byte_order = view.descriptor().format().byte_order();
    for pixel in row[..width * samples * sample_bytes].chunks_exact(sample_bytes) {
        match settings.bit_depth {
            BitDepth::Eight => out.push(pixel[0]),
            BitDepth::Sixteen => {
                let value = read_u16(pixel, byte_order);
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    Ok(())
}

fn full_png_data(
    view: &rusttable_image::ImageView<'_>,
    format: rusttable_image::PixelFormat,
    settings: Settings,
) -> Result<Vec<u8>, Error> {
    let row_len = usize::try_from(view.descriptor().dimensions().width())
        .ok()
        .and_then(|w| w.checked_mul(format.bytes_per_pixel()))
        .ok_or(Error::Malformed("row length"))?;
    let height = usize::try_from(view.descriptor().dimensions().height())
        .map_err(|_| Error::Malformed("height"))?;
    let mut raw = Vec::new();
    raw.try_reserve(
        row_len
            .checked_mul(height)
            .ok_or(Error::Malformed("image size"))?,
    )
    .map_err(|_| Error::Codec("image allocation".into()))?;
    let mut row = Vec::new();
    for y in 0..u32::try_from(height).map_err(|_| Error::Malformed("height"))? {
        pack_row(view, y, settings, &mut row)?;
        raw.extend_from_slice(&row);
    }
    Ok(raw)
}

fn read_u16(bytes: &[u8], order: ByteOrder) -> u16 {
    match order {
        ByteOrder::Big => u16::from_be_bytes([bytes[0], bytes[1]]),
        ByteOrder::Little => u16::from_le_bytes([bytes[0], bytes[1]]),
        ByteOrder::Native => u16::from_ne_bytes([bytes[0], bytes[1]]),
    }
}

fn inspect(
    bytes: &[u8],
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<Receipt, Error> {
    if bytes.len() < 8 || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(Error::Malformed("signature"));
    }
    let mut cursor = 8usize;
    let mut chunks = Vec::new();
    let mut seen_iend = false;
    while cursor < bytes.len() {
        let len = usize::try_from(u32::from_be_bytes(
            bytes
                .get(cursor..cursor + 4)
                .ok_or(Error::Malformed("length"))?
                .try_into()
                .unwrap(),
        ))
        .unwrap();
        cursor += 4;
        let kind = bytes
            .get(cursor..cursor + 4)
            .ok_or(Error::Malformed("chunk type"))?;
        cursor += 4;
        let data = bytes
            .get(cursor..cursor + len)
            .ok_or(Error::Malformed("chunk data"))?;
        cursor += len;
        let crc = bytes
            .get(cursor..cursor + 4)
            .ok_or(Error::Malformed("CRC"))?;
        cursor += 4;
        if crc32(&[kind, data].concat()) != u32::from_be_bytes(crc.try_into().unwrap()) {
            return Err(Error::Malformed("CRC"));
        }
        chunks.push(String::from_utf8_lossy(kind).into_owned());
        if kind == b"IEND" {
            seen_iend = true;
            break;
        }
    }
    if !seen_iend || cursor != bytes.len() {
        return Err(Error::Malformed("end marker"));
    }
    let png_decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = png_decoder.read_info().map_err(codec)?;
    let mut decoded_data = vec![
        0;
        reader
            .output_buffer_size()
            .ok_or(Error::Malformed("decoder size"))?
    ];
    let info = reader.next_frame(&mut decoded_data).map_err(codec)?;
    if info.width != artifact.image().descriptor().dimensions().width()
        || info.height != artifact.image().descriptor().dimensions().height()
    {
        return Err(Error::Malformed("dimensions"));
    }
    let required = match settings.channels {
        ChannelLayout::Gray => png::ColorType::Grayscale,
        ChannelLayout::GrayA => png::ColorType::GrayscaleAlpha,
        ChannelLayout::Rgb => png::ColorType::Rgb,
        ChannelLayout::Rgba => png::ColorType::Rgba,
        _ => return Err(Error::UnsupportedLayout(settings.channels)),
    };
    if info.color_type != required {
        return Err(Error::Malformed("color type"));
    }
    Ok(Receipt {
        dimensions: artifact.image().descriptor().dimensions(),
        encoded_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        chunks,
    })
}

fn codec<E: fmt::Display>(error: E) -> Error {
    Error::Codec(error.to_string())
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}
