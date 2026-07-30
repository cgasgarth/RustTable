use std::fmt;
use std::fmt::Write;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use rusttable_image::{
    DecodeLimits, DecodeLimitsError, DecodedImage, ImageDimensions, ImageInput, ImageOutput,
    ImageOutputError, OutputLimits, OutputLimitsError, OutputOptions, OutputReceipt,
};
use rusttable_image_io::{FileImageInput, FileImageOutput};
use sha2::{Digest, Sha256};

use crate::{CanonicalArtifact, ExportMetadata, MetadataPacket, MetadataPolicy};

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Controls whether a PNG export may replace an existing destination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CollisionPolicy {
    /// Publish only if the destination does not already exist.
    CreateNew,
    /// Atomically replace the destination when the platform supports it.
    ReplaceExisting,
    /// Fail when the destination already exists.
    Fail,
    /// Reuse a destination whose `RustTable` commit manifest has the same artifact hash.
    SkipIfSame,
    /// Allocate the first deterministic `-01`, `-02`, ... suffix.
    UniqueSuffix,
    /// Use the queued revision as a deterministic version suffix.
    VersionRevision,
}

/// The result of publishing at the destination collision boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngCollisionResult {
    /// A new destination was created without replacing any existing file.
    CreatedNew,
    /// A complete, verified staging file atomically replaced the destination.
    ReplacedExisting,
}

/// A cancellation-aware PNG publication stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngPublishStage {
    /// Validating the request, destination, dimensions, and collision policy.
    Preparing,
    /// Encoding the rendered pixels into an owned staging file.
    Encoding,
    /// Independently decoding and hashing the owned staging file.
    Verifying,
    /// Crossing the irreversible create-new or replace publication boundary.
    Publishing,
    /// The final destination was verified and the receipt is ready.
    Completed,
}

/// A progress notification emitted by [`PngPublisher`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PngPublishProgress {
    stage: PngPublishStage,
}

impl PngPublishProgress {
    #[must_use]
    pub const fn stage(self) -> PngPublishStage {
        self.stage
    }
}

/// The action requested by a [`PngPublishObserver`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngPublishControl {
    /// Continue publication into the next stage.
    Continue,
    /// Stop before any irreversible publication work begins.
    Cancel,
}

/// Receives explicit PNG publication progress and may request cancellation.
pub trait PngPublishObserver {
    /// Observes the next publication stage.
    fn observe(&mut self, progress: PngPublishProgress) -> PngPublishControl;
}

impl<Observer> PngPublishObserver for Observer
where
    Observer: FnMut(PngPublishProgress) -> PngPublishControl,
{
    fn observe(&mut self, progress: PngPublishProgress) -> PngPublishControl {
        self(progress)
    }
}

/// How a publication completed when cancellation was requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngPublishCompletion {
    /// No cancellation was requested after final verification.
    Completed,
    /// Cancellation arrived after the irreversible publication boundary.
    CompletedAfterCancellation,
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

/// Cryptographic evidence from independently checking a published PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngVerificationReceipt {
    dimensions: ImageDimensions,
    artifact_sha256: String,
    pixel_sha256: String,
}

/// Explicit evidence of the immutable metadata policy applied to a PNG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngMetadataReceipt {
    policy_identity: String,
    packet_sha256: String,
    included_groups: Vec<String>,
    excluded_groups: Vec<String>,
    included_fields: Vec<String>,
}

impl PngMetadataReceipt {
    #[must_use]
    pub fn from_packet(packet: &MetadataPacket, policy: MetadataPolicy) -> Self {
        Self {
            policy_identity: policy.identity(),
            packet_sha256: hash_hex(&packet.canonical_hash()),
            included_groups: policy.included_groups(),
            excluded_groups: policy.excluded_groups(),
            included_fields: packet.property_names().into_iter().collect(),
        }
    }

    #[must_use]
    pub fn policy_identity(&self) -> &str {
        &self.policy_identity
    }

    #[must_use]
    pub fn packet_sha256(&self) -> &str {
        &self.packet_sha256
    }

    #[must_use]
    pub fn included_groups(&self) -> &[String] {
        &self.included_groups
    }

    #[must_use]
    pub fn excluded_groups(&self) -> &[String] {
        &self.excluded_groups
    }

    #[must_use]
    pub fn included_fields(&self) -> &[String] {
        &self.included_fields
    }
}

impl PngVerificationReceipt {
    #[must_use]
    pub const fn dimensions(&self) -> ImageDimensions {
        self.dimensions
    }

    #[must_use]
    pub fn artifact_sha256(&self) -> &str {
        &self.artifact_sha256
    }

    #[must_use]
    pub fn pixel_sha256(&self) -> &str {
        &self.pixel_sha256
    }
}

/// Receipt returned after a PNG was published and independently decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PngExportReceipt {
    output: OutputReceipt,
    verification: PngVerificationReceipt,
    metadata: Option<PngMetadataReceipt>,
    collision: PngCollisionResult,
    completion: PngPublishCompletion,
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
        self.verification.dimensions()
    }

    #[must_use]
    pub const fn verification(&self) -> &PngVerificationReceipt {
        &self.verification
    }

    #[must_use]
    pub fn metadata(&self) -> Option<&PngMetadataReceipt> {
        self.metadata.as_ref()
    }

    #[must_use]
    pub const fn collision(&self) -> PngCollisionResult {
        self.collision
    }

    #[must_use]
    pub const fn completion(&self) -> PngPublishCompletion {
        self.completion
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
    Cancelled {
        stage: PngPublishStage,
    },
    DimensionLimit {
        actual: ImageDimensions,
        max_width: u32,
        max_height: u32,
    },
    Output(ImageOutputError),
    Metadata(String),
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
            Self::Cancelled { stage } => {
                write!(formatter, "PNG publication cancelled during {stage:?}")
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
            Self::Metadata(source) => write!(formatter, "PNG metadata output failed: {source}"),
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
            | Self::Cancelled { .. }
            | Self::DimensionLimit { .. }
            | Self::Output(_)
            | Self::Metadata(_)
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
        self.publish_with_observer(image, destination, collision, |_: PngPublishProgress| {
            PngPublishControl::Continue
        })
    }

    /// Encodes, verifies, and publishes one PNG while reporting each boundary.
    ///
    /// A cancellation request through [`PngPublishObserver`] before
    /// [`PngPublishStage::Publishing`] removes the owned staging file and
    /// leaves the destination unchanged. Cancellation reported at
    /// [`PngPublishStage::Completed`] is recorded in the receipt because an
    /// atomic create or replacement cannot be rolled back safely.
    ///
    /// # Errors
    ///
    /// Returns [`PngPublishError::Cancelled`] before the publication boundary,
    /// or another typed publication error.
    pub fn publish_with_observer<Observer>(
        &self,
        image: &DecodedImage,
        destination: &Path,
        collision: CollisionPolicy,
        observer: Observer,
    ) -> Result<PngExportReceipt, PngPublishError>
    where
        Observer: PngPublishObserver,
    {
        self.publish_with_metadata_and_observer(image, destination, collision, None, None, observer)
    }

    /// Encodes and publishes one PNG with a pre-resolved immutable metadata packet.
    ///
    /// Metadata is encoded while the owned staging file is still private. Any
    /// policy or serialization failure therefore leaves the destination unchanged
    /// and is returned explicitly to the caller.
    ///
    /// # Errors
    ///
    /// Returns a typed error for invalid destinations, bounds, metadata or pixel
    /// encoding, verification, collisions, cancellation, or filesystem failures.
    pub fn publish_with_metadata(
        &self,
        image: &DecodedImage,
        destination: &Path,
        collision: CollisionPolicy,
        metadata: ExportMetadata,
        metadata_receipt: PngMetadataReceipt,
    ) -> Result<PngExportReceipt, PngPublishError> {
        self.publish_with_metadata_and_observer(
            image,
            destination,
            collision,
            Some(metadata),
            Some(metadata_receipt),
            |_: PngPublishProgress| PngPublishControl::Continue,
        )
    }

    /// Metadata-aware PNG publication with explicit progress and cancellation.
    ///
    /// # Errors
    ///
    /// Returns a typed error for invalid destinations, bounds, metadata or pixel
    /// encoding, verification, collisions, cancellation, or filesystem failures.
    pub fn publish_with_metadata_and_observer<Observer>(
        &self,
        image: &DecodedImage,
        destination: &Path,
        collision: CollisionPolicy,
        metadata: Option<ExportMetadata>,
        metadata_receipt: Option<PngMetadataReceipt>,
        mut observer: Observer,
    ) -> Result<PngExportReceipt, PngPublishError>
    where
        Observer: PngPublishObserver,
    {
        observe(&mut observer, PngPublishStage::Preparing)?;
        validate_destination(destination)?;
        self.validate_dimensions(image.dimensions())?;
        if matches!(
            collision,
            CollisionPolicy::CreateNew | CollisionPolicy::Fail
        ) && destination.exists()
        {
            return Err(PngPublishError::DestinationExists {
                path: destination.to_owned(),
            });
        }

        let staging = staging_path(destination);
        observe(&mut observer, PngPublishStage::Encoding)?;
        let staged_receipt_result = self.encode_staging(image, &staging, metadata);
        let staged_receipt = match staged_receipt_result {
            Ok(receipt) => receipt,
            Err(error) => {
                remove_staging(&staging);
                return Err(error);
            }
        };
        let verification = observe(&mut observer, PngPublishStage::Verifying)
            .and_then(|()| self.verify(image, &staging));
        if let Err(error) = verification {
            remove_staging(&staging);
            return Err(error);
        }

        let publication =
            observe(&mut observer, PngPublishStage::Publishing).and_then(|()| match collision {
                CollisionPolicy::CreateNew | CollisionPolicy::Fail => {
                    publish_new(&staging, destination)
                }
                CollisionPolicy::ReplaceExisting => publish_replace(&staging, destination),
                CollisionPolicy::SkipIfSame
                | CollisionPolicy::UniqueSuffix
                | CollisionPolicy::VersionRevision => Err(PngPublishError::DestinationExists {
                    path: destination.to_owned(),
                }),
            });
        if let Err(error) = publication {
            remove_staging(&staging);
            return Err(error);
        }

        let verification = self.verify(image, destination)?;
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
            verification,
            metadata: metadata_receipt,
            collision: match collision {
                CollisionPolicy::CreateNew | CollisionPolicy::Fail => {
                    PngCollisionResult::CreatedNew
                }
                CollisionPolicy::ReplaceExisting => PngCollisionResult::ReplacedExisting,
                CollisionPolicy::SkipIfSame
                | CollisionPolicy::UniqueSuffix
                | CollisionPolicy::VersionRevision => PngCollisionResult::CreatedNew,
            },
            completion: match observer.observe(PngPublishProgress {
                stage: PngPublishStage::Completed,
            }) {
                PngPublishControl::Continue => PngPublishCompletion::Completed,
                PngPublishControl::Cancel => PngPublishCompletion::CompletedAfterCancellation,
            },
        })
    }

    fn encode_staging(
        &self,
        image: &DecodedImage,
        staging: &Path,
        metadata: Option<ExportMetadata>,
    ) -> Result<OutputReceipt, PngPublishError> {
        if let Some(metadata) = metadata {
            let artifact = CanonicalArtifact::new(image.as_owned(), metadata);
            let settings = crate::encoders::png::Settings {
                bit_depth: crate::encoders::png::BitDepth::Eight,
                channels: rusttable_image::ChannelLayout::Rgba,
                compression: crate::encoders::png::Compression::Balanced,
                filter: crate::encoders::png::Filter::Adaptive,
                interlace: false,
                metadata: crate::encoders::png::MetadataPolicy::All,
                max_metadata_bytes: 16 * 1024 * 1024,
            };
            let receipt = crate::encoders::png::Encoder::new(settings)
                .encode_to_path(&artifact, staging)
                .map_err(|error| PngPublishError::Metadata(error.to_string()))?;
            OutputReceipt::new(
                staging.to_owned(),
                rusttable_image::OutputFormat::Png,
                receipt.dimensions,
                receipt.encoded_bytes,
            )
            .map_err(|_| {
                PngPublishError::Output(ImageOutputError::EncodeFailure {
                    format: rusttable_image::OutputFormat::Png,
                })
            })
        } else {
            let output = FileImageOutput::new(self.limits.output);
            output
                .write_new(image, staging, OutputOptions::Png)
                .map_err(PngPublishError::Output)
        }
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
    ) -> Result<PngVerificationReceipt, PngPublishError> {
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
        let artifact = fs::read(path).map_err(|source| PngPublishError::Io {
            operation: "hash verified PNG",
            path: path.to_owned(),
            source,
        })?;
        Ok(PngVerificationReceipt {
            dimensions: decoded.dimensions(),
            artifact_sha256: hash_hex(&artifact),
            pixel_sha256: hash_hex(decoded.pixels()),
        })
    }
}

fn observe<Observer>(observer: &mut Observer, stage: PngPublishStage) -> Result<(), PngPublishError>
where
    Observer: PngPublishObserver,
{
    match observer.observe(PngPublishProgress { stage }) {
        PngPublishControl::Continue => Ok(()),
        PngPublishControl::Cancel => Err(PngPublishError::Cancelled { stage }),
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

fn hash_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}
