#![forbid(unsafe_code)]
#![allow(clippy::chunks_exact_to_as_chunks)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::too_many_lines)]
#![doc = "Safe `RustTable` boundary for the pinned libheif HEVC encoder."]

use std::fmt;

use libheif_rs::{
    Channel, Chroma, ColorPrimaries, ColorProfileNCLX, ColorSpace, CompressionFormat,
    EncoderQuality, HeifContext, Image, LibHeif,
};

pub const CODEC_VERSION: &str = "libheif-1.23.x (libheif-rs-2.7.0)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Eight,
    Ten,
    Twelve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaMode {
    C420,
    C422,
    C444,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
    Identity,
}

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub quality: Option<u8>,
    pub chroma: ChromaMode,
    pub depth: Depth,
    pub matrix: Matrix,
    pub full_range: bool,
    pub threads: u16,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Planes<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a [u16],
    pub cb: &'a [u16],
    pub cr: &'a [u16],
    pub alpha: Option<&'a [u16]>,
    pub chroma_width: u32,
    pub chroma_height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataRefs<'a> {
    pub icc: Option<&'a [u8]>,
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
}

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    InvalidOptions(&'static str),
    Codec(String),
    OutputLimit { limit: u64, actual: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "invalid HEIF input: {message}"),
            Self::InvalidOptions(message) => write!(formatter, "invalid HEIF options: {message}"),
            Self::Codec(message) => write!(formatter, "HEIF codec failed: {message}"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "HEIF output is {actual} bytes; limit is {limit}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub fn encode(
    planes: Planes<'_>,
    options: Options,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    validate(planes, options, metadata)?;
    let libheif = LibHeif::new_checked().map_err(codec)?;
    let mut context = HeifContext::new().map_err(codec)?;
    let mut image = Image::new(
        planes.width,
        planes.height,
        ColorSpace::YCbCr(chroma(options.chroma)),
    )
    .map_err(codec)?;
    let depth = depth_bits(options.depth);
    image
        .create_plane(Channel::Y, planes.width, planes.height, depth)
        .map_err(codec)?;
    image
        .create_plane(
            Channel::Cb,
            planes.chroma_width,
            planes.chroma_height,
            depth,
        )
        .map_err(codec)?;
    image
        .create_plane(
            Channel::Cr,
            planes.chroma_width,
            planes.chroma_height,
            depth,
        )
        .map_err(codec)?;
    if planes.alpha.is_some() {
        image
            .create_plane(Channel::Alpha, planes.width, planes.height, depth)
            .map_err(codec)?;
        image.set_premultiplied_alpha(false);
    }
    write_plane(
        &mut image,
        Channel::Y,
        planes.y,
        planes.width,
        planes.height,
        depth,
    )?;
    write_plane(
        &mut image,
        Channel::Cb,
        planes.cb,
        planes.chroma_width,
        planes.chroma_height,
        depth,
    )?;
    write_plane(
        &mut image,
        Channel::Cr,
        planes.cr,
        planes.chroma_width,
        planes.chroma_height,
        depth,
    )?;
    if let Some(alpha) = planes.alpha {
        write_plane(
            &mut image,
            Channel::Alpha,
            alpha,
            planes.width,
            planes.height,
            depth,
        )?;
    }
    set_nclx(&mut image, options)?;
    if let Some(icc) = metadata.icc {
        let profile =
            libheif_rs::ColorProfileRaw::new(libheif_rs::color_profile_types::PROF, icc.to_vec());
        image.set_color_profile_raw(&profile).map_err(codec)?;
    }
    let mut encoder = libheif
        .encoder_for_format(CompressionFormat::Hevc)
        .map_err(codec)?;
    if let Some(quality) = options.quality {
        encoder
            .set_quality(EncoderQuality::Lossy(quality))
            .map_err(codec)?;
    } else {
        encoder
            .set_quality(EncoderQuality::LossLess)
            .map_err(codec)?;
    }
    let mut encoding_options = libheif_rs::EncodingOptions::new().map_err(codec)?;
    encoding_options.set_save_alpha_channel(planes.alpha.is_some());
    let handle = context
        .encode_image(&image, &mut encoder, Some(encoding_options))
        .map_err(codec)?;
    if let Some(exif) = metadata.exif {
        context.add_exif_metadata(&handle, exif).map_err(codec)?;
    }
    if let Some(xmp) = metadata.xmp {
        context.add_xmp_metadata(&handle, xmp).map_err(codec)?;
    }
    let bytes = context.write_to_bytes().map_err(codec)?;
    if u64::try_from(bytes.len()).map_or(true, |actual| actual > options.max_output_bytes) {
        return Err(Error::OutputLimit {
            limit: options.max_output_bytes,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

pub fn inspect(
    bytes: &[u8],
    width: u32,
    height: u32,
    has_alpha: bool,
) -> Result<Vec<String>, Error> {
    let context = HeifContext::read_from_bytes(bytes).map_err(codec)?;
    let handle = context.primary_image_handle().map_err(codec)?;
    if handle.width() != width || handle.height() != height {
        return Err(Error::Codec("container dimensions do not match".to_owned()));
    }
    if handle.has_alpha_channel() != has_alpha {
        return Err(Error::Codec(
            "container alpha association does not match".to_owned(),
        ));
    }
    let mut boxes = Vec::new();
    let mut cursor = 0;
    while cursor + 8 <= bytes.len() {
        let size = u32::from_be_bytes(
            bytes[cursor..cursor + 4]
                .try_into()
                .map_err(|_| Error::InvalidInput("box size"))?,
        ) as usize;
        if size < 8 || cursor.checked_add(size).is_none_or(|end| end > bytes.len()) {
            return Err(Error::InvalidInput("box extent"));
        }
        let name = std::str::from_utf8(&bytes[cursor + 4..cursor + 8])
            .map_err(|_| Error::InvalidInput("box name"))?;
        boxes.push(name.to_owned());
        cursor += size;
    }
    if cursor != bytes.len()
        || !boxes.iter().any(|name| name == "ftyp")
        || !boxes.iter().any(|name| name == "meta")
        || !boxes.iter().any(|name| name == "mdat")
    {
        return Err(Error::InvalidInput("incomplete HEIF container"));
    }
    Ok(boxes)
}

fn validate(planes: Planes<'_>, options: Options, metadata: MetadataRefs<'_>) -> Result<(), Error> {
    if planes.width == 0
        || planes.height == 0
        || planes.chroma_width == 0
        || planes.chroma_height == 0
    {
        return Err(Error::InvalidInput("dimensions"));
    }
    let pixels = usize::try_from(u64::from(planes.width) * u64::from(planes.height))
        .map_err(|_| Error::InvalidInput("pixel count"))?;
    let chroma_pixels =
        usize::try_from(u64::from(planes.chroma_width) * u64::from(planes.chroma_height))
            .map_err(|_| Error::InvalidInput("chroma pixel count"))?;
    if planes.y.len() != pixels
        || planes.cb.len() != chroma_pixels
        || planes.cr.len() != chroma_pixels
        || planes.alpha.is_some_and(|alpha| alpha.len() != pixels)
    {
        return Err(Error::InvalidInput("plane length"));
    }
    if options.threads == 0 || options.max_output_bytes == 0 {
        return Err(Error::InvalidOptions("threads or output limit"));
    }
    if options
        .quality
        .is_some_and(|quality| quality == 0 || quality > 100)
    {
        return Err(Error::InvalidOptions("quality"));
    }
    if metadata.icc.is_some_and(<[u8]>::is_empty)
        || metadata.exif.is_some_and(<[u8]>::is_empty)
        || metadata.xmp.is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::InvalidInput("empty metadata"));
    }
    Ok(())
}

fn write_plane(
    image: &mut Image,
    channel: Channel,
    values: &[u16],
    width: u32,
    _height: u32,
    depth: u8,
) -> Result<(), Error> {
    let mut planes = image.planes_mut();
    let plane = match channel {
        Channel::Y => planes.y.as_mut(),
        Channel::Cb => planes.cb.as_mut(),
        Channel::Cr => planes.cr.as_mut(),
        Channel::Alpha => planes.a.as_mut(),
        _ => None,
    }
    .ok_or(Error::Codec("missing image plane".to_owned()))?;
    let bytes_per_sample = if depth <= 8 { 1 } else { 2 };
    let row_bytes = usize::try_from(width)
        .map_err(|_| Error::InvalidInput("plane width"))?
        .checked_mul(bytes_per_sample)
        .ok_or(Error::InvalidInput("plane stride"))?;
    for (row, source) in values
        .chunks(usize::try_from(width).map_err(|_| Error::InvalidInput("plane width"))?)
        .enumerate()
    {
        let start = row
            .checked_mul(plane.stride)
            .ok_or(Error::InvalidInput("plane stride"))?;
        let target = plane
            .data
            .get_mut(start..start + row_bytes)
            .ok_or(Error::Codec("plane allocation is short".to_owned()))?;
        if bytes_per_sample == 1 {
            for (target, value) in target.iter_mut().zip(source) {
                *target = u8::try_from(*value).map_err(|_| Error::InvalidInput("8-bit sample"))?;
            }
        } else {
            for (target, value) in target.chunks_exact_mut(2).zip(source) {
                target.copy_from_slice(&value.to_ne_bytes());
            }
        }
    }
    Ok(())
}

fn set_nclx(image: &mut Image, options: Options) -> Result<(), Error> {
    let mut profile = ColorProfileNCLX::new().ok_or(Error::Codec("nclx allocation".to_owned()))?;
    profile.set_color_primaries(match options.matrix {
        Matrix::Bt2020 => ColorPrimaries::ITU_R_BT_2020_2_and_2100_0,
        Matrix::Bt601 => ColorPrimaries::ITU_R_BT_601_6,
        Matrix::Bt709 | Matrix::Identity => ColorPrimaries::ITU_R_BT_709_5,
    });
    image.set_color_profile_nclx(&profile).map_err(codec)
}

const fn chroma(value: ChromaMode) -> Chroma {
    match value {
        ChromaMode::C420 => Chroma::C420,
        ChromaMode::C422 => Chroma::C422,
        ChromaMode::C444 => Chroma::C444,
    }
}

const fn depth_bits(value: Depth) -> u8 {
    match value {
        Depth::Eight => 8,
        Depth::Ten => 10,
        Depth::Twelve => 12,
    }
}

fn codec(error: impl fmt::Debug) -> Error {
    Error::Codec(format!("{error:?}"))
}
