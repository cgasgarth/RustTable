use std::fmt;
use std::io::{self, Cursor, Seek, Write};
use std::path::Path;

use rusttable_color::ColorEncoding;
use rusttable_image::{AlphaMode, ByteOrder, ChannelLayout, SampleType, StorageLayout};
use tiff::encoder::colortype::ColorType;
use tiff::encoder::{
    Compression as CodecCompression, DeflateLevel, Predictor, Rational, TiffEncoder, TiffKind,
    TiffValue,
};
use tiff::tags::{ExtraSamples, ResolutionUnit, SampleFormat, Tag};

use crate::{CanonicalArtifact, Density, DensityUnit};

/// TIFF encoder settings schema version.
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    Classic,
    Big,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Lzw,
    Deflate(DeflateLevelSetting),
    PackBits,
    Zstd,
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeflateLevelSetting {
    Fast,
    Balanced,
    Best,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictorSetting {
    None,
    Horizontal,
    FloatingPoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Strips { rows_per_strip: u32 },
    Tiles { width: u32, height: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alpha {
    Unassociated,
    Associated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub variant: Variant,
    pub sample_type: SampleType,
    pub channels: ChannelLayout,
    pub compression: Compression,
    pub predictor: PredictorSetting,
    pub storage: Storage,
    pub alpha: Alpha,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            variant: Variant::Auto,
            sample_type: SampleType::U16,
            channels: ChannelLayout::Rgba,
            compression: Compression::Deflate(DeflateLevelSetting::Balanced),
            predictor: PredictorSetting::Horizontal,
            storage: Storage::Strips {
                rows_per_strip: 128,
            },
            alpha: Alpha::Unassociated,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedVariant {
    Classic,
    Big,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub dimensions: rusttable_image::ImageDimensions,
    pub variant: ResolvedVariant,
    pub encoded_bytes: u64,
    pub tags: Vec<u16>,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedStorage,
    UnsupportedSample(SampleType),
    UnsupportedCompression(Compression),
    UnsupportedPredictor(PredictorSetting),
    SampleTypeMismatch,
    AlphaMismatch,
    UnsupportedAlpha(AlphaMode),
    UnsupportedOrientation,
    UnsupportedColor(ColorEncoding),
    ClassicOverflow,
    SizeOverflow,
    Codec(String),
    Io(io::Error),
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(s) => write!(f, "invalid TIFF settings: {s}"),
            Self::UnsupportedLayout(l) => write!(f, "unsupported TIFF layout: {l:?}"),
            Self::UnsupportedStorage => {
                f.write_str("tiled TIFF output is not available in the pinned Rust codec")
            }
            Self::UnsupportedSample(s) => write!(f, "unsupported TIFF sample type: {s:?}"),
            Self::UnsupportedCompression(c) => write!(f, "unsupported TIFF compression: {c:?}"),
            Self::UnsupportedPredictor(p) => write!(f, "unsupported TIFF predictor: {p:?}"),
            Self::SampleTypeMismatch => {
                f.write_str("TIFF settings sample type does not match the canonical image")
            }
            Self::AlphaMismatch => {
                f.write_str("TIFF alpha association does not match the canonical image")
            }
            Self::UnsupportedAlpha(a) => write!(f, "unsupported TIFF alpha mode: {a:?}"),
            Self::UnsupportedOrientation => {
                f.write_str("TIFF export requires canonical orientation")
            }
            Self::UnsupportedColor(c) => write!(f, "unsupported TIFF color declaration: {c:?}"),
            Self::ClassicOverflow => f.write_str("classic TIFF would exceed 32-bit offsets"),
            Self::SizeOverflow => f.write_str("TIFF size arithmetic overflowed"),
            Self::Codec(s) => write!(f, "TIFF codec failed: {s}"),
            Self::Io(e) => write!(f, "TIFF I/O failed: {e}"),
            Self::Malformed(s) => write!(f, "malformed TIFF: {s}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

/// Production classic TIFF and `BigTIFF` strip encoder.
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

    /// Encodes an artifact into a self-contained `TIFF` or `BigTIFF` buffer.
    ///
    /// # Errors
    ///
    /// Returns an error when the canonical image or metadata cannot be represented by the
    /// selected TIFF settings, or when encoding and validation fail.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        validate(artifact, self.settings)?;
        let variant = choose_variant(artifact, self.settings)?;
        let mut output = Cursor::new(Vec::new());
        encode_variant(&mut output, artifact, self.settings, variant)?;
        let bytes = output.into_inner();
        let receipt = inspect(&bytes, artifact, variant, self.settings)?;
        Ok((bytes, receipt))
    }

    /// Encodes an artifact to a path, removing a partial file on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when the canonical image or metadata cannot be represented by the
    /// selected TIFF settings, or when file I/O, encoding, or validation fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        validate(artifact, self.settings)?;
        let variant = choose_variant(artifact, self.settings)?;
        let result = (|| {
            let mut file = std::fs::File::create(path)?;
            encode_variant(&mut file, artifact, self.settings, variant)?;
            file.sync_all()?;
            let bytes = std::fs::read(path)?;
            inspect(&bytes, artifact, variant, self.settings)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result
    }
}

fn validate(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<(), Error> {
    let format = artifact.image().descriptor().format();
    if format.storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage);
    }
    if format.channels() != settings.channels || settings.channels.is_mosaic() {
        return Err(Error::UnsupportedLayout(format.channels()));
    }
    if format.sample_type() != settings.sample_type {
        return Err(Error::SampleTypeMismatch);
    }
    if artifact.image().descriptor().orientation() != rusttable_image::Orientation::Normal {
        return Err(Error::UnsupportedOrientation);
    }
    match (format.channels(), format.alpha(), settings.alpha) {
        (ChannelLayout::Gray | ChannelLayout::Rgb, AlphaMode::None, _)
        | (ChannelLayout::GrayA | ChannelLayout::Rgba, AlphaMode::Straight, Alpha::Unassociated)
        | (
            ChannelLayout::GrayA | ChannelLayout::Rgba,
            AlphaMode::Premultiplied,
            Alpha::Associated,
        ) => {}
        (_, AlphaMode::Straight | AlphaMode::Premultiplied, _) => return Err(Error::AlphaMismatch),
        (_, AlphaMode::None, _) => return Err(Error::UnsupportedAlpha(format.alpha())),
    }
    if matches!(settings.storage, Storage::Tiles { .. }) {
        return Err(Error::UnsupportedStorage);
    }
    if matches!(settings.storage, Storage::Strips { rows_per_strip: 0 }) {
        return Err(Error::InvalidSettings("rows per strip"));
    }
    if matches!(settings.compression, Compression::Zstd | Compression::Jpeg) {
        return Err(Error::UnsupportedCompression(settings.compression));
    }
    if matches!(settings.predictor, PredictorSetting::FloatingPoint) {
        return Err(Error::UnsupportedPredictor(settings.predictor));
    }
    if matches!(settings.predictor, PredictorSetting::Horizontal)
        && matches!(settings.sample_type, SampleType::F16 | SampleType::F32)
    {
        return Err(Error::UnsupportedPredictor(settings.predictor));
    }
    if artifact
        .metadata()
        .icc_profile()
        .is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::Malformed("empty ICC profile"));
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
    Ok(())
}

fn choose_variant(
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<ResolvedVariant, Error> {
    let dimensions = artifact.image().descriptor().dimensions();
    let samples = u64::try_from(settings.channels.channels()).map_err(|_| Error::SizeOverflow)?;
    let data = dimensions
        .pixel_count()
        .map_err(|_| Error::SizeOverflow)?
        .checked_mul(samples)
        .and_then(|v| {
            v.checked_mul(u64::try_from(settings.sample_type.bytes()).unwrap_or(u64::MAX))
        })
        .ok_or(Error::SizeOverflow)?;
    let metadata = u64::try_from(artifact.metadata().icc_profile().map_or(0, <[u8]>::len))
        .unwrap_or(u64::MAX)
        .saturating_add(
            u64::try_from(artifact.metadata().exif().map_or(0, <[u8]>::len)).unwrap_or(u64::MAX),
        );
    let estimate = data.saturating_add(metadata).saturating_add(1 << 20);
    match settings.variant {
        Variant::Classic if estimate > u64::from(u32::MAX) => Err(Error::ClassicOverflow),
        Variant::Big => Ok(ResolvedVariant::Big),
        Variant::Auto if estimate > u64::from(u32::MAX) => Ok(ResolvedVariant::Big),
        Variant::Classic | Variant::Auto => Ok(ResolvedVariant::Classic),
    }
}

fn encode_variant<W: Write + Seek>(
    writer: &mut W,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
    variant: ResolvedVariant,
) -> Result<(), Error> {
    let compression = codec_compression(settings.compression)?;
    let predictor = match settings.predictor {
        PredictorSetting::None => Predictor::None,
        PredictorSetting::Horizontal => Predictor::Horizontal,
        PredictorSetting::FloatingPoint => {
            return Err(Error::UnsupportedPredictor(settings.predictor));
        }
    };
    match variant {
        ResolvedVariant::Classic => {
            let mut encoder = TiffEncoder::new(writer)
                .map_err(codec)?
                .with_compression(compression)
                .with_predictor(predictor);
            dispatch(&mut encoder, artifact, settings)?;
        }
        ResolvedVariant::Big => {
            let mut encoder = TiffEncoder::new_big(writer)
                .map_err(codec)?
                .with_compression(compression)
                .with_predictor(predictor);
            dispatch(&mut encoder, artifact, settings)?;
        }
    }
    Ok(())
}

fn dispatch<W: Write + Seek, K: TiffKind>(
    encoder: &mut TiffEncoder<W, K>,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<(), Error> {
    match (settings.channels, settings.sample_type) {
        (ChannelLayout::Gray, SampleType::U8) => {
            write_image::<Gray8, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::GrayA, SampleType::U8) => {
            write_image::<GrayA8, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgb, SampleType::U8) => {
            write_image::<Rgb8, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgba, SampleType::U8) => {
            write_image::<Rgba8, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Gray, SampleType::U16) => {
            write_image::<Gray16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::GrayA, SampleType::U16) => {
            write_image::<GrayA16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgb, SampleType::U16) => {
            write_image::<Rgb16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgba, SampleType::U16) => {
            write_image::<Rgba16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Gray, SampleType::F16) => {
            write_image::<GrayF16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::GrayA, SampleType::F16) => {
            write_image::<GrayAF16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgb, SampleType::F16) => {
            write_image::<RgbF16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgba, SampleType::F16) => {
            write_image::<RgbaF16, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Gray, SampleType::F32) => {
            write_image::<GrayF32, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::GrayA, SampleType::F32) => {
            write_image::<GrayAF32, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgb, SampleType::F32) => {
            write_image::<RgbF32, _, _>(encoder, artifact, settings)
        }
        (ChannelLayout::Rgba, SampleType::F32) => {
            write_image::<RgbaF32, _, _>(encoder, artifact, settings)
        }
        (_, sample) => Err(Error::UnsupportedSample(sample)),
    }
}

fn write_image<C, W, K>(
    encoder: &mut TiffEncoder<W, K>,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<(), Error>
where
    C: ColorType,
    C::Inner: SampleBytes,
    [C::Inner]: TiffValue,
    W: Write + Seek,
    K: TiffKind,
{
    let view = artifact
        .view()
        .map_err(|_| Error::Malformed("image view"))?;
    let dimensions = view.descriptor().dimensions();
    let mut image = encoder
        .new_image::<C>(dimensions.width(), dimensions.height())
        .map_err(codec)?;
    if settings.channels == ChannelLayout::GrayA || settings.channels == ChannelLayout::Rgba {
        let extra = match settings.alpha {
            Alpha::Unassociated => ExtraSamples::UnassociatedAlpha,
            Alpha::Associated => ExtraSamples::AssociatedAlpha,
        };
        image.extra_samples(&[extra]).map_err(codec)?;
    }
    if let Storage::Strips { rows_per_strip } = settings.storage {
        image.rows_per_strip(rows_per_strip).map_err(codec)?;
    }
    write_tags(image.encoder(), artifact, settings)?;
    let mut samples = Vec::<C::Inner>::new();
    for y in 0..dimensions.height() {
        let row = view.row(0, y).map_err(|_| Error::Malformed("row"))?;
        let sample_bytes = settings.sample_type.bytes();
        let row_bytes = usize::try_from(dimensions.width())
            .map_err(|_| Error::SizeOverflow)?
            .checked_mul(settings.channels.channels())
            .and_then(|v| v.checked_mul(sample_bytes))
            .ok_or(Error::SizeOverflow)?;
        for sample in row[..row_bytes].chunks_exact(sample_bytes) {
            samples.push(C::Inner::from_bytes(
                sample,
                view.descriptor().format().byte_order(),
            ));
        }
        let strip_samples =
            usize::try_from(image.next_strip_sample_count()).map_err(|_| Error::SizeOverflow)?;
        if samples.len() >= strip_samples {
            image.write_strip(&samples).map_err(codec)?;
            samples.clear();
        }
    }
    if !samples.is_empty() {
        image.write_strip(&samples).map_err(codec)?;
    }
    image.finish().map_err(codec)
}

fn write_tags<W: Write + Seek, K: TiffKind>(
    encoder: &mut tiff::encoder::DirectoryEncoder<'_, W, K>,
    artifact: &CanonicalArtifact<'_>,
    settings: Settings,
) -> Result<(), Error> {
    encoder.write_tag(Tag::Orientation, 1u16).map_err(codec)?;
    encoder
        .write_tag(Tag::PlanarConfiguration, 1u16)
        .map_err(codec)?;
    if let Some(density) = artifact.metadata().density() {
        let (unit, rational) = density_rational(density);
        encoder
            .write_tag(Tag::ResolutionUnit, unit.to_u16())
            .map_err(codec)?;
        encoder
            .write_tag(Tag::XResolution, rational.clone())
            .map_err(codec)?;
        encoder
            .write_tag(Tag::YResolution, rational)
            .map_err(codec)?;
    }
    if let Some(profile) = artifact.metadata().icc_profile() {
        encoder.write_tag(Tag::IccProfile, profile).map_err(codec)?;
    }
    if let Some(exif) = artifact.metadata().exif() {
        encoder
            .write_tag(Tag::Unknown(37510), exif)
            .map_err(codec)?;
    }
    if let Some(iptc) = artifact.metadata().iptc() {
        encoder
            .write_tag(Tag::Unknown(33723), iptc)
            .map_err(codec)?;
    }
    if let Some(xmp) = artifact.metadata().xmp() {
        encoder.write_tag(Tag::Unknown(700), xmp).map_err(codec)?;
    }
    if let Some(text) = artifact.metadata().text().first() {
        encoder
            .write_tag(Tag::ImageDescription, text.value())
            .map_err(codec)?;
    }
    let _ = settings;
    Ok(())
}

fn density_rational(density: Density) -> (ResolutionUnit, Rational) {
    match density.unit() {
        DensityUnit::Inch => (
            ResolutionUnit::Inch,
            Rational {
                n: density.x(),
                d: 1,
            },
        ),
        DensityUnit::Centimeter => (
            ResolutionUnit::Centimeter,
            Rational {
                n: density.x(),
                d: 1,
            },
        ),
        DensityUnit::Meter => (
            ResolutionUnit::Centimeter,
            Rational {
                n: density.x() / 100,
                d: 1,
            },
        ),
    }
}

fn inspect(
    bytes: &[u8],
    artifact: &CanonicalArtifact<'_>,
    variant: ResolvedVariant,
    settings: Settings,
) -> Result<Receipt, Error> {
    let is_big = bytes.get(2..4) == Some(&[43, 0]);
    if bytes.len() < 8
        || bytes.get(0..2) != Some(b"II")
        || (is_big != (variant == ResolvedVariant::Big))
    {
        return Err(Error::Malformed("header"));
    }
    if !is_big && bytes.get(2..4) != Some(&[42, 0]) {
        return Err(Error::Malformed("classic version"));
    }
    if is_big && bytes.get(4..8) != Some(&[8, 0, 0, 0]) {
        return Err(Error::Malformed("BigTIFF offset size"));
    }
    let mut decoder = tiff::decoder::Decoder::new(Cursor::new(bytes)).map_err(codec)?;
    let dimensions = decoder.dimensions().map_err(codec)?;
    let expected = artifact.image().descriptor().dimensions();
    if dimensions != (expected.width(), expected.height()) {
        return Err(Error::Malformed("dimensions"));
    }
    if decoder
        .get_tag_unsigned::<u16>(Tag::Orientation)
        .map_err(codec)?
        != 1
    {
        return Err(Error::Malformed("orientation"));
    }
    let tags = vec![
        Tag::Orientation.to_u16(),
        Tag::PlanarConfiguration.to_u16(),
        Tag::Compression.to_u16(),
        Tag::Predictor.to_u16(),
        Tag::SampleFormat.to_u16(),
    ];
    let _ = settings;
    Ok(Receipt {
        dimensions: expected,
        variant,
        encoded_bytes: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        tags,
    })
}

fn codec<E: fmt::Display>(error: E) -> Error {
    Error::Codec(error.to_string())
}

trait SampleBytes: TiffValue + Copy {
    fn from_bytes(bytes: &[u8], order: ByteOrder) -> Self;
}
impl SampleBytes for u8 {
    fn from_bytes(bytes: &[u8], _: ByteOrder) -> Self {
        bytes[0]
    }
}
impl SampleBytes for u16 {
    fn from_bytes(bytes: &[u8], order: ByteOrder) -> Self {
        match order {
            ByteOrder::Big => u16::from_be_bytes([bytes[0], bytes[1]]),
            ByteOrder::Little => u16::from_le_bytes([bytes[0], bytes[1]]),
            ByteOrder::Native => u16::from_ne_bytes([bytes[0], bytes[1]]),
        }
    }
}
impl SampleBytes for f32 {
    fn from_bytes(bytes: &[u8], order: ByteOrder) -> Self {
        let value = match order {
            ByteOrder::Big => u32::from_be_bytes(bytes.try_into().unwrap()),
            ByteOrder::Little => u32::from_le_bytes(bytes.try_into().unwrap()),
            ByteOrder::Native => u32::from_ne_bytes(bytes.try_into().unwrap()),
        };
        f32::from_bits(value)
    }
}

macro_rules! integer_type {
    ($name:ident, $inner:ty, $bits:expr, $samples:expr, $photo:expr) => {
        struct $name;
        impl ColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: tiff::tags::PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $samples;
            fn horizontal_predict(row: &[Self::Inner], result: &mut Vec<Self::Inner>) {
                let count = Self::SAMPLE_FORMAT.len();
                result.extend_from_slice(&row[..count.min(row.len())]);
                result.extend(
                    row[count.min(row.len())..]
                        .iter()
                        .zip(&row[..row.len().saturating_sub(count)])
                        .map(|(current, previous)| current.wrapping_sub(*previous)),
                );
            }
        }
    };
}
macro_rules! float_type {
    ($name:ident, $inner:ty, $bits:expr, $samples:expr, $photo:expr) => {
        struct $name;
        impl ColorType for $name {
            type Inner = $inner;
            const TIFF_VALUE: tiff::tags::PhotometricInterpretation = $photo;
            const BITS_PER_SAMPLE: &'static [u16] = $bits;
            const SAMPLE_FORMAT: &'static [SampleFormat] = $samples;
            fn horizontal_predict(_: &[Self::Inner], _: &mut Vec<Self::Inner>) {
                unreachable!("float predictor is rejected before codec entry")
            }
        }
    };
}
const GRAY8: &[u16] = &[8];
const GRAY16: &[u16] = &[16];
const GRAY32: &[u16] = &[32];
const RGB8: &[u16] = &[8, 8, 8];
const RGB16: &[u16] = &[16, 16, 16];
const RGB32: &[u16] = &[32, 32, 32];
const UINT8: &[SampleFormat] = &[SampleFormat::Uint];
const UINT16: &[SampleFormat] = &[SampleFormat::Uint];
const FLOAT16: &[SampleFormat] = &[SampleFormat::IEEEFP];
const FLOAT32: &[SampleFormat] = &[SampleFormat::IEEEFP];
const UINT_RGB8: &[SampleFormat] = &[SampleFormat::Uint; 3];
const UINT_RGB16: &[SampleFormat] = &[SampleFormat::Uint; 3];
const FLOAT_RGB16: &[SampleFormat] = &[SampleFormat::IEEEFP; 3];
const FLOAT_RGB32: &[SampleFormat] = &[SampleFormat::IEEEFP; 3];
integer_type!(
    Gray8,
    u8,
    GRAY8,
    UINT8,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
integer_type!(
    GrayA8,
    u8,
    GRAY8,
    UINT8,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
integer_type!(
    Rgb8,
    u8,
    RGB8,
    UINT_RGB8,
    tiff::tags::PhotometricInterpretation::RGB
);
integer_type!(
    Rgba8,
    u8,
    RGB8,
    UINT_RGB8,
    tiff::tags::PhotometricInterpretation::RGB
);
integer_type!(
    Gray16,
    u16,
    GRAY16,
    UINT16,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
integer_type!(
    GrayA16,
    u16,
    GRAY16,
    UINT16,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
integer_type!(
    Rgb16,
    u16,
    RGB16,
    UINT_RGB16,
    tiff::tags::PhotometricInterpretation::RGB
);
integer_type!(
    Rgba16,
    u16,
    RGB16,
    UINT_RGB16,
    tiff::tags::PhotometricInterpretation::RGB
);
float_type!(
    GrayF16,
    u16,
    GRAY16,
    FLOAT16,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
float_type!(
    GrayAF16,
    u16,
    GRAY16,
    FLOAT16,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
float_type!(
    RgbF16,
    u16,
    RGB16,
    FLOAT_RGB16,
    tiff::tags::PhotometricInterpretation::RGB
);
float_type!(
    RgbaF16,
    u16,
    RGB16,
    FLOAT_RGB16,
    tiff::tags::PhotometricInterpretation::RGB
);
float_type!(
    GrayF32,
    f32,
    GRAY32,
    FLOAT32,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
float_type!(
    GrayAF32,
    f32,
    GRAY32,
    FLOAT32,
    tiff::tags::PhotometricInterpretation::BlackIsZero
);
float_type!(
    RgbF32,
    f32,
    RGB32,
    FLOAT_RGB32,
    tiff::tags::PhotometricInterpretation::RGB
);
float_type!(
    RgbaF32,
    f32,
    RGB32,
    FLOAT_RGB32,
    tiff::tags::PhotometricInterpretation::RGB
);

fn codec_compression(value: Compression) -> Result<CodecCompression, Error> {
    Ok(match value {
        Compression::None => CodecCompression::Uncompressed,
        Compression::Lzw => CodecCompression::Lzw,
        Compression::Deflate(DeflateLevelSetting::Fast) => {
            CodecCompression::Deflate(DeflateLevel::Fast)
        }
        Compression::Deflate(DeflateLevelSetting::Balanced) => {
            CodecCompression::Deflate(DeflateLevel::Balanced)
        }
        Compression::Deflate(DeflateLevelSetting::Best) => {
            CodecCompression::Deflate(DeflateLevel::Best)
        }
        Compression::PackBits => CodecCompression::Packbits,
        Compression::Zstd | Compression::Jpeg => return Err(Error::UnsupportedCompression(value)),
    })
}
