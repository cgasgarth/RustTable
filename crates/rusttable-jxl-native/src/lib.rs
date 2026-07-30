#![forbid(unsafe_code)]
#![doc = "Safe `RustTable` boundary for the pinned JPEG XL encoder."]

use std::fmt;

use jpegxl_rs::encode::{ColorEncoding, EncoderFrame, EncoderSpeed, Metadata};
use jpegxl_rs::{ThreadsRunner, encoder_builder};

pub const CODEC_VERSION: &str = "libjxl-0.12.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    U8,
    U16,
    F32,
}

impl SampleType {
    const fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F32 => 4,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct EncodeOptions {
    pub lossless: bool,
    pub quality: f32,
    pub effort: u8,
    pub decoding_speed: u8,
    pub threads: u16,
    pub use_container: bool,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub bytes: &'a [u8],
    pub sample: SampleType,
    pub channels: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetadataRefs<'a> {
    pub exif: Option<&'a [u8]>,
    pub xmp: Option<&'a [u8]>,
}

#[derive(Debug)]
pub enum Error {
    InvalidInput(&'static str),
    InvalidOptions(&'static str),
    Allocation,
    Codec(String),
    OutputLimit { limit: u64, actual: usize },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput(message) => write!(formatter, "invalid JPEG XL input: {message}"),
            Self::InvalidOptions(message) => {
                write!(formatter, "invalid JPEG XL options: {message}")
            }
            Self::Allocation => formatter.write_str("JPEG XL runner allocation failed"),
            Self::Codec(message) => write!(formatter, "JPEG XL codec failed: {message}"),
            Self::OutputLimit { limit, actual } => {
                write!(
                    formatter,
                    "JPEG XL output is {actual} bytes; limit is {limit}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

/// # Errors
///
/// Returns an error when dimensions, options, metadata, or codec allocation are invalid.
pub fn encode(
    input: Input<'_>,
    width: u32,
    height: u32,
    options: EncodeOptions,
    metadata: MetadataRefs<'_>,
) -> Result<Vec<u8>, Error> {
    validate(input, width, height, options)?;
    let runner =
        ThreadsRunner::new(None, Some(usize::from(options.threads))).ok_or(Error::Allocation)?;
    let speed = match options.effort {
        1 => EncoderSpeed::Lightning,
        2 => EncoderSpeed::Thunder,
        3 => EncoderSpeed::Falcon,
        4 => EncoderSpeed::Cheetah,
        5 => EncoderSpeed::Hare,
        6 => EncoderSpeed::Wombat,
        7 => EncoderSpeed::Squirrel,
        8 => EncoderSpeed::Kitten,
        9 => EncoderSpeed::Tortoise,
        _ => EncoderSpeed::Glacier,
    };
    let builder = encoder_builder()
        .has_alpha(input.channels == 4)
        .lossless(options.lossless)
        .uses_original_profile(options.lossless)
        .speed(speed)
        .quality(options.quality)
        .use_container(options.use_container)
        .decoding_speed(i64::from(options.decoding_speed))
        .parallel_runner(&runner)
        .color_encoding(if input.channels == 1 {
            ColorEncoding::SrgbLuma
        } else if matches!(input.sample, SampleType::F32) {
            ColorEncoding::LinearSrgb
        } else {
            ColorEncoding::Srgb
        });
    let mut encoder = builder.build().map_err(codec)?;
    let exif = metadata.exif.map(|data| {
        let mut value = vec![0; 4];
        value.extend_from_slice(data);
        value
    });
    if let Some(data) = exif.as_deref() {
        encoder
            .add_metadata(&Metadata::Exif(data), true)
            .map_err(codec)?;
    }
    if let Some(data) = metadata.xmp {
        encoder
            .add_metadata(&Metadata::Xmp(data), true)
            .map_err(codec)?;
    }
    let result = match input.sample {
        SampleType::U8 => {
            let frame = EncoderFrame::new(input.bytes).num_channels(u32::from(input.channels));
            encoder
                .encode_frame::<u8, u8>(&frame, width, height)
                .map(|value| value.data)
        }
        SampleType::U16 => {
            let (chunks, _) = input.bytes.as_chunks::<2>();
            let values = chunks
                .iter()
                .map(|chunk| u16::from_ne_bytes(*chunk))
                .collect::<Vec<_>>();
            let frame = EncoderFrame::new(&values).num_channels(u32::from(input.channels));
            encoder
                .encode_frame::<u16, u16>(&frame, width, height)
                .map(|value| value.data)
        }
        SampleType::F32 => {
            let (chunks, _) = input.bytes.as_chunks::<4>();
            let values = chunks
                .iter()
                .map(|chunk| f32::from_ne_bytes(*chunk))
                .collect::<Vec<_>>();
            let frame = EncoderFrame::new(&values).num_channels(u32::from(input.channels));
            encoder
                .encode_frame::<f32, f32>(&frame, width, height)
                .map(|value| value.data)
        }
    };
    let bytes = result.map_err(codec)?;
    if u64::try_from(bytes.len()).map_or(true, |actual| actual > options.max_output_bytes) {
        return Err(Error::OutputLimit {
            limit: options.max_output_bytes,
            actual: bytes.len(),
        });
    }
    Ok(bytes)
}

fn validate(
    input: Input<'_>,
    width: u32,
    height: u32,
    options: EncodeOptions,
) -> Result<(), Error> {
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return Err(Error::InvalidInput("dimensions"));
    }
    if !matches!(input.channels, 1 | 3 | 4) {
        return Err(Error::InvalidInput("channels"));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .and_then(|value| value.checked_mul(u64::from(input.channels)))
        .ok_or(Error::InvalidInput("pixel count overflow"))?;
    let expected = pixels
        .checked_mul(
            u64::try_from(input.sample.bytes()).map_err(|_| Error::InvalidInput("sample size"))?,
        )
        .ok_or(Error::InvalidInput("buffer size overflow"))?;
    if u64::try_from(input.bytes.len()) != Ok(expected) {
        return Err(Error::InvalidInput("buffer length"));
    }
    if options.threads == 0 || options.max_output_bytes == 0 || !options.quality.is_finite() {
        return Err(Error::InvalidOptions("thread, output, or quality limit"));
    }
    if options.decoding_speed > 4 || !(0.0..=15.0).contains(&options.quality) {
        return Err(Error::InvalidOptions("decoding speed or quality"));
    }
    Ok(())
}

fn codec(error: impl fmt::Debug) -> Error {
    Error::Codec(format!("{error:?}"))
}
