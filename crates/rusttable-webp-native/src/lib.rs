#![forbid(unsafe_code)]
#![doc = "Safe `RustTable` boundary for the pinned WebP encoder and muxer."]

use std::fmt;

use webpx::{EncoderConfig, Unstoppable};

pub const CODEC_VERSION: &str = "libwebp-1.x (webpx 0.4.0)";

#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    pub lossless: bool,
    pub quality: f32,
    pub alpha_quality: u8,
    pub near_lossless: u8,
    pub method: u8,
    pub exact: bool,
    pub sharp_yuv: bool,
    pub threads: u16,
    pub max_output_bytes: u64,
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
            Self::InvalidInput(message) => write!(formatter, "invalid WebP input: {message}"),
            Self::InvalidOptions(message) => write!(formatter, "invalid WebP options: {message}"),
            Self::Codec(message) => write!(formatter, "WebP codec failed: {message}"),
            Self::OutputLimit { limit, actual } => {
                write!(formatter, "WebP output is {actual} bytes; limit is {limit}")
            }
        }
    }
}

impl std::error::Error for Error {}

/// # Errors
///
/// Returns an error when dimensions, options, metadata, or codec allocation are invalid.
pub fn encode_rgb(
    data: &[u8],
    width: u32,
    height: u32,
    options: EncodeOptions,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    encode(data, width, height, false, options, metadata)
}

/// # Errors
///
/// Returns an error when dimensions, options, metadata, or codec allocation are invalid.
pub fn encode_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    options: EncodeOptions,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    encode(data, width, height, true, options, metadata)
}

fn encode(
    data: &[u8],
    width: u32,
    height: u32,
    alpha: bool,
    options: EncodeOptions,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    validate(data, width, height, alpha, options, metadata)?;
    let mut config = EncoderConfig::new()
        .quality(options.quality)
        .lossless(options.lossless)
        .alpha_quality(options.alpha_quality)
        .near_lossless(options.near_lossless)
        .method(options.method)
        .exact(options.exact)
        .sharp_yuv(options.sharp_yuv)
        .thread_level(u8::from(options.threads > 1));
    if let Some(profile) = metadata.icc {
        config = config.icc_profile(profile);
    }
    if let Some(exif) = metadata.exif {
        config = config.exif(exif);
    }
    if let Some(xmp) = metadata.xmp {
        config = config.xmp(xmp);
    }
    let bytes = if alpha {
        config
            .encode_rgba(data, width, height, Unstoppable)
            .map_err(codec)?
    } else {
        config
            .encode_rgb(data, width, height, Unstoppable)
            .map_err(codec)?
    };
    if u64::try_from(bytes.len()).map_or(true, |actual| actual > options.max_output_bytes) {
        return Err(Error::OutputLimit {
            limit: options.max_output_bytes,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

fn validate(
    data: &[u8],
    width: u32,
    height: u32,
    alpha: bool,
    options: EncodeOptions,
    metadata: MetadataRefs<'_>,
) -> Result<(), Error> {
    if width == 0 || height == 0 || width > 16_383 || height > 16_383 {
        return Err(Error::InvalidInput("dimensions"));
    }
    let channels = if alpha { 4_u64 } else { 3 };
    let expected = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|value| value.checked_mul(channels))
        .ok_or(Error::InvalidInput("pixel count overflow"))?;
    if u64::try_from(data.len()) != Ok(expected) {
        return Err(Error::InvalidInput("buffer length"));
    }
    if options.threads == 0 || options.max_output_bytes == 0 || !options.quality.is_finite() {
        return Err(Error::InvalidOptions("thread, output, or quality limit"));
    }
    if !(0.0..=100.0).contains(&options.quality)
        || options.method > 6
        || options.alpha_quality > 100
        || options.near_lossless > 100
    {
        return Err(Error::InvalidOptions("quality, method, or near-lossless"));
    }
    if metadata.icc.is_some_and(<[u8]>::is_empty)
        || metadata.exif.is_some_and(<[u8]>::is_empty)
        || metadata.xmp.is_some_and(<[u8]>::is_empty)
    {
        return Err(Error::InvalidInput("empty metadata"));
    }
    Ok(())
}

fn codec(error: impl fmt::Debug) -> Error {
    Error::Codec(format!("{error:?}"))
}
