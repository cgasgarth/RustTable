//! Durable export orchestration for copy jobs.
#![allow(clippy::chunks_exact_to_as_chunks)]
#![allow(clippy::collapsible_if)]
#![allow(clippy::double_must_use)]
#![allow(clippy::format_collect)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unnested_or_patterns)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rusttable_catalog_store::{
    ExportJobId, ExportJobPriority, ExportJobRecord, ExportJobStage, ExportJobState,
    ExportQueueError, RedbExportQueueStore, queue_now_millis,
};
use rusttable_import::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceSnapshot, SourceSnapshotReadError,
    SourceSnapshotReader,
};
use sha2::{Digest, Sha256};

use crate::copy::{
    self, Encoder as CopyEncoder, Receipt as CopyReceipt, Settings as CopySettings,
    SidecarSettings, SourceDescriptor,
};
use crate::{ExportPriority, ExportRequest};

const COPY_SNAPSHOT_SCHEMA: &str = "rusttable.copy-queue-snapshot.v1";
const MAX_SOURCE_ID_BYTES: usize = 256;
const MAX_TARGET_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyQueueRequest {
    pub request_snapshot: Vec<u8>,
    pub request_hash: [u8; 32],
    pub source_id: String,
    pub source: SourceDescriptor,
    pub destination_id: String,
    pub destination_target: String,
    pub priority: ExportJobPriority,
    pub sidecar: Option<SidecarSettings>,
}

impl CopyQueueRequest {
    /// Takes the exact canonical request bytes at enqueue time; later edits cannot change this job.
    pub fn from_request(
        request: &ExportRequest,
        source_id: impl Into<String>,
        source: SourceDescriptor,
        destination_target: impl Into<String>,
        sidecar: Option<SidecarSettings>,
    ) -> Result<Self, QueueError> {
        CopyEncoder::new(CopySettings::default()).validate_request(request)?;
        let request_snapshot = request
            .canonical_bytes()
            .map_err(QueueError::RequestEncoding)?;
        let request_hash = request
            .request_hash()
            .map_err(QueueError::RequestEncoding)?;
        if sidecar
            .as_ref()
            .is_some_and(|value| value.request_hash() != request_hash)
        {
            return Err(QueueError::Copy(copy::Error::IncompatibleRequest(
                "sidecar request hash",
            )));
        }
        let source_id = source_id.into();
        let destination_target = destination_target.into();
        validate_identifier(&source_id, MAX_SOURCE_ID_BYTES, "source ID")?;
        validate_target(&destination_target)?;
        Ok(Self {
            request_snapshot,
            request_hash,
            source_id,
            source,
            destination_id: request_destination_id(request),
            destination_target,
            priority: request.priority().into(),
            sidecar,
        })
    }

    #[must_use]
    pub fn idempotency_key(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.request_hash);
        hasher.update(self.source.expected_sha256());
        hasher.update(self.source.byte_length().to_be_bytes());
        hasher.update(self.destination_id.as_bytes());
        hasher.update([0]);
        hasher.update(self.destination_target.as_bytes());
        format!("copy:{}", hex(&hasher.finalize().into()))
    }

    fn snapshot_bytes(&self) -> Vec<u8> {
        let sidecar = self.sidecar.as_ref().map_or_else(
            || "none".to_owned(),
            |value| format!("{}:{}", value.edit_revision(), hex(&value.request_hash())),
        );
        format!("schema={COPY_SNAPSHOT_SCHEMA}\nsource_id={}\nsource_sha256={}\nsource_bytes={}\nsource_extension={}\nrequest_hash={}\nsidecar={}\nsidecar_history_sha256={}\n", self.source_id, hex(&self.source.expected_sha256()), self.source.byte_length(), self.source.extension(), hex(&self.request_hash), sidecar, self.sidecar.as_ref().map_or_else(|| "none".to_owned(), |value| hex(&Sha256::digest(value.history()).into()))).into_bytes()
    }
}

impl From<ExportPriority> for ExportJobPriority {
    fn from(priority: ExportPriority) -> Self {
        match priority {
            ExportPriority::Background => Self::Background,
            ExportPriority::Normal => Self::Normal,
            ExportPriority::Interactive => Self::Interactive,
        }
    }
}

pub trait CopySourceProvider {
    fn open_snapshot(&self, source_id: &str) -> Result<SourceSnapshot, SourceProviderError>;
}

#[derive(Debug, Clone)]
pub struct FileCopySourceProvider {
    limits: ImportSourceLimits,
    sources: BTreeMap<String, PathBuf>,
}

impl FileCopySourceProvider {
    pub fn new(max_source_bytes: u64) -> Result<Self, QueueError> {
        Ok(Self {
            limits: ImportSourceLimits::new(max_source_bytes)
                .map_err(|_| QueueError::SourceProvider)?,
            sources: BTreeMap::new(),
        })
    }

    pub fn register(
        &mut self,
        source_id: impl Into<String>,
        path: impl Into<PathBuf>,
    ) -> Result<(), QueueError> {
        let source_id = source_id.into();
        validate_identifier(&source_id, MAX_SOURCE_ID_BYTES, "source ID")?;
        self.sources.insert(source_id, path.into());
        Ok(())
    }
}

impl CopySourceProvider for FileCopySourceProvider {
    fn open_snapshot(&self, source_id: &str) -> Result<SourceSnapshot, SourceProviderError> {
        let path = self
            .sources
            .get(source_id)
            .ok_or(SourceProviderError::Permanent)?;
        FileSourceSnapshotReader
            .read_snapshot(path, self.limits)
            .map_err(|_| SourceProviderError::Unavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceProviderError {
    Unavailable,
    Changed,
    Permanent,
}

impl fmt::Display for SourceProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "copy source provider failed: {self:?}")
    }
}
impl std::error::Error for SourceProviderError {}

pub trait CopyDestination {
    fn commit(
        &self,
        staging: &Path,
        logical_target: &str,
        idempotency_key: &str,
    ) -> Result<Vec<u8>, DestinationError>;
}

#[derive(Debug, Clone)]
pub struct LocalBundleDestination {
    root: PathBuf,
}

impl LocalBundleDestination {
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, DestinationError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|source| DestinationError::Io {
            stage: "destination create",
            source,
        })?;
        Ok(Self { root })
    }
}

impl CopyDestination for LocalBundleDestination {
    fn commit(
        &self,
        staging: &Path,
        logical_target: &str,
        idempotency_key: &str,
    ) -> Result<Vec<u8>, DestinationError> {
        validate_target(logical_target).map_err(|_| DestinationError::InvalidTarget)?;
        let destination = self.root.join(logical_target);
        if destination.exists() {
            let manifest = destination.join(".rusttable-export-commit");
            let mut bytes = Vec::new();
            File::open(&manifest)
                .and_then(|mut file| file.read_to_end(&mut bytes))
                .map_err(|_| DestinationError::CommitConflict)?;
            let text = String::from_utf8(bytes).map_err(|_| DestinationError::CommitConflict)?;
            if text
                .lines()
                .any(|line| line == format!("idempotency={idempotency_key}"))
            {
                return Ok(text.into_bytes());
            }
            return Err(DestinationError::CommitConflict);
        }
        let commit_manifest = staging.join(".rusttable-export-commit");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&commit_manifest)
            .map_err(|source| DestinationError::Io {
                stage: "commit manifest",
                source,
            })?;
        let bytes = format!("schema=rusttable.export-commit.v1\nidempotency={idempotency_key}\n")
            .into_bytes();
        file.write_all(&bytes)
            .map_err(|source| DestinationError::Io {
                stage: "commit manifest write",
                source,
            })?;
        file.sync_all().map_err(|source| DestinationError::Io {
            stage: "commit manifest fsync",
            source,
        })?;
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|source| DestinationError::Io {
                stage: "destination parent",
                source,
            })?;
        }
        fs::rename(staging, &destination).map_err(|source| DestinationError::Io {
            stage: "atomic bundle rename",
            source,
        })?;
        if let Some(parent) = destination.parent() {
            File::open(parent)
                .and_then(|file| file.sync_all())
                .map_err(|source| DestinationError::Io {
                    stage: "destination directory fsync",
                    source,
                })?;
        }
        Ok(bytes)
    }
}

#[derive(Debug)]
pub enum DestinationError {
    InvalidTarget,
    CommitConflict,
    Io {
        stage: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for DestinationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "copy destination failed: {self:?}")
    }
}
impl std::error::Error for DestinationError {}

#[derive(Debug)]
pub enum QueueError {
    Store(ExportQueueError),
    InvalidRequest(crate::ExportValidationError),
    RequestEncoding(crate::ExportContractError),
    InvalidIdentifier(&'static str),
    InvalidTarget,
    SourceProvider,
    Source(SourceProviderError),
    Copy(copy::Error),
    Destination(DestinationError),
    Staging(io::Error),
    Cancelled,
}

impl fmt::Display for QueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "export queue failed: {self:?}")
    }
}
impl std::error::Error for QueueError {}
impl From<ExportQueueError> for QueueError {
    fn from(error: ExportQueueError) -> Self {
        Self::Store(error)
    }
}
impl From<copy::Error> for QueueError {
    fn from(error: copy::Error) -> Self {
        Self::Copy(error)
    }
}

pub struct ExportQueue {
    store: RedbExportQueueStore,
    staging_root: PathBuf,
}

impl ExportQueue {
    pub fn open(database: &Path, staging_root: impl Into<PathBuf>) -> Result<Self, QueueError> {
        let staging_root = staging_root.into();
        fs::create_dir_all(&staging_root).map_err(QueueError::Staging)?;
        let store = RedbExportQueueStore::open(database)?;
        store.recover_in_process(queue_now_millis())?;
        Ok(Self {
            store,
            staging_root,
        })
    }

    pub fn enqueue_copy(&self, request: CopyQueueRequest) -> Result<ExportJobRecord, QueueError> {
        let id = ExportJobId::new(next_job_id(&self.store)?)
            .ok_or(QueueError::Store(ExportQueueError::Corrupt))?;
        let idempotency_key = request.idempotency_key();
        let snapshot = request.snapshot_bytes();
        let job = ExportJobRecord::new(
            id,
            request.request_hash,
            idempotency_key,
            request.destination_id,
            request.destination_target,
            snapshot,
            request.priority,
            queue_now_millis(),
        );
        Ok(self.store.enqueue(job)?)
    }

    #[must_use]
    pub fn get(&self, id: ExportJobId) -> Result<Option<ExportJobRecord>, QueueError> {
        Ok(self.store.get(id)?)
    }

    pub fn cancel(&self, id: ExportJobId) -> Result<ExportJobRecord, QueueError> {
        Ok(self.store.cancel(id, queue_now_millis())?)
    }
    pub fn retry(&self, id: ExportJobId) -> Result<ExportJobRecord, QueueError> {
        Ok(self.store.retry(id, queue_now_millis())?)
    }
    pub fn recover(&self) -> Result<Vec<ExportJobRecord>, QueueError> {
        Ok(self.store.recover_in_process(queue_now_millis())?)
    }

    /// Runs one immutable-copy job through durable state, staging, and an atomic bundle commit.
    pub fn execute_copy<P, D>(
        &self,
        id: ExportJobId,
        provider: &P,
        destination: &D,
    ) -> Result<CopyReceipt, QueueError>
    where
        P: CopySourceProvider,
        D: CopyDestination,
    {
        let result = self.execute_copy_inner(id, provider, destination);
        if let Err(error) = &result {
            if !self.is_cancelled(id) {
                let retryable = is_retryable(error);
                let attempt = self
                    .store
                    .get(id)
                    .ok()
                    .flatten()
                    .map_or(0, |job| job.attempt());
                let can_retry = retryable && attempt < 3;
                let retry_at =
                    can_retry.then(|| queue_now_millis().saturating_add(retry_delay(id, attempt)));
                let _ = self.store.fail(
                    id,
                    can_retry,
                    error.to_string(),
                    retry_at,
                    queue_now_millis(),
                );
            }
        }
        result
    }

    fn execute_copy_inner<P, D>(
        &self,
        id: ExportJobId,
        provider: &P,
        destination: &D,
    ) -> Result<CopyReceipt, QueueError>
    where
        P: CopySourceProvider,
        D: CopyDestination,
    {
        let job = self
            .store
            .get(id)?
            .ok_or(QueueError::Store(ExportQueueError::UnknownJob(id)))?;
        if job.state() == ExportJobState::Cancelled {
            return Err(QueueError::Cancelled);
        }
        if job.state() != ExportJobState::Queued {
            return Err(QueueError::Store(ExportQueueError::InvalidTransition {
                from: job.state(),
                to: ExportJobState::Preparing,
            }));
        }
        self.store
            .transition(id, ExportJobState::Preparing, queue_now_millis())?;
        let spec = parse_copy_snapshot(job.snapshot())?;
        let snapshot = provider
            .open_snapshot(&spec.source_id)
            .map_err(QueueError::Source)?;
        let settings = if let Some(sidecar) = &spec.sidecar {
            CopySettings::default().with_sidecar(sidecar.clone())
        } else {
            CopySettings::default()
        };
        let encoder = CopyEncoder::new(settings);
        let staging = self.staging_root.join(id.to_string());
        if staging.exists() {
            fs::remove_dir_all(&staging).map_err(QueueError::Staging)?;
        }
        fs::create_dir(&staging).map_err(QueueError::Staging)?;
        let primary = staging.join(format!("primary.{}", spec.source.extension()));
        let sidecar_path = spec.sidecar.as_ref().map(|_| staging.join("primary.xmp"));
        self.store
            .transition(id, ExportJobState::Encoding, queue_now_millis())?;
        let receipt = encoder.encode_to_paths(
            &snapshot,
            &spec.source,
            &primary,
            sidecar_path.as_deref(),
            || self.is_cancelled(id),
            |done, total| {
                let _ = self.store.update_progress(
                    id,
                    ExportJobStage::Encoding,
                    done,
                    total,
                    queue_now_millis(),
                );
            },
        )?;
        self.store.set_staging_manifest(
            id,
            receipt.manifest.bytes().to_vec(),
            queue_now_millis(),
        )?;
        File::open(&staging)
            .and_then(|file| file.sync_all())
            .map_err(QueueError::Staging)?;
        if self.is_cancelled(id) {
            let _ = fs::remove_dir_all(&staging);
            return Err(QueueError::Cancelled);
        }
        self.store
            .transition(id, ExportJobState::Committing, queue_now_millis())?;
        let commit_receipt = destination
            .commit(&staging, job.destination_target(), job.idempotency_key())
            .map_err(QueueError::Destination)?;
        self.store.succeed(id, commit_receipt, queue_now_millis())?;
        Ok(receipt)
    }

    fn is_cancelled(&self, id: ExportJobId) -> bool {
        self.store
            .get(id)
            .ok()
            .flatten()
            .is_some_and(|job| job.state() == ExportJobState::Cancelled)
    }
}

struct ParsedCopySnapshot {
    source_id: String,
    source: SourceDescriptor,
    sidecar: Option<SidecarSettings>,
}

fn parse_copy_snapshot(bytes: &[u8]) -> Result<ParsedCopySnapshot, QueueError> {
    let text = std::str::from_utf8(bytes).map_err(|_| QueueError::SourceProvider)?;
    let values = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect::<std::collections::BTreeMap<_, _>>();
    if values.get("schema") != Some(&COPY_SNAPSHOT_SCHEMA) {
        return Err(QueueError::SourceProvider);
    }
    let source_id = values
        .get("source_id")
        .ok_or(QueueError::SourceProvider)?
        .to_string();
    let expected_sha256 = parse_hex(
        values
            .get("source_sha256")
            .ok_or(QueueError::SourceProvider)?,
    )?;
    let byte_length = values
        .get("source_bytes")
        .ok_or(QueueError::SourceProvider)?
        .parse()
        .map_err(|_| QueueError::SourceProvider)?;
    let extension = values
        .get("source_extension")
        .ok_or(QueueError::SourceProvider)?
        .to_string();
    let sidecar = match values.get("sidecar") {
        Some(value) if *value != "none" => {
            let (revision, hash) = value.split_once(':').ok_or(QueueError::SourceProvider)?;
            Some(SidecarSettings::new(
                revision.parse().map_err(|_| QueueError::SourceProvider)?,
                parse_hex(hash)?,
            ))
        }
        _ => None,
    };
    Ok(ParsedCopySnapshot {
        source_id,
        source: SourceDescriptor::new(expected_sha256, byte_length, extension),
        sidecar,
    })
}

fn next_job_id(store: &RedbExportQueueStore) -> Result<u128, QueueError> {
    Ok(store
        .list()?
        .into_iter()
        .map(|job| job.id().get())
        .max()
        .unwrap_or(0)
        .saturating_add(1))
}

fn request_destination_id(request: &ExportRequest) -> String {
    request.destination_id().to_owned()
}

fn validate_identifier(value: &str, max: usize, _field: &'static str) -> Result<(), QueueError> {
    if value.is_empty() || value.len() > max || value.contains('\0') || value.contains('\n') {
        Err(QueueError::InvalidIdentifier("opaque identifier"))
    } else {
        Ok(())
    }
}
fn validate_target(value: &str) -> Result<(), QueueError> {
    if value.is_empty()
        || value.len() > MAX_TARGET_BYTES
        || Path::new(value).is_absolute()
        || value.split('/').any(|part| part.is_empty() || part == "..")
    {
        Err(QueueError::InvalidTarget)
    } else {
        Ok(())
    }
}
fn parse_hex(value: &str) -> Result<[u8; 32], QueueError> {
    if value.len() != 64 {
        return Err(QueueError::SourceProvider);
    }
    let mut bytes = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_digit(pair[0])? << 4) | hex_digit(pair[1])?;
    }
    Ok(bytes)
}
fn hex_digit(value: u8) -> Result<u8, QueueError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(QueueError::SourceProvider),
    }
}
fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_retryable(error: &QueueError) -> bool {
    matches!(
        error,
        QueueError::Source(SourceProviderError::Unavailable)
            | QueueError::Copy(copy::Error::Io { .. })
            | QueueError::Copy(copy::Error::SourceRead(SourceSnapshotReadError::Io { .. },))
            | QueueError::Destination(DestinationError::Io { .. })
    )
}

fn retry_delay(id: ExportJobId, attempt: u32) -> u64 {
    let mut hash = Sha256::new();
    hash.update(id.get().to_be_bytes());
    hash.update(attempt.to_be_bytes());
    let digest = hash.finalize();
    let jitter = u64::from(u16::from_be_bytes([digest[0], digest[1]]) % 251);
    1_000_u64
        .saturating_mul(1_u64 << attempt.min(6))
        .saturating_add(jitter)
}
