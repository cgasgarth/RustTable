//! Streaming PPM/PGM/PFM encoders and staged-output validators.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ImageDimensions, SampleType, StorageLayout,
};
use sha2::{Digest, Sha256};

use crate::CanonicalArtifact;

pub const ENCODER_ID_PPM: &str = "ppm";
pub const ENCODER_ID_PFM: &str = "pfm";
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 512 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Ppm,
    Pgm,
    Pfm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegerDepth {
    Eight,
    Sixteen,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NonFinitePolicy {
    Reject,
    Canonicalize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub format: Format,
    pub integer_depth: IntegerDepth,
    pub non_finite: NonFinitePolicy,
    pub max_output_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            format: Format::Ppm,
            integer_depth: IntegerDepth::Eight,
            non_finite: NonFinitePolicy::Reject,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }
}

impl Settings {
    ///
    /// # Errors
    ///
    /// Returns an error when the output bound or format/depth pairing is invalid.
    pub fn validate(self) -> Result<(), Error> {
        if self.max_output_bytes == 0 {
            return Err(Error::InvalidSettings(
                "maximum output bytes must be nonzero",
            ));
        }
        if self.format == Format::Pfm && self.integer_depth != IntegerDepth::Eight {
            return Err(Error::InvalidSettings("PFM has fixed 32-bit float samples"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub formats: &'static [Format],
    pub integer_depths: &'static [IntegerDepth],
    pub float32_little_endian: bool,
}

#[must_use]
pub const fn capabilities() -> Capabilities {
    Capabilities {
        formats: &[Format::Ppm, Format::Pgm, Format::Pfm],
        integer_depths: &[IntegerDepth::Eight, IntegerDepth::Sixteen],
        float32_little_endian: true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub format: Format,
    pub dimensions: ImageDimensions,
    pub encoded_bytes: u64,
    pub payload_bytes: u64,
    pub sample_sha256: [u8; 32],
    pub artifact_sha256: [u8; 32],
    pub non_finite_count: u64,
    pub little_endian: bool,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    UnsupportedLayout(ChannelLayout),
    UnsupportedSample(SampleType),
    UnsupportedAlpha(AlphaMode),
    UnsupportedStorage(StorageLayout),
    UnsupportedByteOrder(ByteOrder),
    UnsupportedMetadata,
    UnsupportedFormatInput {
        format: Format,
        sample: SampleType,
        channels: ChannelLayout,
    },
    InvalidDimensions,
    ArithmeticOverflow,
    OutputTooLarge {
        limit: u64,
        actual: u64,
    },
    NonFinite {
        row: u32,
        sample: usize,
    },
    AllocationFailure,
    ImageView,
    Io(io::Error),
    Malformed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(message) => {
                write!(formatter, "invalid portable-anymap settings: {message}")
            }
            Self::UnsupportedLayout(layout) => {
                write!(formatter, "unsupported portable-anymap layout: {layout:?}")
            }
            Self::UnsupportedSample(sample) => write!(
                formatter,
                "unsupported portable-anymap sample type: {sample:?}"
            ),
            Self::UnsupportedAlpha(alpha) => write!(
                formatter,
                "portable-anymap does not support alpha: {alpha:?}"
            ),
            Self::UnsupportedStorage(storage) => write!(
                formatter,
                "unsupported portable-anymap storage: {storage:?}"
            ),
            Self::UnsupportedByteOrder(order) => {
                write!(formatter, "unsupported source byte order: {order:?}")
            }
            Self::UnsupportedMetadata => {
                formatter.write_str("portable-anymap does not support metadata or profiles")
            }
            Self::UnsupportedFormatInput {
                format,
                sample,
                channels,
            } => write!(
                formatter,
                "{format:?} cannot encode {sample:?}/{channels:?}"
            ),
            Self::InvalidDimensions => {
                formatter.write_str("portable-anymap dimensions must be nonzero")
            }
            Self::ArithmeticOverflow => {
                formatter.write_str("portable-anymap size arithmetic overflowed")
            }
            Self::OutputTooLarge { limit, actual } => write!(
                formatter,
                "portable-anymap output is {actual} bytes, limit is {limit}"
            ),
            Self::NonFinite { row, sample } => {
                write!(formatter, "PFM sample {sample} in row {row} is not finite")
            }
            Self::AllocationFailure => {
                formatter.write_str("portable-anymap row buffer allocation failed")
            }
            Self::ImageView => formatter.write_str("canonical artifact image view is invalid"),
            Self::Io(error) => write!(formatter, "portable-anymap I/O failed: {error}"),
            Self::Malformed(message) => {
                write!(formatter, "malformed portable-anymap artifact: {message}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
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

    /// Encodes into an owned buffer and validates its deterministic header and payload length.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, packing, or staged-output validation fails.
    pub fn encode_to_vec(
        &self,
        artifact: &CanonicalArtifact<'_>,
    ) -> Result<(Vec<u8>, Receipt), Error> {
        self.settings.validate()?;
        let mut bytes = Vec::new();
        let receipt = self.encode_to_writer(artifact, &mut bytes)?;
        validate_bytes(
            &bytes,
            self.settings.format,
            artifact.image().descriptor().format().channels() == ChannelLayout::Gray,
            receipt.dimensions,
            receipt.payload_bytes,
            self.settings.integer_depth,
        )?;
        Ok((bytes, receipt))
    }

    /// Writes only a bounded header and one reusable row buffer at a time.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, packing, or writer handling fails.
    pub fn encode_to_writer<W: Write>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        self.encode_rows(artifact, writer, &mut || false)
    }

    /// The queue worker can use this checkpoint between every source row.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, cancellation, packing, or writer handling fails.
    pub fn encode_to_writer_with_cancel<W: Write, C: FnMut() -> bool>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
        mut cancelled: C,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        self.encode_rows(artifact, writer, &mut cancelled)
    }

    /// Encodes to a staging path and removes the partial artifact on failure.
    ///
    /// # Errors
    ///
    /// Returns an error when validation, encoding, verification, or staging I/O fails.
    pub fn encode_to_path(
        &self,
        artifact: &CanonicalArtifact<'_>,
        path: &Path,
    ) -> Result<Receipt, Error> {
        self.settings.validate()?;
        validate_artifact(artifact, self.settings)?;
        let result = (|| {
            let file = File::create(path)?;
            let mut writer = BufWriter::new(file);
            let receipt = self.encode_rows(artifact, &mut writer, &mut || false)?;
            writer.flush()?;
            writer
                .into_inner()
                .map_err(|error| Error::Io(error.into_error()))?
                .sync_all()?;
            let bytes = fs::read(path)?;
            validate_bytes(
                &bytes,
                self.settings.format,
                artifact.image().descriptor().format().channels() == ChannelLayout::Gray,
                receipt.dimensions,
                receipt.payload_bytes,
                self.settings.integer_depth,
            )?;
            Ok(receipt)
        })();
        if result.is_err() {
            let _ = fs::remove_file(path);
        }
        result
    }

    fn encode_rows<W: Write, C: FnMut() -> bool>(
        &self,
        artifact: &CanonicalArtifact<'_>,
        writer: &mut W,
        cancelled: &mut C,
    ) -> Result<Receipt, Error> {
        let view = artifact.view().map_err(|_| Error::ImageView)?;
        let dimensions = view.descriptor().dimensions();
        let gray = view.descriptor().format().channels() == ChannelLayout::Gray;
        let header = header(
            self.settings.format,
            gray,
            dimensions,
            self.settings.integer_depth,
        )?;
        let planned = output_size(
            self.settings.format,
            gray,
            dimensions,
            self.settings.integer_depth,
        )?;
        if planned > self.settings.max_output_bytes {
            return Err(Error::OutputTooLarge {
                limit: self.settings.max_output_bytes,
                actual: planned,
            });
        }
        let mut sink = DigestWriter::new(writer);
        sink.write_all(&header)?;
        let row_bytes = row_bytes(
            self.settings.format,
            gray,
            dimensions.width(),
            self.settings.integer_depth,
        )?;
        let mut row = Vec::new();
        row.try_reserve_exact(row_bytes)
            .map_err(|_| Error::AllocationFailure)?;
        let mut sample_hasher = Sha256::new();
        let mut payload_bytes = 0u64;
        let mut non_finite_count = 0u64;
        for source_y in 0..dimensions.height() {
            if cancelled() {
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "portable-anymap encoding cancelled",
                )));
            }
            let row_y = if self.settings.format == Format::Pfm {
                dimensions.height() - 1 - source_y
            } else {
                source_y
            };
            pack_row(&view, row_y, self.settings, &mut row, &mut non_finite_count)?;
            sample_hasher.update(&row);
            payload_bytes = payload_bytes
                .checked_add(u64::try_from(row.len()).map_err(|_| Error::ArithmeticOverflow)?)
                .ok_or(Error::ArithmeticOverflow)?;
            sink.write_all(&row)?;
        }
        let (encoded_bytes, artifact_sha256) = sink.finish();
        Ok(Receipt {
            format: self.settings.format,
            dimensions,
            encoded_bytes,
            payload_bytes,
            sample_sha256: sample_hasher.finalize().into(),
            artifact_sha256,
            non_finite_count,
            little_endian: self.settings.format == Format::Pfm,
        })
    }
}

fn validate_artifact(artifact: &CanonicalArtifact<'_>, settings: Settings) -> Result<(), Error> {
    let descriptor = artifact.image().descriptor();
    let dimensions = descriptor.dimensions();
    if dimensions.width() == 0 || dimensions.height() == 0 {
        return Err(Error::InvalidDimensions);
    }
    if descriptor.orientation() != rusttable_image::Orientation::Normal {
        return Err(Error::Malformed("input orientation is not canonical"));
    }
    let format = descriptor.format();
    if format.storage() != StorageLayout::Interleaved {
        return Err(Error::UnsupportedStorage(format.storage()));
    }
    if format.alpha() != AlphaMode::None {
        return Err(Error::UnsupportedAlpha(format.alpha()));
    }
    if !matches!(format.channels(), ChannelLayout::Gray | ChannelLayout::Rgb) {
        return Err(Error::UnsupportedLayout(format.channels()));
    }
    if settings.format == Format::Pfm {
        if format.sample_type() != SampleType::F32 {
            return Err(Error::UnsupportedFormatInput {
                format: settings.format,
                sample: format.sample_type(),
                channels: format.channels(),
            });
        }
    } else if !matches!(
        (settings.integer_depth, format.sample_type()),
        (IntegerDepth::Eight, SampleType::U8) | (IntegerDepth::Sixteen, SampleType::U16)
    ) {
        return Err(Error::UnsupportedFormatInput {
            format: settings.format,
            sample: format.sample_type(),
            channels: format.channels(),
        });
    }
    match settings.format {
        Format::Pgm if format.channels() != ChannelLayout::Gray => {
            return Err(Error::UnsupportedFormatInput {
                format: settings.format,
                sample: format.sample_type(),
                channels: format.channels(),
            });
        }
        Format::Ppm if format.channels() != ChannelLayout::Rgb => {
            return Err(Error::UnsupportedFormatInput {
                format: settings.format,
                sample: format.sample_type(),
                channels: format.channels(),
            });
        }
        Format::Pfm if !matches!(format.channels(), ChannelLayout::Gray | ChannelLayout::Rgb) => {
            return Err(Error::UnsupportedFormatInput {
                format: settings.format,
                sample: format.sample_type(),
                channels: format.channels(),
            });
        }
        _ => {}
    }
    if artifact.metadata().icc_profile().is_some()
        || artifact.metadata().exif().is_some()
        || artifact.metadata().xmp().is_some()
        || artifact.metadata().iptc().is_some()
        || artifact.metadata().density().is_some()
        || !artifact.metadata().text().is_empty()
    {
        return Err(Error::UnsupportedMetadata);
    }
    Ok(())
}

fn header(
    format: Format,
    gray: bool,
    dimensions: ImageDimensions,
    depth: IntegerDepth,
) -> Result<Vec<u8>, Error> {
    let maxval = match depth {
        IntegerDepth::Eight => 255,
        IntegerDepth::Sixteen => 65_535,
    };
    let header = match format {
        Format::Ppm => format!(
            "P6\n{} {}\n{}\n",
            dimensions.width(),
            dimensions.height(),
            maxval
        ),
        Format::Pgm => format!(
            "P5\n{} {}\n{}\n",
            dimensions.width(),
            dimensions.height(),
            maxval
        ),
        Format::Pfm => format!(
            "{}\n{} {}\n-1.0\n",
            if gray { "Pf" } else { "PF" },
            dimensions.width(),
            dimensions.height()
        ),
    };
    if header.len() > MAX_HEADER_BYTES {
        return Err(Error::Malformed("header exceeds bound"));
    }
    Ok(header.into_bytes())
}

fn output_size(
    format: Format,
    gray: bool,
    dimensions: ImageDimensions,
    depth: IntegerDepth,
) -> Result<u64, Error> {
    let channels = match format {
        Format::Pgm => 1u64,
        Format::Ppm => 3u64,
        Format::Pfm => u64::from(!gray) + 1,
    };
    let bytes = match format {
        Format::Pfm => 4u64,
        Format::Ppm | Format::Pgm => match depth {
            IntegerDepth::Eight => 1,
            IntegerDepth::Sixteen => 2,
        },
    };
    let payload = u64::from(dimensions.width())
        .checked_mul(u64::from(dimensions.height()))
        .and_then(|v| v.checked_mul(channels))
        .and_then(|v| v.checked_mul(bytes))
        .ok_or(Error::ArithmeticOverflow)?;
    let header = u64::try_from(header(format, gray, dimensions, depth)?.len())
        .map_err(|_| Error::ArithmeticOverflow)?;
    header.checked_add(payload).ok_or(Error::ArithmeticOverflow)
}

fn row_bytes(format: Format, gray: bool, width: u32, depth: IntegerDepth) -> Result<usize, Error> {
    let channels = match format {
        Format::Pgm => 1usize,
        Format::Ppm => 3usize,
        Format::Pfm => usize::from(!gray) + 1,
    };
    let bytes = match format {
        Format::Pfm => 4usize,
        Format::Ppm | Format::Pgm => match depth {
            IntegerDepth::Eight => 1,
            IntegerDepth::Sixteen => 2,
        },
    };
    usize::try_from(width)
        .ok()
        .and_then(|w| w.checked_mul(channels))
        .and_then(|v| v.checked_mul(bytes))
        .ok_or(Error::ArithmeticOverflow)
}

fn pack_row(
    view: &rusttable_image::ImageView<'_>,
    y: u32,
    settings: Settings,
    output: &mut Vec<u8>,
    non_finite_count: &mut u64,
) -> Result<(), Error> {
    output.clear();
    let width = usize::try_from(view.descriptor().dimensions().width())
        .map_err(|_| Error::ArithmeticOverflow)?;
    let channels = view.descriptor().format().channels().channels();
    let row = view.row(0, y).map_err(|_| Error::ImageView)?;
    match settings.format {
        Format::Ppm | Format::Pgm => {
            let bytes = match settings.integer_depth {
                IntegerDepth::Eight => 1,
                IntegerDepth::Sixteen => 2,
            };
            output.reserve(
                width
                    .checked_mul(channels)
                    .and_then(|v| v.checked_mul(bytes))
                    .ok_or(Error::ArithmeticOverflow)?,
            );
            for sample in row[..width * channels * bytes].chunks_exact(bytes) {
                if bytes == 1 {
                    output.push(sample[0]);
                } else {
                    let value = match view.descriptor().format().byte_order() {
                        ByteOrder::Big => u16::from_be_bytes([sample[0], sample[1]]),
                        ByteOrder::Little => u16::from_le_bytes([sample[0], sample[1]]),
                        ByteOrder::Native => u16::from_ne_bytes([sample[0], sample[1]]),
                    };
                    output.extend_from_slice(&value.to_be_bytes());
                }
            }
        }
        Format::Pfm => {
            output.reserve(
                width
                    .checked_mul(channels)
                    .and_then(|v| v.checked_mul(4))
                    .ok_or(Error::ArithmeticOverflow)?,
            );
            let (samples, _) = row[..width * channels * 4].as_chunks::<4>();
            for (sample_index, sample) in samples.iter().enumerate() {
                let mut value = match view.descriptor().format().byte_order() {
                    ByteOrder::Little => f32::from_le_bytes(*sample),
                    ByteOrder::Big => f32::from_be_bytes(*sample),
                    ByteOrder::Native => f32::from_ne_bytes(*sample),
                };
                if !value.is_finite() {
                    if settings.non_finite == NonFinitePolicy::Reject {
                        return Err(Error::NonFinite {
                            row: y,
                            sample: sample_index,
                        });
                    }
                    value = if value.is_nan() {
                        0.0
                    } else if value.is_sign_positive() {
                        f32::MAX
                    } else {
                        -f32::MAX
                    };
                    *non_finite_count = non_finite_count
                        .checked_add(1)
                        .ok_or(Error::ArithmeticOverflow)?;
                }
                output.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    Ok(())
}

fn validate_bytes(
    bytes: &[u8],
    format: Format,
    gray: bool,
    dimensions: ImageDimensions,
    payload_bytes: u64,
    depth: IntegerDepth,
) -> Result<(), Error> {
    let magic = match format {
        Format::Ppm => b"P6\n".as_slice(),
        Format::Pgm => b"P5\n".as_slice(),
        Format::Pfm if gray => b"Pf\n".as_slice(),
        Format::Pfm => b"PF\n".as_slice(),
    };
    if !bytes.starts_with(magic) {
        return Err(Error::Malformed("magic"));
    }
    let header_len = header(format, gray, dimensions, depth)?.len();
    let expected = u64::try_from(header_len)
        .map_err(|_| Error::ArithmeticOverflow)?
        .checked_add(payload_bytes)
        .ok_or(Error::ArithmeticOverflow)?;
    if u64::try_from(bytes.len()).map_err(|_| Error::ArithmeticOverflow)? != expected {
        return Err(Error::Malformed("payload length"));
    }
    if bytes[header_len..].is_empty() {
        return Err(Error::Malformed("empty payload"));
    }
    Ok(())
}

struct DigestWriter<'a, W> {
    writer: &'a mut W,
    hasher: Sha256,
    bytes: u64,
}

impl<'a, W> DigestWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }
    fn finish(self) -> (u64, [u8; 32]) {
        (self.bytes, self.hasher.finalize().into())
    }
}

impl<W: Write> Write for DigestWriter<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.writer.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.bytes = self
            .bytes
            .checked_add(written as u64)
            .ok_or_else(|| io::Error::other("portable-anymap byte count overflow"))?;
        Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
