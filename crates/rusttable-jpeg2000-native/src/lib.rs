#![forbid(unsafe_code)]
#![doc = "Safe `RustTable` boundary for the pinned pure-Rust JPEG 2000 codec."]

use std::fmt;

pub const CODEC_VERSION: &str = "dicom-toolkit-jpeg2000-0.5.0";

#[derive(Debug, Clone, Copy)]
pub struct Options {
    pub reversible: bool,
    pub decomposition_levels: u8,
    pub code_block_width_exp: u8,
    pub code_block_height_exp: u8,
}

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    Codec(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "invalid JPEG 2000 input: {message}"),
            Self::Codec(message) => write!(formatter, "JPEG 2000 codec failed: {message}"),
        }
    }
}

impl std::error::Error for Error {}

/// # Errors
///
/// Returns an error when the input dimensions, sample layout, or codec contract is invalid.
pub fn encode(
    pixels: &[u8],
    width: u32,
    height: u32,
    components: u8,
    bit_depth: u8,
    options: Options,
) -> Result<Vec<u8>, Error> {
    let bytes_per_sample = usize::from(if bit_depth <= 8 { 1u8 } else { 2u8 });
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(usize::from(components)))
        .and_then(|samples| samples.checked_mul(bytes_per_sample))
        .ok_or(Error::InvalidInput("pixel buffer size overflow"))?;
    if width == 0 || height == 0 || pixels.len() != expected {
        return Err(Error::InvalidInput("dimensions or pixel buffer length"));
    }
    if !(1..=4).contains(&components) || !(1..=16).contains(&bit_depth) {
        return Err(Error::InvalidInput("component or depth limit"));
    }
    let options = dicom_toolkit_jpeg2000::EncodeOptions {
        num_decomposition_levels: options.decomposition_levels,
        reversible: options.reversible,
        code_block_width_exp: options.code_block_width_exp,
        code_block_height_exp: options.code_block_height_exp,
        guard_bits: if options.reversible { 1 } else { 2 },
        use_ht_block_coding: false,
    };
    dicom_toolkit_jpeg2000::encode(
        pixels, width, height, components, bit_depth, false, &options,
    )
    .map_err(Error::Codec)
}

/// # Errors
///
/// Returns an error when the codestream cannot be independently parsed or decoded.
pub fn decode(data: &[u8]) -> Result<Decoded, Error> {
    let image = dicom_toolkit_jpeg2000::Image::new(
        data,
        &dicom_toolkit_jpeg2000::DecodeSettings::default(),
    )
    .map_err(|_| Error::Codec("generated codestream is not independently readable"))?;
    let decoded = image
        .decode_native()
        .map_err(|_| Error::Codec("generated codestream failed independent decode"))?;
    Ok(Decoded {
        pixels: decoded.data,
        width: decoded.width,
        height: decoded.height,
        components: decoded.num_components,
        bit_depth: decoded.bit_depth,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub components: u8,
    pub bit_depth: u8,
}
