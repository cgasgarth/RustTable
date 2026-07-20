//! Durable export queue records and transactional state transitions.
#![allow(clippy::match_same_arms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::too_many_arguments)]

use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use postcard::{from_bytes, to_allocvec};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const QUEUE_SCHEMA_VERSION: u8 = 1;
const QUEUE_META: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_queue_meta");
const QUEUE_JOBS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_queue_jobs");
const QUEUE_IDEMPOTENCY: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_queue_idempotency");
const QUEUE_TRANSITIONS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("rusttable_export_queue_transitions");
const SCHEMA_KEY: &[u8] = b"schema-version";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExportJobId(u128);

impl ExportJobId {
    #[must_use]
    pub fn new(value: u128) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    #[must_use]
    pub const fn get(self) -> u128 {
        self.0
    }
}

impl fmt::Display for ExportJobId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:032x}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExportJobPriority {
    Background,
    Normal,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExportJobState {
    Queued,
    Preparing,
    Rendering,
    Encoding,
    Committing,
    Succeeded,
    FailedRetryable,
    FailedPermanent,
    Cancelled,
    Interrupted,
}

impl ExportJobState {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::FailedPermanent | Self::Cancelled
        )
    }

    #[must_use]
    pub const fn is_in_process(self) -> bool {
        matches!(
            self,
            Self::Preparing | Self::Rendering | Self::Encoding | Self::Committing
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ExportJobStage {
    Preparing,
    Rendering,
    Encoding,
    Committing,
}

impl ExportJobStage {
    #[must_use]
    const fn weight(self) -> u64 {
        match self {
            Self::Preparing => 5,
            Self::Rendering => 50,
            Self::Encoding => 30,
            Self::Committing => 15,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportJobRecord {
    id: ExportJobId,
    request_hash: [u8; 32],
    idempotency_key: String,
    destination_id: String,
    destination_target: String,
    snapshot: Vec<u8>,
    priority: ExportJobPriority,
    state: ExportJobState,
    stage: Option<ExportJobStage>,
    progress: u16,
    attempt: u32,
    next_retry_at: Option<u64>,
    transition_count: u64,
    staging_manifest: Option<Vec<u8>>,
    receipt: Option<Vec<u8>>,
    last_error: Option<String>,
    created_at: u64,
    updated_at: u64,
}

impl ExportJobRecord {
    #[must_use]
    pub fn new(
        id: ExportJobId,
        request_hash: [u8; 32],
        idempotency_key: String,
        destination_id: String,
        destination_target: String,
        snapshot: Vec<u8>,
        priority: ExportJobPriority,
        now: u64,
    ) -> Self {
        Self {
            id,
            request_hash,
            idempotency_key,
            destination_id,
            destination_target,
            snapshot,
            priority,
            state: ExportJobState::Queued,
            stage: None,
            progress: 0,
            attempt: 0,
            next_retry_at: None,
            transition_count: 0,
            staging_manifest: None,
            receipt: None,
            last_error: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[must_use]
    pub const fn id(&self) -> ExportJobId {
        self.id
    }
    #[must_use]
    pub const fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }
    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }
    #[must_use]
    pub fn destination_id(&self) -> &str {
        &self.destination_id
    }
    #[must_use]
    pub fn destination_target(&self) -> &str {
        &self.destination_target
    }
    #[must_use]
    pub fn snapshot(&self) -> &[u8] {
        &self.snapshot
    }
    #[must_use]
    pub const fn priority(&self) -> ExportJobPriority {
        self.priority
    }
    #[must_use]
    pub const fn state(&self) -> ExportJobState {
        self.state
    }
    #[must_use]
    pub const fn stage(&self) -> Option<ExportJobStage> {
        self.stage
    }
    #[must_use]
    pub const fn progress(&self) -> u16 {
        self.progress
    }
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }
    #[must_use]
    pub const fn next_retry_at(&self) -> Option<u64> {
        self.next_retry_at
    }
    #[must_use]
    pub const fn transition_count(&self) -> u64 {
        self.transition_count
    }
    #[must_use]
    pub fn staging_manifest(&self) -> Option<&[u8]> {
        self.staging_manifest.as_deref()
    }
    #[must_use]
    pub fn receipt(&self) -> Option<&[u8]> {
        self.receipt.as_deref()
    }
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportQueueError {
    Unavailable,
    Corrupt,
    CommitFailed,
    InvalidIdempotencyKey,
    IdempotencyConflict {
        job_id: ExportJobId,
    },
    UnknownJob(ExportJobId),
    InvalidTransition {
        from: ExportJobState,
        to: ExportJobState,
    },
    ProgressRegressed {
        current: u16,
        requested: u16,
    },
    InvalidProgress,
    InvalidTimestamp,
    Serialization,
}

impl fmt::Display for ExportQueueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("export queue store is unavailable"),
            Self::Corrupt => formatter.write_str("export queue data is corrupt"),
            Self::CommitFailed => formatter.write_str("export queue commit failed"),
            Self::InvalidIdempotencyKey => formatter.write_str("invalid export idempotency key"),
            Self::IdempotencyConflict { job_id } => {
                write!(formatter, "idempotency key conflicts with job {job_id}")
            }
            Self::UnknownJob(job_id) => write!(formatter, "unknown export job {job_id}"),
            Self::InvalidTransition { from, to } => {
                write!(formatter, "invalid export transition {from:?} -> {to:?}")
            }
            Self::ProgressRegressed { current, requested } => write!(
                formatter,
                "export progress regressed from {current} to {requested}"
            ),
            Self::InvalidProgress => {
                formatter.write_str("export progress must be between 0 and 10,000")
            }
            Self::InvalidTimestamp => formatter.write_str("export retry timestamp is invalid"),
            Self::Serialization => formatter.write_str("export queue record serialization failed"),
        }
    }
}

impl std::error::Error for ExportQueueError {}

#[derive(Debug, Serialize, Deserialize)]
struct StoredJob {
    id: ExportJobId,
    request_hash: [u8; 32],
    idempotency_key: String,
    destination_id: String,
    destination_target: String,
    snapshot: Vec<u8>,
    priority: ExportJobPriority,
    state: ExportJobState,
    stage: Option<ExportJobStage>,
    progress: u16,
    attempt: u32,
    next_retry_at: Option<u64>,
    transition_count: u64,
    staging_manifest: Option<Vec<u8>>,
    receipt: Option<Vec<u8>>,
    last_error: Option<String>,
    created_at: u64,
    updated_at: u64,
}

impl From<&ExportJobRecord> for StoredJob {
    fn from(job: &ExportJobRecord) -> Self {
        Self {
            id: job.id,
            request_hash: job.request_hash,
            idempotency_key: job.idempotency_key.clone(),
            destination_id: job.destination_id.clone(),
            destination_target: job.destination_target.clone(),
            snapshot: job.snapshot.clone(),
            priority: job.priority,
            state: job.state,
            stage: job.stage,
            progress: job.progress,
            attempt: job.attempt,
            next_retry_at: job.next_retry_at,
            transition_count: job.transition_count,
            staging_manifest: job.staging_manifest.clone(),
            receipt: job.receipt.clone(),
            last_error: job.last_error.clone(),
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

impl TryFrom<StoredJob> for ExportJobRecord {
    type Error = ExportQueueError;

    fn try_from(job: StoredJob) -> Result<Self, Self::Error> {
        if job.id.get() == 0 || job.progress > 10_000 || job.idempotency_key.trim().is_empty() {
            return Err(ExportQueueError::Corrupt);
        }
        Ok(Self {
            id: job.id,
            request_hash: job.request_hash,
            idempotency_key: job.idempotency_key,
            destination_id: job.destination_id,
            destination_target: job.destination_target,
            snapshot: job.snapshot,
            priority: job.priority,
            state: job.state,
            stage: job.stage,
            progress: job.progress,
            attempt: job.attempt,
            next_retry_at: job.next_retry_at,
            transition_count: job.transition_count,
            staging_manifest: job.staging_manifest,
            receipt: job.receipt,
            last_error: job.last_error,
            created_at: job.created_at,
            updated_at: job.updated_at,
        })
    }
}

pub struct RedbExportQueueStore {
    database: Arc<Database>,
}

impl RedbExportQueueStore {
    /// Opens a queue alongside the catalog database and creates its tables once.
    pub fn open(path: &Path) -> Result<Self, ExportQueueError> {
        let database = Arc::new(Database::create(path).map_err(|_| ExportQueueError::Unavailable)?);
        let transaction = database
            .begin_write()
            .map_err(|_| ExportQueueError::Unavailable)?;
        {
            let mut meta = transaction
                .open_table(QUEUE_META)
                .map_err(|_| ExportQueueError::Unavailable)?;
            if let Some(version) = meta
                .get(SCHEMA_KEY)
                .map_err(|_| ExportQueueError::Corrupt)?
            {
                if version.value() != [QUEUE_SCHEMA_VERSION] {
                    return Err(ExportQueueError::Corrupt);
                }
            } else {
                meta.insert(SCHEMA_KEY, &[QUEUE_SCHEMA_VERSION][..])
                    .map_err(|_| ExportQueueError::Unavailable)?;
            }
            transaction
                .open_table(QUEUE_JOBS)
                .map_err(|_| ExportQueueError::Unavailable)?;
            transaction
                .open_table(QUEUE_IDEMPOTENCY)
                .map_err(|_| ExportQueueError::Unavailable)?;
            transaction
                .open_table(QUEUE_TRANSITIONS)
                .map_err(|_| ExportQueueError::Unavailable)?;
        }
        transaction
            .commit()
            .map_err(|_| ExportQueueError::CommitFailed)?;
        Ok(Self { database })
    }

    /// Enqueues an immutable snapshot. A duplicate key is safe to replay; a conflicting key is rejected.
    pub fn enqueue(&self, mut job: ExportJobRecord) -> Result<ExportJobRecord, ExportQueueError> {
        if job.idempotency_key.trim().is_empty() || job.idempotency_key.contains('\0') {
            return Err(ExportQueueError::InvalidIdempotencyKey);
        }
        if job.id.get() == 0 {
            return Err(ExportQueueError::Corrupt);
        }
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| ExportQueueError::Unavailable)?;
        {
            let mut ids = transaction
                .open_table(QUEUE_IDEMPOTENCY)
                .map_err(|_| ExportQueueError::Unavailable)?;
            if let Some(existing_id) = ids
                .get(job.idempotency_key.as_bytes())
                .map_err(|_| ExportQueueError::Corrupt)?
            {
                let existing_id = existing_id.value().to_vec();
                let jobs = transaction
                    .open_table(QUEUE_JOBS)
                    .map_err(|_| ExportQueueError::Corrupt)?;
                let bytes = jobs
                    .get(existing_id.as_slice())
                    .map_err(|_| ExportQueueError::Corrupt)?
                    .ok_or(ExportQueueError::Corrupt)?
                    .value()
                    .to_vec();
                let existing = decode(&bytes)?;
                if existing.request_hash != job.request_hash
                    || existing.destination_target != job.destination_target
                {
                    return Err(ExportQueueError::IdempotencyConflict {
                        job_id: existing.id,
                    });
                }
                return Ok(existing);
            }
            let mut jobs = transaction
                .open_table(QUEUE_JOBS)
                .map_err(|_| ExportQueueError::Unavailable)?;
            let mut job_key = job.id.get().to_be_bytes();
            if jobs
                .get(job_key.as_slice())
                .map_err(|_| ExportQueueError::Unavailable)?
                .is_some()
            {
                let next = jobs
                    .iter()
                    .map_err(|_| ExportQueueError::Corrupt)?
                    .filter_map(Result::ok)
                    .filter_map(|(key, _)| <[u8; 16]>::try_from(key.value()).ok())
                    .map(u128::from_be_bytes)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or(ExportQueueError::Corrupt)?;
                job.id = ExportJobId::new(next).ok_or(ExportQueueError::Corrupt)?;
                job_key = job.id.get().to_be_bytes();
            }
            let encoded = encode(&job)?;
            jobs.insert(job_key.as_slice(), encoded.as_slice())
                .map_err(|_| ExportQueueError::Unavailable)?;
            ids.insert(job.idempotency_key.as_bytes(), job_key.as_slice())
                .map_err(|_| ExportQueueError::Unavailable)?;
            append_transition(&transaction, &mut job)?;
        }
        transaction
            .commit()
            .map_err(|_| ExportQueueError::CommitFailed)?;
        Ok(job)
    }

    pub fn get(&self, id: ExportJobId) -> Result<Option<ExportJobRecord>, ExportQueueError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| ExportQueueError::Unavailable)?;
        let jobs = transaction
            .open_table(QUEUE_JOBS)
            .map_err(|_| ExportQueueError::Corrupt)?;
        let key = id.get().to_be_bytes();
        jobs.get(key.as_slice())
            .map_err(|_| ExportQueueError::Corrupt)?
            .map(|value| decode(value.value()))
            .transpose()
    }

    pub fn list(&self) -> Result<Vec<ExportJobRecord>, ExportQueueError> {
        let transaction = self
            .database
            .begin_read()
            .map_err(|_| ExportQueueError::Unavailable)?;
        let jobs = transaction
            .open_table(QUEUE_JOBS)
            .map_err(|_| ExportQueueError::Corrupt)?;
        jobs.iter()
            .map_err(|_| ExportQueueError::Corrupt)?
            .map(|entry| {
                let (_, value) = entry.map_err(|_| ExportQueueError::Corrupt)?;
                decode(value.value())
            })
            .collect()
    }

    pub fn transition(
        &self,
        id: ExportJobId,
        to: ExportJobState,
        now: u64,
    ) -> Result<ExportJobRecord, ExportQueueError> {
        self.mutate(id, now, |job| {
            if !valid_transition(job.state, to) {
                return Err(ExportQueueError::InvalidTransition {
                    from: job.state,
                    to,
                });
            }
            job.state = to;
            job.stage = match to {
                ExportJobState::Preparing => Some(ExportJobStage::Preparing),
                ExportJobState::Rendering => Some(ExportJobStage::Rendering),
                ExportJobState::Encoding => Some(ExportJobStage::Encoding),
                ExportJobState::Committing => Some(ExportJobStage::Committing),
                ExportJobState::Succeeded => Some(ExportJobStage::Committing),
                _ => job.stage,
            };
            if to == ExportJobState::Queued {
                job.next_retry_at = None;
            }
            Ok(())
        })
    }

    pub fn update_progress(
        &self,
        id: ExportJobId,
        stage: ExportJobStage,
        completed: u64,
        total: u64,
        now: u64,
    ) -> Result<ExportJobRecord, ExportQueueError> {
        if total == 0 || completed > total {
            return Err(ExportQueueError::InvalidProgress);
        }
        let stage_progress = completed.saturating_mul(10_000) / total;
        let progress = u16::try_from((stage.weight() * stage_progress) / 100)
            .map_err(|_| ExportQueueError::InvalidProgress)?;
        self.mutate(id, now, |job| {
            if progress < job.progress {
                return Err(ExportQueueError::ProgressRegressed {
                    current: job.progress,
                    requested: progress,
                });
            }
            job.stage = Some(stage);
            job.progress = progress;
            Ok(())
        })
    }

    pub fn set_staging_manifest(
        &self,
        id: ExportJobId,
        manifest: Vec<u8>,
        now: u64,
    ) -> Result<ExportJobRecord, ExportQueueError> {
        self.mutate(id, now, |job| {
            job.staging_manifest = Some(manifest.clone());
            Ok(())
        })
    }

    pub fn succeed(
        &self,
        id: ExportJobId,
        receipt: Vec<u8>,
        now: u64,
    ) -> Result<ExportJobRecord, ExportQueueError> {
        self.mutate(id, now, |job| {
            if !valid_transition(job.state, ExportJobState::Succeeded) {
                return Err(ExportQueueError::InvalidTransition {
                    from: job.state,
                    to: ExportJobState::Succeeded,
                });
            }
            job.state = ExportJobState::Succeeded;
            job.progress = 10_000;
            job.receipt = Some(receipt.clone());
            Ok(())
        })
    }

    pub fn fail(
        &self,
        id: ExportJobId,
        retryable: bool,
        error: String,
        retry_at: Option<u64>,
        now: u64,
    ) -> Result<ExportJobRecord, ExportQueueError> {
        if retryable && retry_at.is_none() {
            return Err(ExportQueueError::InvalidTimestamp);
        }
        self.mutate(id, now, |job| {
            let to = if retryable {
                ExportJobState::FailedRetryable
            } else {
                ExportJobState::FailedPermanent
            };
            if !valid_transition(job.state, to) {
                return Err(ExportQueueError::InvalidTransition {
                    from: job.state,
                    to,
                });
            }
            job.state = to;
            job.attempt = job.attempt.saturating_add(1);
            job.last_error = Some(redact_error(&error));
            job.next_retry_at = retry_at;
            Ok(())
        })
    }

    pub fn cancel(&self, id: ExportJobId, now: u64) -> Result<ExportJobRecord, ExportQueueError> {
        self.transition(id, ExportJobState::Cancelled, now)
    }

    pub fn retry(&self, id: ExportJobId, now: u64) -> Result<ExportJobRecord, ExportQueueError> {
        self.mutate(id, now, |job| {
            if !matches!(
                job.state,
                ExportJobState::FailedRetryable | ExportJobState::Interrupted
            ) {
                return Err(ExportQueueError::InvalidTransition {
                    from: job.state,
                    to: ExportJobState::Queued,
                });
            }
            job.state = ExportJobState::Queued;
            job.stage = None;
            job.progress = 0;
            job.next_retry_at = None;
            job.last_error = None;
            Ok(())
        })
    }

    /// Converts all in-process jobs to `Interrupted` before a worker is restarted.
    pub fn recover_in_process(&self, now: u64) -> Result<Vec<ExportJobRecord>, ExportQueueError> {
        let ids = self
            .list()?
            .into_iter()
            .filter(|job| job.state.is_in_process())
            .map(|job| job.id)
            .collect::<Vec<_>>();
        ids.into_iter()
            .map(|id| self.transition(id, ExportJobState::Interrupted, now))
            .collect()
    }

    fn mutate<F>(
        &self,
        id: ExportJobId,
        now: u64,
        mut change: F,
    ) -> Result<ExportJobRecord, ExportQueueError>
    where
        F: FnMut(&mut ExportJobRecord) -> Result<(), ExportQueueError>,
    {
        let transaction = self
            .database
            .begin_write()
            .map_err(|_| ExportQueueError::Unavailable)?;
        let mut jobs = transaction
            .open_table(QUEUE_JOBS)
            .map_err(|_| ExportQueueError::Unavailable)?;
        let key = id.get().to_be_bytes();
        let bytes = jobs
            .get(key.as_slice())
            .map_err(|_| ExportQueueError::Corrupt)?
            .ok_or(ExportQueueError::UnknownJob(id))?
            .value()
            .to_vec();
        let mut job = decode(&bytes)?;
        change(&mut job)?;
        job.updated_at = now;
        job.transition_count = job.transition_count.saturating_add(1);
        let encoded = encode(&job)?;
        jobs.insert(key.as_slice(), encoded.as_slice())
            .map_err(|_| ExportQueueError::Unavailable)?;
        append_transition(&transaction, &mut job)?;
        drop(jobs);
        transaction
            .commit()
            .map_err(|_| ExportQueueError::CommitFailed)?;
        Ok(job)
    }
}

fn encode(job: &ExportJobRecord) -> Result<Vec<u8>, ExportQueueError> {
    to_allocvec(&StoredJob::from(job)).map_err(|_| ExportQueueError::Serialization)
}

fn decode(bytes: &[u8]) -> Result<ExportJobRecord, ExportQueueError> {
    from_bytes::<StoredJob>(bytes)
        .map_err(|_| ExportQueueError::Corrupt)
        .and_then(ExportJobRecord::try_from)
}

fn append_transition(
    transaction: &redb::WriteTransaction,
    job: &mut ExportJobRecord,
) -> Result<(), ExportQueueError> {
    let key = transition_key(job.id, job.transition_count);
    let value = to_allocvec(&(job.state, job.stage, job.progress, job.updated_at))
        .map_err(|_| ExportQueueError::Serialization)?;
    transaction
        .open_table(QUEUE_TRANSITIONS)
        .map_err(|_| ExportQueueError::Unavailable)?
        .insert(key.as_slice(), value.as_slice())
        .map_err(|_| ExportQueueError::Unavailable)?;
    Ok(())
}

fn transition_key(id: ExportJobId, sequence: u64) -> [u8; 24] {
    let mut key = [0_u8; 24];
    key[..16].copy_from_slice(&id.get().to_be_bytes());
    key[16..].copy_from_slice(&sequence.to_be_bytes());
    key
}

const fn valid_transition(from: ExportJobState, to: ExportJobState) -> bool {
    matches!(
        (from, to),
        (
            ExportJobState::Queued,
            ExportJobState::Preparing | ExportJobState::Cancelled
        ) | (
            ExportJobState::Preparing,
            ExportJobState::Rendering
                | ExportJobState::Encoding
                | ExportJobState::FailedRetryable
                | ExportJobState::FailedPermanent
                | ExportJobState::Cancelled
                | ExportJobState::Interrupted
        ) | (
            ExportJobState::Rendering,
            ExportJobState::Encoding
                | ExportJobState::FailedRetryable
                | ExportJobState::FailedPermanent
                | ExportJobState::Cancelled
                | ExportJobState::Interrupted
        ) | (
            ExportJobState::Encoding,
            ExportJobState::Committing
                | ExportJobState::FailedRetryable
                | ExportJobState::FailedPermanent
                | ExportJobState::Cancelled
                | ExportJobState::Interrupted
        ) | (
            ExportJobState::Committing,
            ExportJobState::Succeeded
                | ExportJobState::FailedRetryable
                | ExportJobState::FailedPermanent
                | ExportJobState::Interrupted
        )
    )
}

fn redact_error(error: &str) -> String {
    error
        .split_whitespace()
        .filter(|part| !part.contains('/') && !part.contains('\\'))
        .collect::<Vec<_>>()
        .join(" ")
}

#[must_use]
pub fn queue_now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}
