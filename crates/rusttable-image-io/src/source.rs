use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusttable_image::DecodeLimits;
use sha2::{Digest, Sha256};

static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSnapshotMode {
    Handle,
    StableCopy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotPolicy {
    max_bytes: u64,
    max_read_bytes: u64,
    mode: SourceSnapshotMode,
    allow_symlinks: bool,
    allowed_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotPolicyError {
    ZeroLimit,
}

impl SnapshotPolicy {
    /// Creates a bounded handle-backed snapshot policy.
    ///
    /// # Errors
    ///
    /// Returns an error when either limit is zero.
    pub fn new(max_bytes: u64, max_read_bytes: u64) -> Result<Self, SnapshotPolicyError> {
        if max_bytes == 0 || max_read_bytes == 0 {
            return Err(SnapshotPolicyError::ZeroLimit);
        }
        Ok(Self {
            max_bytes,
            max_read_bytes,
            mode: SourceSnapshotMode::Handle,
            allow_symlinks: false,
            allowed_root: None,
        })
    }

    /// Derives matching source and read budgets from image decode limits.
    ///
    /// # Errors
    ///
    /// Returns an error when the decode source limit is zero.
    pub fn from_decode_limits(limits: DecodeLimits) -> Result<Self, SnapshotPolicyError> {
        Self::new(limits.max_source_bytes(), limits.max_source_bytes())
    }

    #[must_use]
    pub const fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    #[must_use]
    pub const fn max_read_bytes(&self) -> u64 {
        self.max_read_bytes
    }

    #[must_use]
    pub const fn mode(&self) -> SourceSnapshotMode {
        self.mode
    }

    #[must_use]
    pub const fn allows_symlinks(&self) -> bool {
        self.allow_symlinks
    }

    #[must_use]
    pub fn with_stable_copy(mut self) -> Self {
        self.mode = SourceSnapshotMode::StableCopy;
        self
    }

    #[must_use]
    pub fn with_symlinks_under(mut self, root: impl Into<PathBuf>) -> Self {
        self.allow_symlinks = true;
        self.allowed_root = Some(root.into());
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FileIdentityClass {
    Regular,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SourceIdentity {
    Unix { device: u64, inode: u64 },
    Windows { volume: u32, file_index: u64 },
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    identity: SourceIdentity,
    length: u64,
    modified_nanos: u128,
}

impl Fingerprint {
    fn from_metadata(metadata: &Metadata) -> Self {
        Self {
            identity: source_identity(metadata),
            length: metadata.len(),
            modified_nanos: modified_nanos(metadata),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceAlias(String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceAliasError {
    Empty,
    TooLong,
}

impl SourceAlias {
    /// Creates a bounded privacy-safe diagnostic alias.
    ///
    /// # Errors
    ///
    /// Returns an error for empty aliases or aliases longer than 128 bytes.
    pub fn new(alias: impl Into<String>) -> Result<Self, SourceAliasError> {
        let alias = alias.into();
        if alias.is_empty() {
            return Err(SourceAliasError::Empty);
        }
        if alias.len() > 128 {
            return Err(SourceAliasError::TooLong);
        }
        Ok(Self(alias))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentEvidence {
    length: u64,
    bytes_read: u64,
    hash: HashStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashStatus {
    NotComputed,
    Sha256([u8; 32]),
}

impl ContentEvidence {
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    #[must_use]
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    #[must_use]
    pub const fn hash(&self) -> &HashStatus {
        &self.hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotReceipt {
    alias: SourceAlias,
    length: u64,
    identity_class: FileIdentityClass,
    hash_status: HashStatus,
    bytes_read: u64,
    elapsed: Duration,
}

impl SnapshotReceipt {
    #[must_use]
    pub const fn length(&self) -> u64 {
        self.length
    }

    #[must_use]
    pub fn alias(&self) -> &SourceAlias {
        &self.alias
    }

    #[must_use]
    pub const fn identity_class(&self) -> FileIdentityClass {
        self.identity_class
    }

    #[must_use]
    pub const fn hash_status(&self) -> &HashStatus {
        &self.hash_status
    }

    #[must_use]
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceChanged;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSnapshotError {
    InvalidPolicy(SnapshotPolicyError),
    InvalidAlias(SourceAliasError),
    Io {
        operation: &'static str,
        kind: io::ErrorKind,
    },
    NotRegularFile,
    SymlinkRejected,
    SymlinkOutsideRoot,
    SourceTooLarge {
        limit: u64,
        actual: u64,
    },
    SourceChanged,
    TemporaryCopyFailed,
    ArithmeticOverflow,
}

pub trait ReadCancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

pub trait PositionedSourceReader {
    /// Reads exactly one bounded range without changing the reader position.
    ///
    /// # Errors
    ///
    /// Returns a typed bounds, budget, source-change, short-read, or I/O error.
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), SourceReadError>;
}

pub trait SequentialSourceReader {
    /// Reads the next bounded range from the source.
    ///
    /// # Errors
    ///
    /// Returns a typed bounds, budget, source-change, short-read, or I/O error.
    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), SourceReadError>;
    fn position(&self) -> u64;
}

struct SnapshotState {
    file: File,
    logical_path: PathBuf,
    fingerprint: Fingerprint,
    policy: SnapshotPolicy,
    bytes_read: AtomicU64,
    hash: OnceLock<[u8; 32]>,
    _temporary: Option<TemporaryPath>,
    started: Instant,
}

struct TemporaryPath(PathBuf);

impl Drop for TemporaryPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[derive(Clone)]
pub struct SourceSnapshot {
    state: Arc<SnapshotState>,
}

impl fmt::Debug for SourceSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SourceSnapshot")
            .field("logical_path", &self.state.logical_path)
            .field("fingerprint", &self.state.fingerprint)
            .field("policy", &self.state.policy)
            .finish_non_exhaustive()
    }
}

impl SourceSnapshot {
    /// Opens one regular-file snapshot according to the supplied policy.
    ///
    /// # Errors
    ///
    /// Returns an error for path-policy violations, non-regular files, source
    /// changes, size limits, or open/copy failures.
    pub fn open(path: &Path, policy: SnapshotPolicy) -> Result<Self, SourceSnapshotError> {
        validate_policy_path(path, &policy)?;
        match policy.mode {
            SourceSnapshotMode::Handle => Self::open_handle(path, policy),
            SourceSnapshotMode::StableCopy => Self::open_stable_copy(path, policy),
        }
    }

    fn open_handle(path: &Path, policy: SnapshotPolicy) -> Result<Self, SourceSnapshotError> {
        let (file, fingerprint, logical_path) = open_regular(path, &policy)?;
        enforce_size(policy.max_bytes, fingerprint.length)?;
        Ok(Self {
            state: Arc::new(SnapshotState {
                file,
                logical_path,
                fingerprint,
                policy,
                bytes_read: AtomicU64::new(0),
                hash: OnceLock::new(),
                _temporary: None,
                started: Instant::now(),
            }),
        })
    }

    fn open_stable_copy(path: &Path, policy: SnapshotPolicy) -> Result<Self, SourceSnapshotError> {
        let (source, fingerprint, logical_path) = open_regular(path, &policy)?;
        enforce_size(policy.max_bytes, fingerprint.length)?;
        let temporary = temporary_path(path);
        let mut temporary_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| io_error("create temporary snapshot", &error))?;
        let hash = copy_and_sync(&source, &mut temporary_file, fingerprint.length, &policy);
        if hash.is_err() {
            let _ = fs::remove_file(&temporary);
            return Err(SourceSnapshotError::TemporaryCopyFailed);
        }
        if current_path_fingerprint(&logical_path, &policy)? != fingerprint {
            let _ = fs::remove_file(&temporary);
            return Err(SourceSnapshotError::SourceChanged);
        }
        let published = temporary.with_extension("published");
        fs::rename(&temporary, &published)
            .map_err(|error| io_error("publish temporary snapshot", &error))?;
        let file =
            File::open(&published).map_err(|error| io_error("open stable snapshot", &error))?;
        let fingerprint = Fingerprint::from_metadata(
            &file
                .metadata()
                .map_err(|error| io_error("stat stable snapshot", &error))?,
        );
        let hash = hash.map_err(|_| SourceSnapshotError::TemporaryCopyFailed)?;
        let cached_hash = OnceLock::new();
        let _ = cached_hash.set(hash);
        Ok(Self {
            state: Arc::new(SnapshotState {
                file,
                logical_path,
                fingerprint,
                policy,
                bytes_read: AtomicU64::new(0),
                hash: cached_hash,
                _temporary: Some(TemporaryPath(published)),
                started: Instant::now(),
            }),
        })
    }

    #[must_use]
    pub fn length(&self) -> u64 {
        self.state.fingerprint.length
    }

    #[must_use]
    pub fn identity(&self) -> SourceIdentity {
        self.state.fingerprint.identity
    }

    #[must_use]
    pub fn logical_path(&self) -> &Path {
        &self.state.logical_path
    }

    /// Rechecks handle identity, length, and modification evidence.
    ///
    /// # Errors
    ///
    /// Returns `SourceChanged` when the handle or diagnostic path no longer
    /// describes the snapshot opened earlier.
    pub fn revalidate(&self) -> Result<(), SourceChanged> {
        let metadata = self.state.file.metadata().map_err(|_| SourceChanged)?;
        let current = Fingerprint::from_metadata(&metadata);
        if current != self.state.fingerprint {
            return Err(SourceChanged);
        }
        if self.state.policy.mode == SourceSnapshotMode::Handle
            && current_path_fingerprint(&self.state.logical_path, &self.state.policy)
                .map_err(|_| SourceChanged)?
                != self.state.fingerprint
        {
            return Err(SourceChanged);
        }
        Ok(())
    }

    /// Reads the complete source under the configured byte budget.
    ///
    /// # Errors
    ///
    /// Returns a typed allocation, budget, bounds, short-read, or source-change error.
    pub fn read_all(&self) -> Result<Vec<u8>, SourceReadError> {
        let length =
            usize::try_from(self.length()).map_err(|_| SourceReadError::ArithmeticOverflow)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length)
            .map_err(|_| SourceReadError::AllocationFailure)?;
        bytes.resize(length, 0);
        self.read_exact_at(0, &mut bytes)?;
        Ok(bytes)
    }

    /// Creates an independent sequential reader over the same snapshot handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the operating system cannot clone the handle.
    pub fn sequential_reader(&self) -> Result<SequentialReader, SourceReadError> {
        let file = self
            .state
            .file
            .try_clone()
            .map_err(|error| SourceReadError::Io(io_error("clone source handle", &error)))?;
        Ok(SequentialReader {
            snapshot: self.clone(),
            file,
            position: 0,
        })
    }

    /// Reads one range while honoring a caller-owned cancellation token.
    ///
    /// # Errors
    ///
    /// Returns `Cancelled` before or after the read, or the underlying typed
    /// read error.
    pub fn read_exact_at_with_cancellation(
        &self,
        offset: u64,
        buffer: &mut [u8],
        cancellation: &dyn ReadCancellation,
    ) -> Result<(), SourceReadError> {
        if cancellation.is_cancelled() {
            return Err(SourceReadError::Cancelled);
        }
        self.read_exact_at(offset, buffer)?;
        if cancellation.is_cancelled() {
            return Err(SourceReadError::Cancelled);
        }
        Ok(())
    }

    /// Computes and caches SHA-256 after a complete unchanged read.
    ///
    /// # Errors
    ///
    /// Returns a typed budget, short-read, source-change, or I/O error.
    pub fn sha256(&self) -> Result<[u8; 32], SourceReadError> {
        self.revalidate()
            .map_err(|_| SourceReadError::SourceChanged)?;
        if let Some(hash) = self.state.hash.get() {
            return Ok(*hash);
        }
        let mut reader = self.sequential_reader()?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let remaining = self.length().saturating_sub(reader.position());
            if remaining == 0 {
                break;
            }
            let amount = usize::try_from(remaining.min(buffer.len() as u64))
                .map_err(|_| SourceReadError::ArithmeticOverflow)?;
            reader.read_exact(&mut buffer[..amount])?;
            digest.update(&buffer[..amount]);
        }
        self.revalidate()
            .map_err(|_| SourceReadError::SourceChanged)?;
        let result: [u8; 32] = digest.finalize().into();
        let _ = self.state.hash.set(result);
        Ok(result)
    }

    /// Verifies an expected hash and returns bounded content evidence.
    ///
    /// # Errors
    ///
    /// Returns `HashMismatch` or the typed error from hashing the source.
    pub fn verify_sha256(&self, expected: [u8; 32]) -> Result<ContentEvidence, SourceReadError> {
        let actual = self.sha256()?;
        if actual != expected {
            return Err(SourceReadError::HashMismatch);
        }
        Ok(self.content_evidence(HashStatus::Sha256(actual)))
    }

    #[must_use]
    pub fn content_evidence(&self, hash: HashStatus) -> ContentEvidence {
        ContentEvidence {
            length: self.length(),
            bytes_read: self.state.bytes_read.load(Ordering::Relaxed),
            hash,
        }
    }

    #[must_use]
    pub fn receipt(&self, alias: SourceAlias) -> SnapshotReceipt {
        SnapshotReceipt {
            alias,
            length: self.length(),
            identity_class: FileIdentityClass::Regular,
            hash_status: self
                .state
                .hash
                .get()
                .map_or(HashStatus::NotComputed, |hash| HashStatus::Sha256(*hash)),
            bytes_read: self.state.bytes_read.load(Ordering::Relaxed),
            elapsed: self.state.started.elapsed(),
        }
    }

    /// Creates a privacy-safe receipt from a caller-provided alias.
    ///
    /// # Errors
    ///
    /// Returns an error when the alias is empty or too long.
    pub fn receipt_for_alias(
        &self,
        alias: impl Into<String>,
    ) -> Result<SnapshotReceipt, SourceSnapshotError> {
        let alias = SourceAlias::new(alias).map_err(SourceSnapshotError::InvalidAlias)?;
        Ok(self.receipt(alias))
    }

    fn reserve_read(&self, bytes: u64) -> Result<(), SourceReadError> {
        if bytes > self.state.policy.max_read_bytes() {
            return Err(SourceReadError::ReadLimit {
                limit: self.state.policy.max_read_bytes(),
                attempted: bytes,
            });
        }
        self.state
            .bytes_read
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current
                    .checked_add(bytes)
                    .filter(|total| *total <= self.state.policy.max_read_bytes())
            })
            .map(|_| ())
            .map_err(|current| SourceReadError::ReadLimit {
                limit: self.state.policy.max_read_bytes(),
                attempted: current.saturating_add(bytes),
            })
    }
}

impl PositionedSourceReader for SourceSnapshot {
    fn read_exact_at(&self, offset: u64, buffer: &mut [u8]) -> Result<(), SourceReadError> {
        let length =
            u64::try_from(buffer.len()).map_err(|_| SourceReadError::ArithmeticOverflow)?;
        let end = offset
            .checked_add(length)
            .ok_or(SourceReadError::ArithmeticOverflow)?;
        if end > self.length() {
            return Err(SourceReadError::OutOfBounds {
                offset,
                length,
                source_length: self.length(),
            });
        }
        self.reserve_read(length)?;
        read_exact_at_file(&self.state.file, offset, buffer).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                SourceReadError::ShortRead {
                    expected: buffer.len(),
                }
            } else {
                SourceReadError::Io(io_error("positioned read", &error))
            }
        })?;
        self.revalidate()
            .map_err(|_| SourceReadError::SourceChanged)
    }
}

pub struct SequentialReader {
    snapshot: SourceSnapshot,
    file: File,
    position: u64,
}

impl SequentialSourceReader for SequentialReader {
    fn read_exact(&mut self, buffer: &mut [u8]) -> Result<(), SourceReadError> {
        let length =
            u64::try_from(buffer.len()).map_err(|_| SourceReadError::ArithmeticOverflow)?;
        let end = self
            .position
            .checked_add(length)
            .ok_or(SourceReadError::ArithmeticOverflow)?;
        if end > self.snapshot.length() {
            return Err(SourceReadError::OutOfBounds {
                offset: self.position,
                length,
                source_length: self.snapshot.length(),
            });
        }
        self.snapshot.reserve_read(length)?;
        read_exact_at_file(&self.file, self.position, buffer).map_err(|error| {
            if error.kind() == io::ErrorKind::UnexpectedEof {
                SourceReadError::ShortRead {
                    expected: buffer.len(),
                }
            } else {
                SourceReadError::Io(io_error("sequential read", &error))
            }
        })?;
        self.position = end;
        self.snapshot
            .revalidate()
            .map_err(|_| SourceReadError::SourceChanged)
    }

    fn position(&self) -> u64 {
        self.position
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceReadError {
    Io(SourceSnapshotError),
    OutOfBounds {
        offset: u64,
        length: u64,
        source_length: u64,
    },
    ReadLimit {
        limit: u64,
        attempted: u64,
    },
    SourceChanged,
    ShortRead {
        expected: usize,
    },
    HashMismatch,
    Cancelled,
    ArithmeticOverflow,
    AllocationFailure,
}

fn validate_policy_path(path: &Path, policy: &SnapshotPolicy) -> Result<(), SourceSnapshotError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect source path", &error))?;
    if metadata.file_type().is_symlink() {
        if !policy.allows_symlinks() {
            return Err(SourceSnapshotError::SymlinkRejected);
        }
        let resolved =
            fs::canonicalize(path).map_err(|error| io_error("resolve source symlink", &error))?;
        if let Some(root) = &policy.allowed_root {
            let root = fs::canonicalize(root)
                .map_err(|error| io_error("resolve source policy root", &error))?;
            if !resolved.starts_with(root) {
                return Err(SourceSnapshotError::SymlinkOutsideRoot);
            }
        }
    }
    Ok(())
}

fn open_regular(
    path: &Path,
    policy: &SnapshotPolicy,
) -> Result<(File, Fingerprint, PathBuf), SourceSnapshotError> {
    let file = File::open(path).map_err(|error| io_error("open source", &error))?;
    let metadata = file
        .metadata()
        .map_err(|error| io_error("stat source handle", &error))?;
    if !metadata.is_file() {
        return Err(SourceSnapshotError::NotRegularFile);
    }
    let fingerprint = Fingerprint::from_metadata(&metadata);
    let logical_path = path.to_owned();
    if current_path_fingerprint(&logical_path, policy)? != fingerprint {
        return Err(SourceSnapshotError::SourceChanged);
    }
    Ok((file, fingerprint, logical_path))
}

fn current_path_fingerprint(
    path: &Path,
    policy: &SnapshotPolicy,
) -> Result<Fingerprint, SourceSnapshotError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("revalidate source path", &error))?;
    if metadata.file_type().is_symlink() && !policy.allows_symlinks() {
        return Err(SourceSnapshotError::SymlinkRejected);
    }
    let target = if metadata.file_type().is_symlink() {
        if let Some(root) = &policy.allowed_root {
            let resolved = fs::canonicalize(path)
                .map_err(|error| io_error("resolve source symlink", &error))?;
            let root = fs::canonicalize(root)
                .map_err(|error| io_error("resolve source policy root", &error))?;
            if !resolved.starts_with(root) {
                return Err(SourceSnapshotError::SymlinkOutsideRoot);
            }
        }
        fs::metadata(path).map_err(|error| io_error("stat source symlink", &error))?
    } else {
        metadata
    };
    if !target.is_file() {
        return Err(SourceSnapshotError::NotRegularFile);
    }
    Ok(Fingerprint::from_metadata(&target))
}

fn enforce_size(limit: u64, actual: u64) -> Result<(), SourceSnapshotError> {
    if actual > limit {
        return Err(SourceSnapshotError::SourceTooLarge { limit, actual });
    }
    Ok(())
}

fn copy_and_sync(
    source: &File,
    destination: &mut File,
    length: u64,
    policy: &SnapshotPolicy,
) -> io::Result<[u8; 32]> {
    if length > policy.max_read_bytes() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "read budget",
        ));
    }
    let mut source = source.try_clone()?;
    let mut digest = Sha256::new();
    let mut remaining = length;
    let mut buffer = vec![0_u8; 64 * 1024];
    while remaining > 0 {
        let amount = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "snapshot length is unsupported")
        })?;
        source.read_exact(&mut buffer[..amount])?;
        destination.write_all(&buffer[..amount])?;
        digest.update(&buffer[..amount]);
        remaining -= amount as u64;
        if remaining > policy.max_read_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "read budget",
            ));
        }
    }
    destination.sync_all()?;
    Ok(digest.finalize().into())
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = format!(".rusttable-snapshot-{}-{sequence}.tmp", std::process::id());
    path.parent().unwrap_or_else(|| Path::new(".")).join(name)
}

#[cfg(unix)]
fn read_exact_at_file(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    let mut read = 0;
    while read < buffer.len() {
        let amount = file.read_at(&mut buffer[read..], offset + read as u64)?;
        if amount == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short positioned read",
            ));
        }
        read += amount;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at_file(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut read = 0;
    while read < buffer.len() {
        let amount = file.seek_read(&mut buffer[read..], offset + read as u64)?;
        if amount == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short positioned read",
            ));
        }
        read += amount;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn read_exact_at_file(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::io::{Seek, SeekFrom};
    let mut clone = file.try_clone()?;
    clone.seek(SeekFrom::Start(offset))?;
    clone.read_exact(buffer)
}

fn io_error(operation: &'static str, error: &io::Error) -> SourceSnapshotError {
    SourceSnapshotError::Io {
        operation,
        kind: error.kind(),
    }
}

fn modified_nanos(metadata: &Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(unix)]
fn source_identity(metadata: &Metadata) -> SourceIdentity {
    use std::os::unix::fs::MetadataExt;
    SourceIdentity::Unix {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(windows)]
fn source_identity(metadata: &Metadata) -> SourceIdentity {
    use std::os::windows::fs::MetadataExt;
    SourceIdentity::Windows {
        volume: metadata.volume_serial_number().unwrap_or(0),
        file_index: metadata.file_index().unwrap_or(0),
    }
}

#[cfg(not(any(unix, windows)))]
fn source_identity(_metadata: &Metadata) -> SourceIdentity {
    SourceIdentity::Unknown
}

impl fmt::Display for SnapshotPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("snapshot limits must be nonzero")
    }
}

impl std::error::Error for SnapshotPolicyError {}

impl fmt::Display for SourceAliasError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "source alias must be nonempty",
            Self::TooLong => "source alias exceeds 128 bytes",
        })
    }
}

impl std::error::Error for SourceAliasError {}

impl fmt::Display for SourceSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPolicy(error) => error.fmt(formatter),
            Self::InvalidAlias(error) => error.fmt(formatter),
            Self::Io { operation, kind } => write!(formatter, "{operation} failed: {kind:?}"),
            Self::NotRegularFile => formatter.write_str("source is not a regular file"),
            Self::SymlinkRejected => formatter.write_str("source symlink is not allowed"),
            Self::SymlinkOutsideRoot => formatter.write_str("source symlink leaves policy root"),
            Self::SourceTooLarge { limit, actual } => {
                write!(formatter, "source is {actual} bytes, limit is {limit}")
            }
            Self::SourceChanged => formatter.write_str("source changed during snapshot"),
            Self::TemporaryCopyFailed => formatter.write_str("stable snapshot copy failed"),
            Self::ArithmeticOverflow => formatter.write_str("snapshot arithmetic overflowed"),
        }
    }
}

impl std::error::Error for SourceSnapshotError {}

impl fmt::Display for SourceReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => error.fmt(formatter),
            Self::OutOfBounds {
                offset,
                length,
                source_length,
            } => write!(
                formatter,
                "read [{offset}, {}) exceeds source length {source_length}",
                offset.saturating_add(*length)
            ),
            Self::ReadLimit { limit, attempted } => {
                write!(
                    formatter,
                    "read budget {limit} exceeded by {attempted} bytes"
                )
            }
            Self::SourceChanged => formatter.write_str("source changed during read"),
            Self::ShortRead { expected } => {
                write!(formatter, "positioned read ended before {expected} bytes")
            }
            Self::HashMismatch => {
                formatter.write_str("source hash does not match expected content")
            }
            Self::Cancelled => formatter.write_str("source read was cancelled"),
            Self::ArithmeticOverflow => formatter.write_str("source read arithmetic overflowed"),
            Self::AllocationFailure => formatter.write_str("source read allocation failed"),
        }
    }
}

impl std::error::Error for SourceReadError {}

impl From<SourceSnapshotError> for SourceReadError {
    fn from(error: SourceSnapshotError) -> Self {
        Self::Io(error)
    }
}
