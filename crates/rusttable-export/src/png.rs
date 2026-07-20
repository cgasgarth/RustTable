use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_image::{
    DecodeLimits, DecodeLimitsError, DecodedImage, ImageDimensions, ImageInput, ImageOutput,
    ImageOutputError, OutputLimits, OutputLimitsError, OutputOptions, OutputReceipt,
};
use rusttable_image_io::{FileImageInput, FileImageOutput};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Controls whether a PNG export may replace an existing destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollisionPolicy {
    /// Publish only if the destination does not already exist.
    CreateNew,
    /// Atomically replace the destination when the platform supports it.
    ReplaceExisting,
}

/// Bounds applied to every PNG export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngExportLimits {
    output: OutputLimits,
    decode: DecodeLimits,
}

impl PngExportLimits {
    /// Creates nonzero width, height, and encoded-byte bounds.
    ///
    /// The decoder bounds used for independent verification are derived from
    /// the dimension bounds and encoded-byte limit.
    ///
    /// # Errors
    ///
    /// Returns an error when any supplied bound is zero or overflows.
    pub fn new(
        max_width: u32,
        max_height: u32,
        max_encoded_bytes: u64,
    ) -> Result<Self, PngExportLimitsError> {
        let output =
            OutputLimits::new(max_encoded_bytes).map_err(PngExportLimitsError::OutputLimits)?;
        let max_pixel_count = u64::from(max_width)
            .checked_mul(u64::from(max_height))
            .ok_or(PngExportLimitsError::DimensionOverflow)?;
        let max_decoded_bytes = max_pixel_count
            .checked_mul(4)
            .ok_or(PngExportLimitsError::DimensionOverflow)?;
        let decode = DecodeLimits::new(
            max_encoded_bytes,
            max_width,
            max_height,
            max_pixel_count,
            max_decoded_bytes,
        )
        .map_err(PngExportLimitsError::DecodeLimits)?;
        Ok(Self { output, decode })
    }

    #[must_use]
    pub const fn max_width(self) -> u32 {
        self.decode.max_width()
    }

    #[must_use]
    pub const fn max_height(self) -> u32 {
        self.decode.max_height()
    }

    #[must_use]
    pub const fn max_encoded_bytes(self) -> u64 {
        self.output.max_encoded_bytes()
    }
}

/// Construction failures for [`PngExportLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngExportLimitsError {
    OutputLimits(OutputLimitsError),
    DecodeLimits(DecodeLimitsError),
    DimensionOverflow,
}

/// Receipt returned after a PNG was published and independently decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngExportReceipt {
    output: OutputReceipt,
    verified_dimensions: ImageDimensions,
}

impl PngExportReceipt {
    #[must_use]
    pub fn output(&self) -> &OutputReceipt {
        &self.output
    }

    #[must_use]
    pub fn destination(&self) -> &Path {
        self.output.destination()
    }

    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.output.dimensions()
    }

    #[must_use]
    pub const fn encoded_byte_length(&self) -> u64 {
        self.output.encoded_byte_length()
    }

    #[must_use]
    pub const fn verified_dimensions(&self) -> ImageDimensions {
        self.verified_dimensions
    }
}

/// Errors from bounded, verified PNG publication.
#[derive(Debug)]
pub enum PngPublishError {
    InvalidDestination {
        path: PathBuf,
    },
    DestinationExists {
        path: PathBuf,
    },
    DimensionLimit {
        actual: ImageDimensions,
        max_width: u32,
        max_height: u32,
    },
    Output(ImageOutputError),
    VerificationDecode(rusttable_image::ImageInputError),
    VerificationMismatch {
        expected: ImageDimensions,
        actual: ImageDimensions,
    },
    VerificationPixelsMismatch,
    Io {
        operation: &'static str,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for PngExportLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutputLimits(source) => write!(formatter, "invalid PNG output limit: {source}"),
            Self::DecodeLimits(source) => {
                write!(formatter, "invalid PNG verification limit: {source:?}")
            }
            Self::DimensionOverflow => formatter.write_str("PNG dimensions overflowed"),
        }
    }
}

impl std::error::Error for PngExportLimitsError {}

impl fmt::Display for PngPublishError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDestination { path } => {
                write!(formatter, "invalid PNG destination: {}", path.display())
            }
            Self::DestinationExists { path } => {
                write!(
                    formatter,
                    "PNG destination already exists: {}",
                    path.display()
                )
            }
            Self::DimensionLimit {
                actual,
                max_width,
                max_height,
            } => write!(
                formatter,
                "PNG dimensions {}x{} exceed {}x{}",
                actual.width(),
                actual.height(),
                max_width,
                max_height
            ),
            Self::Output(source) => write!(formatter, "PNG output failed: {source}"),
            Self::VerificationDecode(source) => {
                write!(formatter, "written PNG could not be decoded: {source}")
            }
            Self::VerificationMismatch { expected, actual } => write!(
                formatter,
                "written PNG dimensions are {}x{}, expected {}x{}",
                actual.width(),
                actual.height(),
                expected.width(),
                expected.height()
            ),
            Self::VerificationPixelsMismatch => {
                formatter.write_str("written PNG pixels differ from the rendered image")
            }
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "could not {operation} {}: {source}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PngPublishError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::VerificationDecode(source) => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::InvalidDestination { .. }
            | Self::DestinationExists { .. }
            | Self::DimensionLimit { .. }
            | Self::Output(_)
            | Self::VerificationMismatch { .. }
            | Self::VerificationPixelsMismatch => None,
        }
    }
}

/// Publishes rendered images as bounded, verified PNG files.
#[derive(Debug, Clone, Copy)]
pub struct PngPublisher {
    limits: PngExportLimits,
}

impl PngPublisher {
    #[must_use]
    pub const fn new(limits: PngExportLimits) -> Self {
        Self { limits }
    }

    #[must_use]
    pub const fn limits(self) -> PngExportLimits {
        self.limits
    }

    /// Encodes, independently decodes, and atomically publishes one PNG.
    ///
    /// Create-new publication never replaces a destination. Replace-existing
    /// publication stages and verifies the PNG before an atomic rename.
    ///
    /// # Errors
    ///
    /// Returns a typed error for invalid destinations, bounds, encoding,
    /// verification, collisions, or filesystem publication failures.
    pub fn publish(
        &self,
        image: &DecodedImage,
        destination: &Path,
        collision: CollisionPolicy,
    ) -> Result<PngExportReceipt, PngPublishError> {
        validate_destination(destination)?;
        self.validate_dimensions(image.dimensions())?;
        if collision == CollisionPolicy::CreateNew && destination.exists() {
            return Err(PngPublishError::DestinationExists {
                path: destination.to_owned(),
            });
        }

        let staging = staging_path(destination);
        let output = FileImageOutput::new(self.limits.output);
        let staged_receipt = match output.write_new(image, &staging, OutputOptions::Png) {
            Ok(receipt) => receipt,
            Err(error) => {
                remove_staging(&staging);
                return Err(PngPublishError::Output(error));
            }
        };
        let verification = self.verify(image, &staging);
        if let Err(error) = verification {
            remove_staging(&staging);
            return Err(error);
        }

        let publication = match collision {
            CollisionPolicy::CreateNew => publish_new(&staging, destination),
            CollisionPolicy::ReplaceExisting => publish_replace(&staging, destination),
        };
        if let Err(error) = publication {
            remove_staging(&staging);
            return Err(error);
        }

        self.verify(image, destination)?;
        let output = OutputReceipt::new(
            destination.to_owned(),
            staged_receipt.format(),
            staged_receipt.dimensions(),
            staged_receipt.encoded_byte_length(),
        )
        .map_err(|_| {
            PngPublishError::Output(ImageOutputError::EncodeFailure {
                format: staged_receipt.format(),
            })
        })?;
        Ok(PngExportReceipt {
            output,
            verified_dimensions: image.dimensions(),
        })
    }

    fn validate_dimensions(&self, dimensions: ImageDimensions) -> Result<(), PngPublishError> {
        if dimensions.width() > self.limits.max_width()
            || dimensions.height() > self.limits.max_height()
        {
            return Err(PngPublishError::DimensionLimit {
                actual: dimensions,
                max_width: self.limits.max_width(),
                max_height: self.limits.max_height(),
            });
        }
        Ok(())
    }

    fn verify(
        &self,
        expected: &DecodedImage,
        path: &Path,
    ) -> Result<ImageDimensions, PngPublishError> {
        let input = FileImageInput::new(self.limits.decode);
        let decoded = input
            .decode_path(path)
            .map_err(PngPublishError::VerificationDecode)?;
        if decoded.dimensions() != expected.dimensions() {
            return Err(PngPublishError::VerificationMismatch {
                expected: expected.dimensions(),
                actual: decoded.dimensions(),
            });
        }
        if decoded.pixels() != expected.pixels() {
            return Err(PngPublishError::VerificationPixelsMismatch);
        }
        Ok(decoded.dimensions())
    }
}

fn validate_destination(destination: &Path) -> Result<(), PngPublishError> {
    if destination.file_name().is_none() {
        return Err(PngPublishError::InvalidDestination {
            path: destination.to_owned(),
        });
    }
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(PngPublishError::InvalidDestination {
            path: destination.to_owned(),
        });
    }
    Ok(())
}

fn staging_path(destination: &Path) -> PathBuf {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let process = std::process::id();
    parent.join(format!(
        ".rusttable-export-{process:08x}-{sequence:016x}.png.tmp"
    ))
}

fn publish_new(staging: &Path, destination: &Path) -> Result<(), PngPublishError> {
    fs::hard_link(staging, destination).map_err(|source| {
        if source.kind() == io::ErrorKind::AlreadyExists {
            PngPublishError::DestinationExists {
                path: destination.to_owned(),
            }
        } else {
            PngPublishError::Io {
                operation: "publish PNG",
                path: destination.to_owned(),
                source,
            }
        }
    })?;
    remove_staging(staging);
    Ok(())
}

fn publish_replace(staging: &Path, destination: &Path) -> Result<(), PngPublishError> {
    fs::rename(staging, destination).map_err(|source| PngPublishError::Io {
        operation: "replace PNG",
        path: destination.to_owned(),
        source,
    })
}

fn remove_staging(staging: &Path) {
    let _ = fs::remove_file(staging);
}
