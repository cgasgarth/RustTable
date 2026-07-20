#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
#![doc = "Safe `RustTable` boundary for the pinned Rust AV1/AVIF encoder."]

use std::fmt;

use ravif::{AlphaColorMode, BitDepth, ColorModel, Encoder, MatrixCoefficients, PixelRange};

pub const CODEC_VERSION: &str = "ravif-0.13.0/rav1e-0.8.1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    Eight,
    Ten,
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
    pub quality: f32,
    pub alpha_quality: f32,
    pub speed: u8,
    pub depth: Depth,
    pub matrix: Matrix,
    pub full_range: bool,
    pub threads: u16,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataRefs<'a> {
    pub exif: Option<&'a [u8]>,
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
            Self::InvalidInput(message) => write!(formatter, "invalid AVIF input: {message}"),
            Self::InvalidOptions(message) => write!(formatter, "invalid AVIF options: {message}"),
            Self::Codec(message) => write!(formatter, "AVIF codec failed: {message}"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "AVIF output is {actual} bytes; limit is {limit}")
            }
        }
    }
}

impl std::error::Error for Error {}

pub fn encode_8(
    width: u32,
    height: u32,
    color: &[[u8; 3]],
    alpha: Option<&[u8]>,
    options: Options,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    validate(
        width,
        height,
        color.len(),
        alpha.is_none_or(|a| a.len() == color.len()),
        options,
    )?;
    let mut encoder = configured(options);
    if let Some(exif) = metadata.exif {
        encoder = encoder.with_exif(exif);
    }
    let bytes = encoder
        .encode_raw_planes_8_bit(
            width as usize,
            height as usize,
            color.iter().copied(),
            alpha.map(|values| values.iter().copied()),
            range(options),
            matrix(options.matrix),
        )
        .map_err(codec)?;
    enforce_limit(bytes.avif_file, options.max_output_bytes)
}

pub fn encode_10(
    width: u32,
    height: u32,
    color: &[[u16; 3]],
    alpha: Option<&[u16]>,
    options: Options,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    validate(
        width,
        height,
        color.len(),
        alpha.is_none_or(|a| a.len() == color.len()),
        options,
    )?;
    if options.depth != Depth::Ten {
        return Err(Error::InvalidOptions("10-bit input requires 10-bit output"));
    }
    let mut encoder = configured(options);
    if let Some(exif) = metadata.exif {
        encoder = encoder.with_exif(exif);
    }
    let bytes = encoder
        .encode_raw_planes_10_bit(
            width as usize,
            height as usize,
            color.iter().copied(),
            alpha.map(|values| values.iter().copied()),
            range(options),
            matrix(options.matrix),
        )
        .map_err(codec)?;
    enforce_limit(bytes.avif_file, options.max_output_bytes)
}

fn configured(options: Options) -> Encoder<'static> {
    Encoder::new()
        .with_quality(options.quality)
        .with_alpha_quality(options.alpha_quality)
        .with_speed(options.speed)
        .with_bit_depth(match options.depth {
            Depth::Eight => BitDepth::Eight,
            Depth::Ten => BitDepth::Ten,
        })
        .with_internal_color_model(if options.matrix == Matrix::Identity {
            ColorModel::RGB
        } else {
            ColorModel::YCbCr
        })
        .with_num_threads(Some(usize::from(options.threads)))
        .with_alpha_color_mode(AlphaColorMode::UnassociatedDirty)
}

fn validate(
    width: u32,
    height: u32,
    color_len: usize,
    alpha_len_matches: bool,
    options: Options,
) -> Result<(), Error> {
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return Err(Error::InvalidInput("dimensions"));
    }
    let expected = usize::try_from(u64::from(width) * u64::from(height))
        .map_err(|_| Error::InvalidInput("pixel count"))?;
    if color_len != expected || !alpha_len_matches {
        return Err(Error::InvalidInput("plane length"));
    }
    if !options.quality.is_finite()
        || !options.alpha_quality.is_finite()
        || !(0.0..=100.0).contains(&options.quality)
        || !(0.0..=100.0).contains(&options.alpha_quality)
    {
        return Err(Error::InvalidOptions("quality"));
    }
    if !(1..=10).contains(&options.speed) || options.threads == 0 || options.max_output_bytes == 0 {
        return Err(Error::InvalidOptions("speed, threads, or output limit"));
    }
    Ok(())
}

fn matrix(value: Matrix) -> MatrixCoefficients {
    match value {
        Matrix::Bt601 => MatrixCoefficients::BT601,
        Matrix::Bt709 => MatrixCoefficients::BT709,
        Matrix::Bt2020 => MatrixCoefficients::BT2020NCL,
        Matrix::Identity => MatrixCoefficients::Identity,
    }
}

fn range(options: Options) -> PixelRange {
    if options.full_range {
        PixelRange::Full
    } else {
        PixelRange::Limited
    }
}

fn enforce_limit(bytes: Vec<u8>, limit: u64) -> Result<Vec<u8>, Error> {
    if u64::try_from(bytes.len()).map_or(true, |actual| actual > limit) {
        return Err(Error::OutputLimit {
            limit,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

fn codec(error: impl fmt::Debug) -> Error {
    Error::Codec(format!("{error:?}"))
}
