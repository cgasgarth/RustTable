//! Queue-safe, byte-exact copies of immutable imported source objects.
#![allow(clippy::format_collect)]
#![allow(clippy::missing_errors_doc)]

use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use rusttable_core::RenderSizeRequest;
use rusttable_import::{SourceSnapshot, SourceSnapshotReadError};
use sha2::{Digest, Sha256};

use crate::{AlphaPolicy, DitherPolicy, ExportRequest, MetadataAction, MetadataPolicy};

pub const ENCODER_ID_COPY: &str = "copy";
pub const SETTINGS_SCHEMA_VERSION: u16 = 1;
const DEFAULT_CHUNK_BYTES: usize = 128 * 1024;
const DEFAULT_MAX_OUTPUT_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_SIDECAR_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    expected_sha256: [u8; 32],
    byte_length: u64,
    extension: String,
}

impl SourceDescriptor {
    #[must_use]
    pub fn new(expected_sha256: [u8; 32], byte_length: u64, extension: impl Into<String>) -> Self {
        Self {
            expected_sha256,
            byte_length,
            extension: extension.into(),
        }
    }

    /// Captures only non-private source identity evidence needed by a queued job.
    pub fn from_snapshot(snapshot: &SourceSnapshot) -> Result<Self, Error> {
        let extension = snapshot
            .path()
            .extension()
            .and_then(|value| value.to_str())
            .map(str::to_ascii_lowercase)
            .filter(|value| {
                !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            .ok_or(Error::MissingExtension)?;
        Ok(Self {
            expected_sha256: snapshot.content_sha256(),
            byte_length: snapshot.byte_length().get(),
            extension,
        })
    }

    #[must_use]
    pub const fn expected_sha256(&self) -> [u8; 32] {
        self.expected_sha256
    }
    #[must_use]
    pub const fn byte_length(&self) -> u64 {
        self.byte_length
    }
    #[must_use]
    pub fn extension(&self) -> &str {
        &self.extension
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidecarSettings {
    edit_revision: u64,
    request_hash: [u8; 32],
    history: Vec<u8>,
}

impl SidecarSettings {
    #[must_use]
    pub const fn new(edit_revision: u64, request_hash: [u8; 32]) -> Self {
        Self {
            edit_revision,
            request_hash,
            history: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_history(mut self, history: impl Into<Vec<u8>>) -> Self {
        self.history = history.into();
        self
    }

    #[must_use]
    pub const fn edit_revision(&self) -> u64 {
        self.edit_revision
    }
    #[must_use]
    pub const fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }

    #[must_use]
    pub fn history(&self) -> &[u8] {
        &self.history
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    max_output_bytes: u64,
    chunk_bytes: usize,
    output_extension: Option<String>,
    sidecar: Option<SidecarSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
            output_extension: None,
            sidecar: None,
        }
    }
}

impl Settings {
    #[must_use]
    pub fn with_max_output_bytes(mut self, value: u64) -> Self {
        self.max_output_bytes = value;
        self
    }
    #[must_use]
    pub fn with_chunk_bytes(mut self, value: usize) -> Self {
        self.chunk_bytes = value;
        self
    }
    #[must_use]
    pub fn with_output_extension(mut self, value: impl Into<String>) -> Self {
        self.output_extension = Some(value.into());
        self
    }
    #[must_use]
    pub fn with_sidecar(mut self, value: SidecarSettings) -> Self {
        self.sidecar = Some(value);
        self
    }
    #[must_use]
    pub const fn max_output_bytes(&self) -> u64 {
        self.max_output_bytes
    }
    #[must_use]
    pub const fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }
    #[must_use]
    pub fn output_extension(&self) -> Option<&str> {
        self.output_extension.as_deref()
    }
    #[must_use]
    pub const fn sidecar(&self) -> Option<&SidecarSettings> {
        self.sidecar.as_ref()
    }

    /// Validates queue limits before any source or staging file is opened.
    pub fn validate(&self) -> Result<(), Error> {
        if self.max_output_bytes == 0 {
            return Err(Error::InvalidSettings(
                "maximum output bytes must be nonzero",
            ));
        }
        if !(1024..=8 * 1024 * 1024).contains(&self.chunk_bytes) {
            return Err(Error::InvalidSettings(
                "copy chunk size must be between 1 KiB and 8 MiB",
            ));
        }
        if let Some(extension) = &self.output_extension {
            validate_extension(extension)?;
        }
        if self
            .sidecar
            .as_ref()
            .is_some_and(|sidecar| sidecar.history.len() > MAX_SIDECAR_BYTES / 2)
        {
            return Err(Error::SidecarTooLarge);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyManifest {
    bytes: Vec<u8>,
}

impl CopyManifest {
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Receipt {
    pub primary_bytes: u64,
    pub primary_sha256: [u8; 32],
    pub sidecar_bytes: Option<u64>,
    pub sidecar_sha256: Option<[u8; 32]>,
    pub original_extension: String,
    pub manifest: CopyManifest,
}

#[derive(Debug)]
pub enum Error {
    InvalidSettings(&'static str),
    MissingExtension,
    IncompatibleRequest(&'static str),
    SourceIdentityMismatch,
    SourceRead(SourceSnapshotReadError),
    Io {
        stage: &'static str,
        source: io::Error,
    },
    Cancelled,
    OutputTooLarge {
        limit: u64,
        actual: u64,
    },
    SidecarTooLarge,
    InvalidExtension,
    StagingExists(PathBuf),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSettings(message) => write!(formatter, "invalid copy settings: {message}"),
            Self::MissingExtension => {
                formatter.write_str("copy source has no normalized extension")
            }
            Self::IncompatibleRequest(field) => write!(
                formatter,
                "copy encoder rejects incompatible request field: {field}"
            ),
            Self::SourceIdentityMismatch => {
                formatter.write_str("copy source identity does not match the queued snapshot")
            }
            Self::SourceRead(error) => write!(formatter, "copy source read failed: {error}"),
            Self::Io { stage, source } => write!(formatter, "copy {stage} failed: {source}"),
            Self::Cancelled => formatter.write_str("copy encoder cancelled"),
            Self::OutputTooLarge { limit, actual } => {
                write!(formatter, "copy output is {actual} bytes, limit is {limit}")
            }
            Self::SidecarTooLarge => formatter.write_str("copy sidecar exceeds its size limit"),
            Self::InvalidExtension => {
                formatter.write_str("copy output extension is invalid or changes the source format")
            }
            Self::StagingExists(path) => write!(
                formatter,
                "copy staging path already exists: {}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, Default)]
pub struct Encoder {
    settings: Settings,
}

impl Encoder {
    #[must_use]
    pub const fn new(settings: Settings) -> Self {
        Self { settings }
    }
    #[must_use]
    pub const fn settings(&self) -> &Settings {
        &self.settings
    }

    /// Rejects all render, pixel, and metadata mutations before queue execution.
    pub fn validate_request(&self, request: &ExportRequest) -> Result<(), Error> {
        request
            .validate()
            .map_err(|_| Error::IncompatibleRequest("request contract"))?;
        if request.size() != RenderSizeRequest::Source {
            return Err(Error::IncompatibleRequest("size"));
        }
        if request.alpha_policy() != AlphaPolicy::Preserve {
            return Err(Error::IncompatibleRequest("alpha"));
        }
        if request.dither_policy() != DitherPolicy::None {
            return Err(Error::IncompatibleRequest("dither"));
        }
        if !metadata_is_unchanged(request.metadata_policy()) {
            return Err(Error::IncompatibleRequest("metadata policy"));
        }
        self.settings.validate()
    }

    /// Streams the source through a bounded reusable buffer and verifies exact identity.
    pub fn encode_to_writer<W, C, P>(
        &self,
        snapshot: &SourceSnapshot,
        source: &SourceDescriptor,
        writer: &mut W,
        mut cancelled: C,
        mut progress: P,
    ) -> Result<(u64, [u8; 32]), Error>
    where
        W: Write,
        C: FnMut() -> bool,
        P: FnMut(u64, u64),
    {
        self.settings.validate()?;
        verify_source(snapshot, source)?;
        if source.byte_length > self.settings.max_output_bytes {
            return Err(Error::OutputTooLarge {
                limit: self.settings.max_output_bytes,
                actual: source.byte_length,
            });
        }
        let mut reader = snapshot
            .open_reader(source.byte_length)
            .map_err(Error::SourceRead)?;
        let mut buffer = vec![0_u8; self.settings.chunk_bytes];
        let mut hasher = Sha256::new();
        let mut copied = 0_u64;
        while copied < source.byte_length {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let amount = reader
                .read_checked(&mut buffer)
                .map_err(Error::SourceRead)?;
            if amount == 0 {
                return Err(Error::SourceRead(SourceSnapshotReadError::SourceChanged {
                    path: snapshot.path().to_owned(),
                }));
            }
            writer
                .write_all(&buffer[..amount])
                .map_err(|source| Error::Io {
                    stage: "write",
                    source,
                })?;
            hasher.update(&buffer[..amount]);
            copied = copied.saturating_add(u64::try_from(amount).unwrap_or(u64::MAX));
            progress(copied, source.byte_length);
        }
        snapshot
            .revalidate_opened_source()
            .map_err(|error| Error::SourceRead(error.into()))?;
        let hash: [u8; 32] = hasher.finalize().into();
        if copied != source.byte_length || hash != source.expected_sha256 {
            return Err(Error::SourceIdentityMismatch);
        }
        Ok((copied, hash))
    }

    /// Encodes primary and optional sidecar outputs into pre-created queue staging paths.
    pub fn encode_to_paths<C, P>(
        &self,
        snapshot: &SourceSnapshot,
        source: &SourceDescriptor,
        primary: &Path,
        sidecar: Option<&Path>,
        cancelled: C,
        progress: P,
    ) -> Result<Receipt, Error>
    where
        C: FnMut() -> bool,
        P: FnMut(u64, u64),
    {
        self.settings.validate()?;
        let result = (|| {
            ensure_new_path(primary)?;
            if let Some(sidecar) = sidecar {
                ensure_new_path(sidecar)?;
            }
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(primary)
                .map_err(|source| Error::Io {
                    stage: "staging create",
                    source,
                })?;
            let mut writer = BufWriter::new(file);
            let (primary_bytes, primary_sha256) =
                self.encode_to_writer(snapshot, source, &mut writer, cancelled, progress)?;
            let file = writer.into_inner().map_err(|error| Error::Io {
                stage: "staging flush",
                source: error.into_error(),
            })?;
            file.sync_all().map_err(|source| Error::Io {
                stage: "staging fsync",
                source,
            })?;
            let (sidecar_bytes, sidecar_sha256) = if let Some(path) = sidecar {
                let bytes = sidecar_bytes(self.settings.sidecar.as_ref().ok_or(
                    Error::InvalidSettings("sidecar path supplied without sidecar settings"),
                )?);
                if bytes.len() > MAX_SIDECAR_BYTES {
                    return Err(Error::SidecarTooLarge);
                }
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(path)
                    .map_err(|source| Error::Io {
                        stage: "sidecar create",
                        source,
                    })?;
                file.write_all(&bytes).map_err(|source| Error::Io {
                    stage: "sidecar write",
                    source,
                })?;
                file.sync_all().map_err(|source| Error::Io {
                    stage: "sidecar fsync",
                    source,
                })?;
                let hash: [u8; 32] = Sha256::digest(&bytes).into();
                (
                    Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)),
                    Some(hash),
                )
            } else if self.settings.sidecar.is_some() {
                return Err(Error::InvalidSettings(
                    "sidecar settings require a sidecar staging path",
                ));
            } else {
                (None, None)
            };
            let extension = self.output_extension(source)?;
            let manifest = manifest_bytes(
                source,
                primary_bytes,
                primary_sha256,
                sidecar_bytes,
                sidecar_sha256,
                &extension,
            );
            Ok(Receipt {
                primary_bytes,
                primary_sha256,
                sidecar_bytes,
                sidecar_sha256,
                original_extension: extension,
                manifest: CopyManifest { bytes: manifest },
            })
        })();
        if result.is_err() {
            remove_if_exists(primary);
            if let Some(path) = sidecar {
                remove_if_exists(path);
            }
        }
        result
    }

    fn output_extension(&self, source: &SourceDescriptor) -> Result<String, Error> {
        if let Some(extension) = &self.settings.output_extension {
            validate_extension(extension)?;
            if !extension.eq_ignore_ascii_case(&source.extension) {
                return Err(Error::InvalidExtension);
            }
            Ok(extension.to_ascii_lowercase())
        } else {
            Ok(source.extension.clone())
        }
    }
}

fn metadata_is_unchanged(policy: MetadataPolicy) -> bool {
    [
        policy.exif,
        policy.iptc,
        policy.xmp,
        policy.gps,
        policy.faces_and_regions,
        policy.ratings_labels_tags,
        policy.history,
        policy.thumbnail,
        policy.icc_and_cicp,
        policy.software_and_version,
        policy.user_fields,
    ]
    .into_iter()
    .all(|action| action == MetadataAction::Include)
}

fn verify_source(snapshot: &SourceSnapshot, source: &SourceDescriptor) -> Result<(), Error> {
    if snapshot.byte_length().get() != source.byte_length
        || snapshot.content_sha256() != source.expected_sha256
    {
        return Err(Error::SourceIdentityMismatch);
    }
    Ok(())
}

fn validate_extension(extension: &str) -> Result<(), Error> {
    if extension.is_empty()
        || extension.len() > 16
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::InvalidExtension);
    }
    Ok(())
}

fn sidecar_bytes(settings: &SidecarSettings) -> Vec<u8> {
    let mut output = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" xmlns:rusttable=\"https://rusttable.dev/ns/1.0/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rusttable:editRevision=\"{}\" rusttable:requestHash=\"{}\" rusttable:historySha256=\"{}\" rusttable:historyHex=\"{}\"/></rdf:RDF></x:xmpmeta>\n",
        settings.edit_revision,
        hex(&settings.request_hash),
        hex(&Sha256::digest(&settings.history).into()),
        hex_bytes(&settings.history)
    );
    output.shrink_to_fit();
    output.into_bytes()
}

fn manifest_bytes(
    source: &SourceDescriptor,
    primary_bytes: u64,
    primary_hash: [u8; 32],
    sidecar_bytes: Option<u64>,
    sidecar_hash: Option<[u8; 32]>,
    extension: &str,
) -> Vec<u8> {
    format!("schema=rusttable.copy-manifest.v1\nsource_sha256={}\nsource_bytes={}\noriginal_extension={}\nprimary_sha256={}\nprimary_bytes={}\nsidecar_sha256={}\nsidecar_bytes={}\n", hex(&source.expected_sha256), source.byte_length, extension, hex(&primary_hash), primary_bytes, sidecar_hash.map_or_else(|| "none".to_owned(), |hash| hex(&hash)), sidecar_bytes.map_or_else(|| "none".to_owned(), |bytes| bytes.to_string())).into_bytes()
}

fn ensure_new_path(path: &Path) -> Result<(), Error> {
    if path.exists() {
        Err(Error::StagingExists(path.to_owned()))
    } else {
        Ok(())
    }
}

fn remove_if_exists(path: &Path) {
    let _ = fs::remove_file(path);
}

fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}
