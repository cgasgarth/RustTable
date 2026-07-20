use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use sha2::{Digest, Sha256};

use crate::{
    FileSourceSnapshotReader, ImportSourceLimits, SourceHashStatus, SourceIdentityClass,
    SourceSnapshot, SourceSnapshotError, SourceSnapshotReadError, SourceSnapshotReader,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const DEFAULT_CHUNK_BYTES: usize = 64 * 1024;

/// Configuration for copying a changing source into a private stable cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableCopyOptions {
    cache_directory: PathBuf,
    limits: ImportSourceLimits,
    chunk_bytes: usize,
}

impl StableCopyOptions {
    #[must_use]
    pub fn new(cache_directory: impl Into<PathBuf>, limits: ImportSourceLimits) -> Self {
        Self {
            cache_directory: cache_directory.into(),
            limits,
            chunk_bytes: DEFAULT_CHUNK_BYTES,
        }
    }

    /// # Errors
    ///
    /// Returns an error when `chunk_bytes` is zero.
    pub fn with_chunk_bytes(mut self, chunk_bytes: usize) -> Result<Self, StableCopyOptionsError> {
        if chunk_bytes == 0 {
            return Err(StableCopyOptionsError::ZeroChunkSize);
        }
        self.chunk_bytes = chunk_bytes;
        Ok(self)
    }

    #[must_use]
    pub fn cache_directory(&self) -> &Path {
        &self.cache_directory
    }

    #[must_use]
    pub const fn limits(&self) -> ImportSourceLimits {
        self.limits
    }

    #[must_use]
    pub const fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableCopyOptionsError {
    ZeroChunkSize,
}

impl fmt::Display for StableCopyOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroChunkSize => formatter.write_str("stable-copy chunk size must be non-zero"),
        }
    }
}

impl std::error::Error for StableCopyOptionsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StableCopyStage {
    CacheDirectory,
    CreateTemporary,
    ReadSource,
    WriteTemporary,
    SyncTemporary,
    Publish,
    OpenPublished,
    RevalidateSource,
    Cleanup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StableCopyError {
    Source(SourceSnapshotError),
    SourceRead(SourceSnapshotReadError),
    Io {
        stage: StableCopyStage,
        path: PathBuf,
    },
    SourceChanged {
        path: PathBuf,
    },
    CacheCollision {
        path: PathBuf,
    },
    Cleanup {
        path: PathBuf,
    },
}

impl fmt::Display for StableCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "stable source copy failed: {self:?}")
    }
}

impl std::error::Error for StableCopyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::SourceRead(error) => Some(error),
            Self::Io { .. }
            | Self::SourceChanged { .. }
            | Self::CacheCollision { .. }
            | Self::Cleanup { .. } => None,
        }
    }
}

/// Privacy-safe evidence for one published stable copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StableCopyReceipt {
    pub source_alias: String,
    pub identity_class: SourceIdentityClass,
    pub copy_identity_class: SourceIdentityClass,
    pub hash_status: SourceHashStatus,
    pub source_length: u64,
    pub source_sha256: [u8; 32],
    pub copy_length: u64,
    pub copy_sha256: [u8; 32],
    pub bytes_read: u64,
    pub elapsed_millis: u128,
}

#[derive(Debug, Clone)]
pub struct StableCopyResult {
    snapshot: SourceSnapshot,
    receipt: StableCopyReceipt,
}

impl StableCopyResult {
    #[must_use]
    pub const fn snapshot(&self) -> &SourceSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub const fn receipt(&self) -> &StableCopyReceipt {
        &self.receipt
    }

    #[must_use]
    pub fn into_parts(self) -> (SourceSnapshot, StableCopyReceipt) {
        (self.snapshot, self.receipt)
    }
}

impl FileSourceSnapshotReader {
    /// Copies a source through one opened snapshot and publishes it without
    /// replacing an existing cache entry. The resulting snapshot is reopened
    /// from the published cache file and independently validated.
    ///
    /// # Errors
    ///
    /// Returns a typed source, bounded-read, publication, or cleanup failure.
    pub fn read_stable_copy(
        &self,
        path: &Path,
        options: &StableCopyOptions,
    ) -> Result<StableCopyResult, StableCopyError> {
        let started = Instant::now();
        let source = self
            .read_snapshot(path, options.limits())
            .map_err(StableCopyError::Source)?;
        fs::create_dir_all(options.cache_directory()).map_err(|_| StableCopyError::Io {
            stage: StableCopyStage::CacheDirectory,
            path: options.cache_directory().to_owned(),
        })?;

        let temporary_path = temporary_path(options.cache_directory());
        let mut temporary = TemporaryFile::create(&temporary_path)?;
        let mut reader = source
            .open_reader(source.byte_length().get())
            .map_err(StableCopyError::SourceRead)?;
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(options.chunk_bytes())
            .map_err(|_| StableCopyError::Io {
                stage: StableCopyStage::CreateTemporary,
                path: temporary_path.clone(),
            })?;
        buffer.resize(options.chunk_bytes(), 0);

        let mut hasher = Sha256::new();
        let mut bytes_read = 0_u64;
        while reader.remaining() != 0 {
            let amount = buffer
                .len()
                .min(usize::try_from(reader.remaining()).unwrap_or(usize::MAX));
            let read = reader
                .read_checked(&mut buffer[..amount])
                .map_err(StableCopyError::SourceRead)?;
            if read == 0 {
                return Err(StableCopyError::SourceChanged {
                    path: path.to_owned(),
                });
            }
            temporary
                .file
                .write_all(&buffer[..read])
                .map_err(|_| StableCopyError::Io {
                    stage: StableCopyStage::WriteTemporary,
                    path: temporary_path.clone(),
                })?;
            hasher.update(&buffer[..read]);
            bytes_read = bytes_read
                .checked_add(u64::try_from(read).map_err(|_| {
                    StableCopyError::SourceRead(SourceSnapshotReadError::LengthConversion)
                })?)
                .ok_or(StableCopyError::SourceRead(
                    SourceSnapshotReadError::LengthConversion,
                ))?;
        }
        source
            .revalidate_opened_source()
            .map_err(|error| match error {
                SourceSnapshotError::SourceChanged { path } => {
                    StableCopyError::SourceChanged { path }
                }
                other => StableCopyError::Source(other),
            })?;
        let copy_sha256: [u8; 32] = hasher.finalize().into();
        if copy_sha256 != source.content_sha256() {
            return Err(StableCopyError::SourceChanged {
                path: path.to_owned(),
            });
        }

        temporary.file.sync_all().map_err(|_| StableCopyError::Io {
            stage: StableCopyStage::SyncTemporary,
            path: temporary_path.clone(),
        })?;
        let destination = options
            .cache_directory()
            .join(format!("rusttable-source-{}.bin", digest_hex(&copy_sha256)));
        let published = publish(
            &temporary,
            &destination,
            copy_sha256,
            *self,
            options.limits(),
        )?;
        let receipt = StableCopyReceipt {
            source_alias: source_alias(path),
            identity_class: source.identity_class(),
            copy_identity_class: published.identity_class(),
            hash_status: SourceHashStatus::Verified,
            source_length: source.byte_length().get(),
            source_sha256: source.content_sha256(),
            copy_length: published.byte_length().get(),
            copy_sha256: published.content_sha256(),
            bytes_read,
            elapsed_millis: started.elapsed().as_millis(),
        };
        Ok(StableCopyResult {
            snapshot: published,
            receipt,
        })
    }
}

fn publish(
    temporary: &TemporaryFile,
    destination: &Path,
    expected_digest: [u8; 32],
    reader: FileSourceSnapshotReader,
    limits: ImportSourceLimits,
) -> Result<SourceSnapshot, StableCopyError> {
    match fs::hard_link(temporary.path(), destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let existing = reader
                .read_snapshot(destination, limits)
                .map_err(StableCopyError::Source)?;
            if existing.content_sha256() != expected_digest {
                return Err(StableCopyError::CacheCollision {
                    path: destination.to_owned(),
                });
            }
            return Ok(existing);
        }
        Err(_) => {
            return Err(StableCopyError::Io {
                stage: StableCopyStage::Publish,
                path: destination.to_owned(),
            });
        }
    }
    reader
        .read_snapshot(destination, limits)
        .map_err(StableCopyError::Source)
}

struct TemporaryFile {
    file: File,
    path: PathBuf,
}

impl TemporaryFile {
    fn create(path: &Path) -> Result<Self, StableCopyError> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| StableCopyError::Io {
                stage: StableCopyStage::CreateTemporary,
                path: path.to_owned(),
            })?;
        Ok(Self {
            file,
            path: path.to_owned(),
        })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn temporary_path(directory: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    directory.join(format!(
        ".rusttable-source-{}-{sequence}.tmp",
        std::process::id()
    ))
}

fn source_alias(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            name.chars()
                .map(|character| {
                    if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                        character
                    } else {
                        '_'
                    }
                })
                .collect()
        })
        .filter(|alias: &String| !alias.is_empty())
        .unwrap_or_else(|| "source".to_owned())
}

fn digest_hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write as _;

    let mut value = String::with_capacity(64);
    for byte in digest {
        write!(&mut value, "{byte:02x}").expect("writing to a string cannot fail");
    }
    value
}
