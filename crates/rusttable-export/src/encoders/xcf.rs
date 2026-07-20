//! Flattened, deterministic XCF export for one RGB(A) raster layer.

use std::fmt;
use std::io::{self, Write};
use std::path::Path;

use rusttable_color::ColorEncoding;
use rusttable_image::{AlphaMode, ChannelLayout, SampleType, StorageLayout};
use sha2::{Digest, Sha256};

use crate::capabilities::{EncoderCapabilityDescriptor, MetadataField};
use crate::encoders::raster::{
    RasterError, digest, row, sample_f32, sample_u16, shape, validate_metadata,
};
use crate::{CanonicalArtifact, EncodeBudget, EncodeCancellation, NeverCancel};

pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
pub const TILE_SIZE: u32 = 64;
const MAGIC: &[u8] = b"gimp xcf file\n\0";
const MAX_METADATA_BYTES: usize = 16 * 1024 * 1024;
const MAX_OUTPUT_BYTES: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    U8,
    U16,
    F32,
}
impl Precision {
    const fn sample(self) -> SampleType {
        match self {
            Self::U8 => SampleType::U8,
            Self::U16 => SampleType::U16,
            Self::F32 => SampleType::F32,
        }
    }
    const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F32 => 4,
        }
    }
    const fn version(self) -> u32 {
        match self {
            Self::F32 => 19,
            Self::U8 | Self::U16 => 18,
        }
    }
    const fn code(self) -> u32 {
        match self {
            Self::U8 => 0x0001_0000,
            Self::U16 => 0x0002_0000,
            Self::F32 => 0x0003_0000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Rle,
    Raw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub precision: Precision,
    pub channels: ChannelLayout,
    pub linear: bool,
    pub compression: Compression,
    pub max_metadata_bytes: usize,
    pub max_output_bytes: usize,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            precision: Precision::U8,
            channels: ChannelLayout::Rgba,
            linear: false,
            compression: Compression::Rle,
            max_metadata_bytes: MAX_METADATA_BYTES,
            max_output_bytes: MAX_OUTPUT_BYTES,
        }
    }
}
impl Settings {
    /// # Errors
    /// Returns an error when the layout or bounded limits are invalid.
    pub fn validate(self) -> Result<(), Error> {
        if !matches!(self.channels, ChannelLayout::Rgb | ChannelLayout::Rgba) {
            return Err(Error::UnsupportedLayout(self.channels));
        }
        if self.max_metadata_bytes == 0 || self.max_metadata_bytes > MAX_METADATA_BYTES {
            return Err(Error::InvalidSettings("metadata limit"));
        }
        if self.max_output_bytes == 0 || self.max_output_bytes > MAX_OUTPUT_BYTES {
            return Err(Error::InvalidSettings("output limit"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub version: u32,
    pub precision: Precision,
    pub linear: bool,
    pub channels: ChannelLayout,
    pub compression: Compression,
    pub layer_name: String,
    pub tile_count: u32,
    pub parasite_names: Vec<String>,
    pub profile_sha256: Option<[u8; 32]>,
    pub decoded_sha256: [u8; 32],
    pub encoded_bytes: u64,
    pub output_sha256: [u8; 32],
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub version: u32,
    pub dimensions: rusttable_image::ImageDimensions,
    pub precision: Precision,
    pub channels: ChannelLayout,
    pub layer_count: u32,
    pub layer_name: String,
    pub layer_offset: (i32, i32),
    pub tile_count: u32,
    pub parasite_names: Vec<String>,
    pub profile_sha256: Option<[u8; 32]>,
    pub decoded_sha256: [u8; 32],
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedColor(ColorEncoding),
    EmptyProfile,
    MetadataLimit { limit: usize, actual: usize },
    InvalidText,
    NonFiniteSample,
    InvalidLayerName,
    Cancelled,
    OutputLimit { limit: usize, actual: usize },
    Io(io::Error),
    Malformed(&'static str),
    ArithmeticOverflow,
    RoundTripMismatch,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(v) => write!(f, "invalid XCF settings: {v}"),
            Self::UnsupportedLayout(v) => write!(f, "unsupported XCF layout: {v:?}"),
            Self::UnsupportedSample(v) => write!(f, "unsupported XCF sample: {v:?}"),
            Self::UnsupportedAlpha(v) => write!(f, "unsupported XCF alpha: {v:?}"),
            Self::UnsupportedStorage(v) => write!(f, "unsupported XCF storage: {v:?}"),
            Self::UnsupportedColor(v) => write!(f, "unsupported XCF color: {v:?}"),
            Self::EmptyProfile => f.write_str("XCF ICC profile is empty"),
            Self::MetadataLimit { limit, actual } => {
                write!(f, "XCF metadata is {actual} bytes, limit is {limit}")
            }
            Self::InvalidText => f.write_str("invalid XCF metadata text"),
            Self::NonFiniteSample => f.write_str("XCF input contains a non-finite sample"),
            Self::InvalidLayerName => f.write_str("invalid XCF layer name"),
            Self::Cancelled => f.write_str("XCF encoding cancelled"),
            Self::OutputLimit { limit, actual } => {
                write!(f, "XCF output is {actual} bytes, limit is {limit}")
            }
            Self::Io(v) => write!(f, "XCF I/O failed: {v}"),
            Self::Malformed(v) => write!(f, "malformed XCF: {v}"),
            Self::ArithmeticOverflow => f.write_str("XCF arithmetic overflowed"),
            Self::RoundTripMismatch => f.write_str("XCF independent decode differs"),
        }
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(v: io::Error) -> Self {
        Self::Io(v)
    }
}

#[must_use]
pub fn capabilities() -> EncoderCapabilityDescriptor {
    let mut d = EncoderCapabilityDescriptor::new("xcf")
        .with_format(rusttable_image::OutputFormat::Xcf)
        .with_channel_layout(crate::ChannelLayout::Rgb)
        .with_channel_layout(crate::ChannelLayout::Rgba)
        .with_bit_depth(crate::BitDepth::Eight)
        .with_bit_depth(crate::BitDepth::Sixteen)
        .supports_float32()
        .supports_profiles()
        .supports_alpha();
    for field in [
        MetadataField::Exif,
        MetadataField::Iptc,
        MetadataField::Xmp,
        MetadataField::IccAndCicp,
        MetadataField::SoftwareAndVersion,
    ] {
        d = d.with_metadata(field);
    }
    d
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
    /// # Errors
    /// Returns an error when the artifact cannot be encoded as one valid XCF layer.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.encode_to_vec_with_budget(artifact, EncodeBudget::default(), &NeverCancel)
    }
    /// # Errors
    /// Returns an error when the artifact, budget, staging, or cancellation contract fails.
    pub fn encode_to_vec_with_budget<C: EncodeCancellation>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        budget: EncodeBudget,
        cancel: &C,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        validate(artifact, self.settings)?;
        let dim = artifact.image().descriptor().dimensions();
        let columns = dim.width().div_ceil(TILE_SIZE);
        let rows = dim.height().div_ceil(TILE_SIZE);
        let tile_count = columns.checked_mul(rows).ok_or(Error::ArithmeticOverflow)?;
        let name = layer_name()?;
        let mut out = Vec::new();
        let mut state = State {
            digest: Sha256::new(),
            tiles: Vec::with_capacity(tile_count as usize),
        };
        write_header(&mut out, artifact, self.settings);
        let layer_slot = reserve(&mut out);
        let layer_offset = pos(&out)?;
        patch(&mut out, layer_slot, layer_offset)?;
        write_layer(&mut out, dim, &name);
        let hierarchy_slot = reserve(&mut out);
        let hierarchy = pos(&out)?;
        patch(&mut out, hierarchy_slot, hierarchy)?;
        write_hierarchy(
            &mut out,
            artifact,
            self.settings,
            columns,
            rows,
            &mut state,
            cancel,
        )?;
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        if out.len() > self.settings.max_output_bytes
            || out.len()
                > usize::try_from(budget.memory_bytes())
                    .unwrap_or(usize::MAX)
                    .saturating_add(MAX_METADATA_BYTES)
        {
            return Err(Error::OutputLimit {
                limit: self.settings.max_output_bytes,
                actual: out.len(),
            });
        }
        let parsed = inspect(&out)?;
        let decoded: [u8; 32] = state.digest.finalize().into();
        if decoded != parsed.decoded_sha256 {
            return Err(Error::RoundTripMismatch);
        }
        let receipt = Receipt {
            version: self.settings.precision.version(),
            precision: self.settings.precision,
            linear: self.settings.linear,
            channels: self.settings.channels,
            compression: self.settings.compression,
            layer_name: name,
            tile_count,
            parasite_names: parsed.parasite_names.clone(),
            profile_sha256: parsed.profile_sha256,
            decoded_sha256: decoded,
            encoded_bytes: out.len() as u64,
            output_sha256: digest(&out),
        };
        Ok((out, receipt))
    }
    /// # Errors
    /// Returns an error when encoding or path staging fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        let (bytes, receipt) = self.encode_to_vec(artifact)?;
        let result = (|| {
            let mut file = std::fs::File::create(path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            Ok::<_, io::Error>(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(path);
        }
        result.map_err(Error::Io)?;
        Ok(receipt)
    }
}

fn validate(a: &CanonicalArtifact<'_>, s: Settings) -> Result<(), Error> {
    let raster = shape(a).map_err(map_error)?;
    if raster.sample_type != s.precision.sample() {
        return Err(Error::UnsupportedSample(raster.sample_type));
    }
    let source = a.image().descriptor().format();
    let source_channels = source.channels();
    if source_channels != s.channels
        && !(matches!(source_channels, ChannelLayout::Gray | ChannelLayout::GrayA)
            && matches!(s.channels, ChannelLayout::Rgb | ChannelLayout::Rgba))
    {
        return Err(Error::UnsupportedLayout(source_channels));
    }
    if source.alpha() == AlphaMode::Premultiplied {
        return Err(Error::UnsupportedAlpha(AlphaMode::Premultiplied));
    }
    if source.storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage(source.storage()));
    }
    if matches!(
        a.image().descriptor().color_encoding(),
        ColorEncoding::External(_)
    ) && a.metadata().icc_profile().is_none()
    {
        return Err(Error::UnsupportedColor(
            a.image().descriptor().color_encoding(),
        ));
    }
    validate_metadata(a, s.max_metadata_bytes).map_err(map_error)
}
fn layer_name() -> Result<String, Error> {
    let value = "RustTable image";
    if value.as_bytes().contains(&0) {
        Err(Error::InvalidLayerName)
    } else {
        Ok(value.to_owned())
    }
}
fn map_error(e: RasterError) -> Error {
    match e {
        RasterError::Unsupported("premultiplied alpha") => {
            Error::UnsupportedAlpha(AlphaMode::Premultiplied)
        }
        RasterError::Unsupported(_) => Error::Malformed("unsupported raster"),
        RasterError::MetadataLimit { limit, actual } => Error::MetadataLimit { limit, actual },
        RasterError::EmptyProfile => Error::EmptyProfile,
        RasterError::InvalidText => Error::InvalidText,
        RasterError::NonFiniteSample => Error::NonFiniteSample,
        RasterError::InvalidImage | RasterError::InvalidLimit => Error::Malformed("invalid raster"),
    }
}

fn property(out: &mut Vec<u8>, kind: u32, data: &[u8]) {
    put(out, kind);
    put(
        out,
        u32::try_from(data.len()).expect("bounded XCF property"),
    );
    out.extend_from_slice(data);
}
fn parasite(out: &mut Vec<u8>, name: &str, data: &[u8]) {
    string(out, name).expect("validated parasite");
    put(out, 3);
    put(
        out,
        u32::try_from(data.len()).expect("bounded XCF parasite"),
    );
    out.extend_from_slice(data);
}
fn string(out: &mut Vec<u8>, value: &str) -> Result<(), Error> {
    if value.as_bytes().contains(&0) {
        return Err(Error::InvalidText);
    }
    put(
        out,
        u32::try_from(value.len() + 1).map_err(|_| Error::ArithmeticOverflow)?,
    );
    out.extend_from_slice(value.as_bytes());
    out.push(0);
    Ok(())
}
fn put(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn reserve(out: &mut Vec<u8>) -> usize {
    let slot = out.len();
    out.extend_from_slice(&[0; 4]);
    slot
}
fn pos(out: &[u8]) -> Result<u32, Error> {
    u32::try_from(out.len()).map_err(|_| Error::ArithmeticOverflow)
}
fn patch(out: &mut [u8], slot: usize, value: u32) -> Result<(), Error> {
    out.get_mut(slot..slot + 4)
        .ok_or(Error::Malformed("offset patch"))?
        .copy_from_slice(&value.to_be_bytes());
    Ok(())
}

fn write_header(out: &mut Vec<u8>, a: &CanonicalArtifact<'_>, s: Settings) {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(format!("{:03}\0", s.precision.version()).as_bytes());
    let dim = a.image().descriptor().dimensions();
    put(out, dim.width());
    put(out, dim.height());
    put(out, 0);
    property(
        out,
        1,
        &[match s.compression {
            Compression::Rle => 0,
            Compression::Raw => 3,
        }],
    );
    property(out, 25, &s.precision.code().to_be_bytes());
    let mut ps = Vec::new();
    if let Some(v) = a.metadata().icc_profile() {
        parasite(&mut ps, "icc-profile", v);
    }
    if let Some(v) = a.metadata().xmp() {
        parasite(&mut ps, "gimp-metadata", v);
    }
    if let Some(v) = a.metadata().exif() {
        parasite(&mut ps, "exif-data", v);
    }
    if let Some(v) = a.metadata().iptc() {
        parasite(&mut ps, "iptc-data", v);
    }
    for v in a.metadata().text() {
        parasite(&mut ps, v.keyword(), v.value().as_bytes());
    }
    if !ps.is_empty() {
        property(out, 21, &ps);
    }
    property(out, 0, &[]);
    put(out, 1);
    put(out, 0);
}
fn write_layer(out: &mut Vec<u8>, dim: rusttable_image::ImageDimensions, name: &str) {
    put(out, dim.width());
    put(out, dim.height());
    string(out, name).expect("validated layer name");
    put(out, 0);
    property(out, 15, &[0; 8]);
    property(out, 6, &100_u32.to_be_bytes());
    property(out, 8, &1_u32.to_be_bytes());
    property(out, 7, &0_u32.to_be_bytes());
    property(out, 0, &[]);
}

struct State {
    digest: Sha256,
    tiles: Vec<u32>,
}
fn write_hierarchy<C: EncodeCancellation>(
    out: &mut Vec<u8>,
    a: &CanonicalArtifact<'_>,
    s: Settings,
    columns: u32,
    rows: u32,
    state: &mut State,
    cancel: &C,
) -> Result<(), Error> {
    let dim = a.image().descriptor().dimensions();
    put(out, dim.width());
    put(out, dim.height());
    put(
        out,
        u32::try_from(s.channels.channels()).expect("bounded channel count")
            * u32::try_from(s.precision.bytes()).expect("bounded sample size"),
    );
    let level_slot = reserve(out);
    let level = pos(out)?;
    patch(out, level_slot, level)?;
    put(out, dim.width());
    put(out, dim.height());
    let count = columns.checked_mul(rows).ok_or(Error::ArithmeticOverflow)?;
    put(out, count);
    let slots = (0..count).map(|_| reserve(out)).collect::<Vec<_>>();
    for index in 0..count {
        if cancel.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let offset = pos(out)?;
        patch(out, slots[index as usize], offset)?;
        let tile = tile(a, s, index, columns)?;
        state.digest.update(&tile.canonical);
        let encoded = match s.compression {
            Compression::Raw => tile.channels.concat(),
            Compression::Rle => tile
                .channels
                .iter()
                .flat_map(|v| rle(v, s.precision.bytes()))
                .collect(),
        };
        out.extend_from_slice(&encoded);
        state.tiles.push(offset);
    }
    Ok(())
}
struct Tile {
    channels: Vec<Vec<u8>>,
    canonical: Vec<u8>,
}
fn tile(a: &CanonicalArtifact<'_>, s: Settings, index: u32, columns: u32) -> Result<Tile, Error> {
    let dim = a.image().descriptor().dimensions();
    let tx = index % columns;
    let ty = index / columns;
    let channels = s.channels.channels();
    let bytes = s.precision.bytes();
    let mut out = vec![Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize * bytes); channels];
    let mut canonical = Vec::with_capacity((TILE_SIZE * TILE_SIZE) as usize * channels * bytes);
    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let sx = tx * TILE_SIZE + x;
            let sy = ty * TILE_SIZE + y;
            let pixel = if sx < dim.width() && sy < dim.height() {
                pixel(a, s, sx, sy)?
            } else {
                vec![0; channels * bytes]
            };
            for (channel, sample) in out.iter_mut().zip(pixel.chunks_exact(bytes)) {
                channel.extend_from_slice(sample);
            }
            canonical.extend_from_slice(&pixel);
        }
    }
    Ok(Tile {
        channels: out,
        canonical,
    })
}
fn pixel(a: &CanonicalArtifact<'_>, s: Settings, x: u32, y: u32) -> Result<Vec<u8>, Error> {
    let format = a.image().descriptor().format();
    let source = row(a, y).map_err(map_error)?;
    let source_channels = format.channels().channels();
    let sample_bytes = format.sample_type().bytes();
    let start =
        usize::try_from(x).map_err(|_| Error::ArithmeticOverflow)? * source_channels * sample_bytes;
    let mut out = Vec::new();
    for channel in 0..s.channels.channels() {
        let source_channel = match (format.channels(), channel) {
            (ChannelLayout::Gray | ChannelLayout::GrayA, 0..=2) => 0,
            (ChannelLayout::GrayA, 3) => 1,
            (ChannelLayout::Gray, 3) => usize::MAX,
            (_, 3) => 3,
            _ => channel,
        };
        let sample = if source_channel == usize::MAX {
            match s.precision {
                Precision::U8 => vec![255],
                Precision::U16 => 65_535_u16.to_be_bytes().to_vec(),
                Precision::F32 => 1.0_f32.to_bits().to_be_bytes().to_vec(),
            }
        } else {
            source
                [start + source_channel * sample_bytes..start + (source_channel + 1) * sample_bytes]
                .to_vec()
        };
        match s.precision {
            Precision::U8 => out.push(sample[0]),
            Precision::U16 => out.extend_from_slice(
                &sample_u16(&sample, format.byte_order())
                    .map_err(map_error)?
                    .to_be_bytes(),
            ),
            Precision::F32 => out.extend_from_slice(
                &sample_f32(&sample, format.byte_order())
                    .map_err(map_error)?
                    .to_bits()
                    .to_be_bytes(),
            ),
        }
    }
    Ok(out)
}

fn rle(data: &[u8], bytes: usize) -> Vec<u8> {
    let count = data.len() / bytes;
    let mut out = Vec::new();
    let mut index = 0;
    while index < count {
        let mut run = 1;
        while index + run < count
            && run < 128
            && data[index * bytes..(index + 1) * bytes]
                == data[(index + run) * bytes..(index + run + 1) * bytes]
        {
            run += 1;
        }
        if run > 1 {
            out.push(0x80 | u8::try_from(run - 1).expect("RLE bound"));
            out.extend_from_slice(&data[index * bytes..(index + 1) * bytes]);
            index += run;
        } else {
            let start = index;
            index += 1;
            while index < count && index - start < 127 {
                if index + 1 < count
                    && data[index * bytes..(index + 1) * bytes]
                        == data[(index + 1) * bytes..(index + 2) * bytes]
                {
                    break;
                }
                index += 1;
            }
            out.push(u8::try_from(index - start - 1).expect("RLE bound"));
            out.extend_from_slice(&data[start * bytes..index * bytes]);
        }
    }
    out
}

/// # Errors
/// Returns an error when bytes do not contain the bounded `RustTable` XCF subset.
#[expect(
    clippy::too_many_lines,
    reason = "the independent parser mirrors the fixed XCF subset"
)]
pub fn inspect(bytes: &[u8]) -> Result<Inspection, Error> {
    let mut c = Cursor::new(bytes);
    c.expect(MAGIC)?;
    let version = std::str::from_utf8(&c.take(4)?[..3])
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or(Error::Malformed("version"))?;
    let width = c.u32()?;
    let height = c.u32()?;
    if c.u32()? != 0 {
        return Err(Error::Malformed("base type"));
    }
    let mut compression = Compression::Rle;
    let mut precision = Precision::U8;
    let mut names = Vec::new();
    let mut profile = None;
    loop {
        let kind = c.u32()?;
        let length = usize::try_from(c.u32()?).map_err(|_| Error::ArithmeticOverflow)?;
        let data = c.take(length)?;
        match kind {
            0 => break,
            1 => {
                compression = if data == [3] {
                    Compression::Raw
                } else {
                    Compression::Rle
                }
            }
            25 => {
                precision = match data {
                    [0, 1, 0, 0] => Precision::U8,
                    [0, 2, 0, 0] => Precision::U16,
                    [0, 3, 0, 0] => Precision::F32,
                    _ => return Err(Error::Malformed("precision")),
                }
            }
            21 => parasites(data, &mut names, &mut profile)?,
            _ => {}
        }
    }
    if c.u32()? != 1 || c.u32()? != 0 {
        return Err(Error::Malformed("layer count"));
    }
    let layer = c.u32()? as usize;
    let mut l = Cursor::new(bytes.get(layer..).ok_or(Error::Malformed("layer offset"))?);
    if l.u32()? != width || l.u32()? != height {
        return Err(Error::Malformed("layer dimensions"));
    }
    let layer_name = l.string()?;
    let _kind = l.u32()?;
    let mut offset = (0, 0);
    loop {
        let kind = l.u32()?;
        let length = usize::try_from(l.u32()?).map_err(|_| Error::ArithmeticOverflow)?;
        let data = l.take(length)?;
        match kind {
            0 => break,
            15 => {
                offset = (
                    i32::from_be_bytes(
                        data[..4]
                            .try_into()
                            .map_err(|_| Error::Malformed("offset"))?,
                    ),
                    i32::from_be_bytes(
                        data[4..8]
                            .try_into()
                            .map_err(|_| Error::Malformed("offset"))?,
                    ),
                );
            }
            6 if data != 100_u32.to_be_bytes() => return Err(Error::Malformed("opacity")),
            8 if data != 1_u32.to_be_bytes() => return Err(Error::Malformed("visibility")),
            _ => {}
        }
    }
    let hierarchy = l.u32()? as usize;
    let mut h = Cursor::new(
        bytes
            .get(hierarchy..)
            .ok_or(Error::Malformed("hierarchy"))?,
    );
    if h.u32()? != width || h.u32()? != height {
        return Err(Error::Malformed("hierarchy dimensions"));
    }
    let bpp = h.u32()?;
    let level = h.u32()? as usize;
    let mut q = Cursor::new(bytes.get(level..).ok_or(Error::Malformed("level"))?);
    let lw = q.u32()?;
    let lh = q.u32()?;
    let count = q.u32()?;
    if count != lw.div_ceil(TILE_SIZE) * lh.div_ceil(TILE_SIZE) {
        return Err(Error::Malformed("tile count"));
    }
    let mut offsets = Vec::new();
    for _ in 0..count {
        offsets.push(q.u32()? as usize);
    }
    let sample = u32::try_from(precision.bytes()).unwrap_or(u32::MAX);
    let channel_count = if bpp == 3 * sample {
        3
    } else if bpp == 4 * sample {
        4
    } else {
        return Err(Error::Malformed("bytes per pixel"));
    };
    let mut decoded = Sha256::new();
    for (i, start) in offsets.iter().copied().enumerate() {
        let end = offsets.get(i + 1).copied().unwrap_or(bytes.len());
        let payload = bytes
            .get(start..end)
            .ok_or(Error::Malformed("tile bounds"))?;
        let channels = decode_tile(payload, compression, channel_count, precision.bytes())?;
        for y in 0..TILE_SIZE {
            for x in 0..TILE_SIZE {
                for channel in &channels {
                    let at = ((y * TILE_SIZE + x) as usize) * precision.bytes();
                    decoded.update(&channel[at..at + precision.bytes()]);
                }
            }
        }
    }
    Ok(Inspection {
        version,
        dimensions: rusttable_image::ImageDimensions::new(width, height)
            .map_err(|_| Error::Malformed("dimensions"))?,
        precision,
        channels: if channel_count == 4 {
            ChannelLayout::Rgba
        } else {
            ChannelLayout::Rgb
        },
        layer_count: 1,
        layer_name,
        layer_offset: offset,
        tile_count: count,
        parasite_names: names,
        profile_sha256: profile,
        decoded_sha256: decoded.finalize().into(),
    })
}
fn decode_tile(
    data: &[u8],
    compression: Compression,
    channels: usize,
    bytes: usize,
) -> Result<Vec<Vec<u8>>, Error> {
    let size = (TILE_SIZE * TILE_SIZE) as usize * bytes;
    match compression {
        Compression::Raw => {
            if data.len() != size * channels {
                return Err(Error::Malformed("raw tile length"));
            }
            Ok((0..channels)
                .map(|i| data[i * size..(i + 1) * size].to_vec())
                .collect())
        }
        Compression::Rle => {
            let mut out = Vec::new();
            let mut at = 0;
            for _ in 0..channels {
                let mut channel = Vec::with_capacity(size);
                while channel.len() < size {
                    let header = *data.get(at).ok_or(Error::Malformed("RLE header"))?;
                    at += 1;
                    if header & 0x80 != 0 {
                        let count = usize::from(header & 0x7f) + 1;
                        let sample = data
                            .get(at..at + bytes)
                            .ok_or(Error::Malformed("RLE repeat"))?;
                        at += bytes;
                        for _ in 0..count {
                            channel.extend_from_slice(sample);
                        }
                    } else {
                        let count = usize::from(header) + 1;
                        let length = count.checked_mul(bytes).ok_or(Error::ArithmeticOverflow)?;
                        channel.extend_from_slice(
                            data.get(at..at + length)
                                .ok_or(Error::Malformed("RLE literal"))?,
                        );
                        at += length;
                    }
                    if channel.len() > size {
                        return Err(Error::Malformed("RLE overflow"));
                    }
                }
                out.push(channel);
            }
            if at != data.len() {
                return Err(Error::Malformed("RLE trailing bytes"));
            }
            Ok(out)
        }
    }
}
fn parasites(
    data: &[u8],
    names: &mut Vec<String>,
    profile: &mut Option<[u8; 32]>,
) -> Result<(), Error> {
    let mut c = Cursor::new(data);
    while c.remaining() > 0 {
        let name = c.string()?;
        let _flags = c.u32()?;
        let length = usize::try_from(c.u32()?).map_err(|_| Error::ArithmeticOverflow)?;
        let value = c.take(length)?;
        if name == "icc-profile" {
            *profile = Some(digest(value));
        }
        names.push(name);
    }
    Ok(())
}
struct Cursor<'a> {
    data: &'a [u8],
    at: usize,
}
impl<'a> Cursor<'a> {
    const fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }
    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.at)
    }
    fn take(&mut self, len: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(len).ok_or(Error::ArithmeticOverflow)?;
        let value = self
            .data
            .get(self.at..end)
            .ok_or(Error::Malformed("truncated"))?;
        self.at = end;
        Ok(value)
    }
    fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| Error::Malformed("u32"))?,
        ))
    }
    fn expect(&mut self, value: &[u8]) -> Result<(), Error> {
        if self.take(value.len())? == value {
            Ok(())
        } else {
            Err(Error::Malformed("signature"))
        }
    }
    fn string(&mut self) -> Result<String, Error> {
        let len = usize::try_from(self.u32()?).map_err(|_| Error::ArithmeticOverflow)?;
        let value = self.take(len)?;
        if value.last() != Some(&0) {
            return Err(Error::Malformed("string"));
        }
        String::from_utf8(value[..value.len() - 1].to_vec())
            .map_err(|_| Error::Malformed("string encoding"))
    }
}
