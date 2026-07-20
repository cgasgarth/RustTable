use std::fmt;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use rusttable_core::ByteLength;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSourceLimitsError {
    ZeroLimit,
    MaxPlusOneOverflow,
    NotRepresentable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImportSourceLimits {
    max_source_bytes: u64,
}

impl ImportSourceLimits {
    /// Creates a finite source cap whose extra-byte sentinel is representable.
    ///
    /// # Errors
    ///
    /// Returns a typed error when the limit is zero, cannot fit in memory, or
    /// cannot reserve its extra-byte sentinel.
    pub fn new(max_source_bytes: u64) -> Result<Self, ImportSourceLimitsError> {
        if max_source_bytes == 0 {
            return Err(ImportSourceLimitsError::ZeroLimit);
        }
        usize::try_from(max_source_bytes).map_err(|_| ImportSourceLimitsError::NotRepresentable)?;
        max_source_bytes
            .checked_add(1)
            .ok_or(ImportSourceLimitsError::MaxPlusOneOverflow)?;
        Ok(Self { max_source_bytes })
    }

    #[must_use]
    pub const fn max_source_bytes(self) -> u64 {
        self.max_source_bytes
    }
}

#[derive(Debug, Clone)]
pub struct SourceSnapshot {
    path: PathBuf,
    length: ByteLength,
    source: Arc<File>,
    state: SourceFileState,
}

impl SourceSnapshot {
    fn new(
        path: PathBuf,
        length: u64,
        source: Arc<File>,
        state: SourceFileState,
    ) -> Result<Self, SourceSnapshotError> {
        if length == 0 {
            return Err(SourceSnapshotError::EmptySource);
        }
        Ok(Self {
            path,
            length: ByteLength::from_bytes(length),
            source,
            state,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn byte_length(&self) -> ByteLength {
        self.length
    }

    /// Materializes this opened source into a bounded owned buffer for legacy
    /// byte-oriented decoder adapters.
    ///
    /// The snapshot itself never stores this buffer. Callers that can consume
    /// a reader should use [`Self::open_reader`] instead.
    ///
    /// # Errors
    ///
    /// Returns a typed limit, allocation, read, or source-change error.
    pub fn materialize(
        &self,
        limits: ImportSourceLimits,
    ) -> Result<Vec<u8>, SourceSnapshotReadError> {
        let length = self.byte_length().get();
        if length > limits.max_source_bytes() {
            return Err(SourceSnapshotReadError::MaterializationLimitExceeded {
                limit: limits.max_source_bytes(),
                source_length: length,
            });
        }
        let length =
            usize::try_from(length).map_err(|_| SourceSnapshotReadError::LengthConversion)?;
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(length).map_err(|_| {
            SourceSnapshotReadError::AllocationFailure {
                path: self.path.clone(),
            }
        })?;
        bytes.resize(length, 0);
        let mut reader = self.open_reader(self.byte_length().get())?;
        reader.read_exact_checked(&mut bytes)?;
        self.revalidate_opened_source()
            .map_err(SourceSnapshotReadError::from)?;
        Ok(bytes)
    }

    /// Reads an exact range from the file handle captured by this snapshot.
    ///
    /// The range is checked before any I/O. The opened handle is used instead
    /// of reopening the path, and the source is revalidated after the read.
    ///
    /// # Errors
    ///
    /// Returns a typed bounds, source-change, or file-read error. A changed
    /// source invalidates the read result even when the requested range was
    /// read successfully.
    pub fn read_exact_at(
        &self,
        offset: u64,
        buffer: &mut [u8],
    ) -> Result<(), SourceSnapshotReadError> {
        self.check_range(offset, buffer.len())?;
        if !buffer.is_empty() {
            read_exact_at_file(&self.source, offset, buffer).map_err(|_| {
                SourceSnapshotReadError::Io {
                    path: self.path.clone(),
                    offset,
                    length: buffer.len(),
                }
            })?;
        }
        self.revalidate_opened_source()
            .map_err(SourceSnapshotReadError::from)
    }

    /// Creates a reader with its own position and a finite read budget.
    ///
    /// Multiple readers may be created from one snapshot. They use positioned
    /// reads, so consuming one reader does not affect any other reader.
    ///
    /// # Errors
    ///
    /// Returns a bounds error when `max_bytes` exceeds this snapshot's length.
    pub fn open_reader(
        &self,
        max_bytes: u64,
    ) -> Result<SourceSnapshotSequentialReader, SourceSnapshotReadError> {
        if max_bytes > self.byte_length().get() {
            return Err(SourceSnapshotReadError::ReaderLimitExceedsSource {
                limit: max_bytes,
                source_length: self.byte_length().get(),
            });
        }
        Ok(SourceSnapshotSequentialReader {
            snapshot: self.clone(),
            position: 0,
            remaining: max_bytes,
        })
    }

    /// Revalidates the captured handle after source-backed work.
    ///
    /// This checks the captured handle's bounded identity metadata without
    /// loading the source. It is intentionally independent of the current
    /// path, so a path replacement cannot change the handle used by the
    /// snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed I/O or source-change error when the captured handle no
    /// longer matches the immutable bytes accepted at snapshot creation.
    pub fn revalidate_opened_source(&self) -> Result<(), SourceSnapshotError> {
        if stable_source_file_state(&self.source, &self.path)? != self.state {
            return Err(SourceSnapshotError::SourceChanged {
                path: self.path.clone(),
            });
        }
        Ok(())
    }

    fn check_range(&self, offset: u64, length: usize) -> Result<(), SourceSnapshotReadError> {
        let length =
            u64::try_from(length).map_err(|_| SourceSnapshotReadError::LengthConversion)?;
        let end = offset
            .checked_add(length)
            .ok_or(SourceSnapshotReadError::OffsetOverflow { offset, length })?;
        if end > self.byte_length().get() {
            return Err(SourceSnapshotReadError::OutOfBounds {
                offset,
                length,
                source_length: self.byte_length().get(),
            });
        }
        Ok(())
    }

    fn contents_equal(&self, other: &Self) -> Result<bool, SourceSnapshotError> {
        if self.byte_length() != other.byte_length() {
            return Ok(false);
        }
        let mut offset = 0_u64;
        let mut left = [0_u8; 8192];
        let mut right = [0_u8; 8192];
        while offset < self.byte_length().get() {
            let amount = usize::try_from(self.byte_length().get() - offset)
                .unwrap_or(left.len())
                .min(left.len());
            read_exact_at_file(&self.source, offset, &mut left[..amount]).map_err(|_| {
                SourceSnapshotError::Io {
                    stage: SourceReadStage::Read,
                    path: self.path.clone(),
                }
            })?;
            read_exact_at_file(&other.source, offset, &mut right[..amount]).map_err(|_| {
                SourceSnapshotError::Io {
                    stage: SourceReadStage::Read,
                    path: other.path.clone(),
                }
            })?;
            if left[..amount] != right[..amount] {
                return Ok(false);
            }
            offset = offset
                .checked_add(
                    u64::try_from(amount).map_err(|_| SourceSnapshotError::LengthConversion)?,
                )
                .ok_or(SourceSnapshotError::LengthConversion)?;
        }
        self.revalidate_opened_source()?;
        other.revalidate_opened_source()?;
        Ok(true)
    }
}

impl PartialEq for SourceSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path && self.length == other.length && self.state == other.state
    }
}

impl Eq for SourceSnapshot {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceFileState {
    length: u64,
    modified: Option<SystemTime>,
    content_digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSnapshotReadError {
    OutOfBounds {
        offset: u64,
        length: u64,
        source_length: u64,
    },
    OffsetOverflow {
        offset: u64,
        length: u64,
    },
    ReaderLimitExceedsSource {
        limit: u64,
        source_length: u64,
    },
    ReaderBudgetExceeded {
        requested: u64,
        remaining: u64,
    },
    MaterializationLimitExceeded {
        limit: u64,
        source_length: u64,
    },
    LengthConversion,
    AllocationFailure {
        path: PathBuf,
    },
    Io {
        path: PathBuf,
        offset: u64,
        length: usize,
    },
    SourceChanged {
        path: PathBuf,
    },
}

impl fmt::Display for SourceSnapshotReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "source snapshot read failed: {self:?}")
    }
}

impl std::error::Error for SourceSnapshotReadError {}

impl From<SourceSnapshotError> for SourceSnapshotReadError {
    fn from(error: SourceSnapshotError) -> Self {
        match error {
            SourceSnapshotError::SourceChanged { path } => Self::SourceChanged { path },
            SourceSnapshotError::Io { path, .. } => Self::Io {
                path,
                offset: 0,
                length: 0,
            },
            SourceSnapshotError::LengthConversion => Self::LengthConversion,
            other => Self::Io {
                path: other.path().map_or_else(PathBuf::new, Path::to_owned),
                offset: 0,
                length: 0,
            },
        }
    }
}

/// A source reader with an independent position and checked byte budget.
#[derive(Debug, Clone)]
pub struct SourceSnapshotSequentialReader {
    snapshot: SourceSnapshot,
    position: u64,
    remaining: u64,
}

impl SourceSnapshotSequentialReader {
    #[must_use]
    pub const fn position(&self) -> u64 {
        self.position
    }

    #[must_use]
    pub const fn remaining(&self) -> u64 {
        self.remaining
    }

    /// Reads at most the remaining bounded budget and revalidates afterward.
    ///
    /// # Errors
    ///
    /// Returns a typed source-change or positioned-read error.
    pub fn read_checked(&mut self, buffer: &mut [u8]) -> Result<usize, SourceSnapshotReadError> {
        if buffer.is_empty() || self.remaining == 0 {
            return Ok(0);
        }
        let amount = buffer
            .len()
            .min(usize::try_from(self.remaining).unwrap_or(usize::MAX));
        self.snapshot.check_range(self.position, amount)?;
        read_exact_at_file(&self.snapshot.source, self.position, &mut buffer[..amount]).map_err(
            |_| SourceSnapshotReadError::Io {
                path: self.snapshot.path.clone(),
                offset: self.position,
                length: amount,
            },
        )?;
        self.position = self
            .position
            .checked_add(
                u64::try_from(amount).map_err(|_| SourceSnapshotReadError::LengthConversion)?,
            )
            .ok_or(SourceSnapshotReadError::OffsetOverflow {
                offset: self.position,
                length: u64::try_from(amount).unwrap_or(u64::MAX),
            })?;
        self.remaining -=
            u64::try_from(amount).map_err(|_| SourceSnapshotReadError::LengthConversion)?;
        self.snapshot
            .revalidate_opened_source()
            .map_err(SourceSnapshotReadError::from)?;
        Ok(amount)
    }

    /// Reads a complete bounded buffer.
    ///
    /// # Errors
    ///
    /// Returns a checked bounds, source-change, or positioned-read error.
    pub fn read_exact_checked(&mut self, buffer: &mut [u8]) -> Result<(), SourceSnapshotReadError> {
        let length =
            u64::try_from(buffer.len()).map_err(|_| SourceSnapshotReadError::LengthConversion)?;
        if length > self.remaining {
            return Err(SourceSnapshotReadError::ReaderBudgetExceeded {
                requested: length,
                remaining: self.remaining,
            });
        }
        let mut filled = 0;
        while filled < buffer.len() {
            let amount = self.read_checked(&mut buffer[filled..])?;
            if amount == 0 {
                return Err(SourceSnapshotReadError::SourceChanged {
                    path: self.snapshot.path.clone(),
                });
            }
            filled += amount;
        }
        Ok(())
    }
}

impl std::io::Read for SourceSnapshotSequentialReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.read_checked(buffer)
            .map_err(|error| io::Error::other(error.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceReadStage {
    Open,
    Metadata,
    Read,
    Length,
    Allocation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSnapshotError {
    Io {
        stage: SourceReadStage,
        path: PathBuf,
    },
    NotRegularFile {
        path: PathBuf,
    },
    SymlinkRejected {
        path: PathBuf,
    },
    SourceChanged {
        path: PathBuf,
    },
    EmptySource,
    SourceTooLarge {
        path: PathBuf,
        limit: u64,
        actual: u64,
    },
    LengthConversion,
    MaxPlusOneOverflow,
    AllocationFailure {
        path: PathBuf,
    },
}

impl fmt::Display for SourceSnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "source snapshot failed: {self:?}")
    }
}

impl std::error::Error for SourceSnapshotError {}

impl SourceSnapshotError {
    fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. }
            | Self::NotRegularFile { path }
            | Self::SymlinkRejected { path }
            | Self::SourceChanged { path }
            | Self::SourceTooLarge { path, .. }
            | Self::AllocationFailure { path } => Some(path),
            Self::EmptySource | Self::LengthConversion | Self::MaxPlusOneOverflow => None,
        }
    }
}

pub trait SourceSnapshotReader: Send + Sync {
    /// Reads one bounded immutable source snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed source-access or limit failure without producing a
    /// partial snapshot.
    fn read_snapshot(
        &self,
        path: &Path,
        limits: ImportSourceLimits,
    ) -> Result<SourceSnapshot, SourceSnapshotError>;

    /// Reopens and compares the source with the immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns a typed source error when the path is no longer readable or its
    /// exact bytes changed after the snapshot was created.
    fn revalidate(
        &self,
        snapshot: &SourceSnapshot,
        limits: ImportSourceLimits,
    ) -> Result<(), SourceSnapshotError> {
        let current = self.read_snapshot(snapshot.path(), limits)?;
        if !snapshot.contents_equal(&current)? {
            return Err(SourceSnapshotError::SourceChanged {
                path: snapshot.path().to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FileSourceSnapshotReader;

impl SourceSnapshotReader for FileSourceSnapshotReader {
    fn read_snapshot(
        &self,
        path: &Path,
        limits: ImportSourceLimits,
    ) -> Result<SourceSnapshot, SourceSnapshotError> {
        let link_metadata =
            std::fs::symlink_metadata(path).map_err(|_| SourceSnapshotError::Io {
                stage: SourceReadStage::Open,
                path: path.to_owned(),
            })?;
        if link_metadata.file_type().is_symlink() {
            return Err(SourceSnapshotError::SymlinkRejected {
                path: path.to_owned(),
            });
        }
        let file = File::open(path).map_err(|_| SourceSnapshotError::Io {
            stage: SourceReadStage::Open,
            path: path.to_owned(),
        })?;
        let metadata = file.metadata().map_err(|_| SourceSnapshotError::Io {
            stage: SourceReadStage::Metadata,
            path: path.to_owned(),
        })?;
        if !metadata.is_file() {
            return Err(SourceSnapshotError::NotRegularFile {
                path: path.to_owned(),
            });
        }
        if metadata.len() > limits.max_source_bytes() {
            return Err(SourceSnapshotError::SourceTooLarge {
                path: path.to_owned(),
                limit: limits.max_source_bytes(),
                actual: metadata.len(),
            });
        }
        let source = Arc::new(file);
        let state = stable_source_file_state(&source, path)?;
        let snapshot = SourceSnapshot::new(path.to_owned(), metadata.len(), source, state)?;
        Ok(snapshot)
    }
}

fn source_file_state(file: &File, path: &Path) -> Result<SourceFileState, SourceSnapshotError> {
    let metadata = file.metadata().map_err(|_| SourceSnapshotError::Io {
        stage: SourceReadStage::Metadata,
        path: path.to_owned(),
    })?;
    Ok(SourceFileState {
        length: metadata.len(),
        modified: metadata.modified().ok(),
        content_digest: source_digest(file, metadata.len(), path)?,
    })
}

fn stable_source_file_state(
    file: &File,
    path: &Path,
) -> Result<SourceFileState, SourceSnapshotError> {
    let initial = source_file_state(file, path)?;
    if source_file_state(file, path)? != initial {
        return Err(SourceSnapshotError::SourceChanged {
            path: path.to_owned(),
        });
    }
    Ok(initial)
}

fn source_digest(file: &File, length: u64, path: &Path) -> Result<[u8; 32], SourceSnapshotError> {
    let mut digest = Sha256::new();
    let mut offset = 0_u64;
    let mut buffer = [0_u8; 8192];
    while offset < length {
        let amount = usize::try_from(length - offset)
            .unwrap_or(buffer.len())
            .min(buffer.len());
        read_exact_at_file(file, offset, &mut buffer[..amount]).map_err(|_| {
            SourceSnapshotError::Io {
                stage: SourceReadStage::Read,
                path: path.to_owned(),
            }
        })?;
        digest.update(&buffer[..amount]);
        offset = offset
            .checked_add(u64::try_from(amount).map_err(|_| SourceSnapshotError::LengthConversion)?)
            .ok_or(SourceSnapshotError::LengthConversion)?;
    }
    Ok(digest.finalize().into())
}

fn read_exact_at_file(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    let mut filled = 0_usize;
    while filled < buffer.len() {
        let read_offset = offset
            .checked_add(u64::try_from(filled).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "read offset is not representable",
                )
            })?)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "read offset overflow"))?;
        let amount = positioned_read(file, &mut buffer[filled..], read_offset)?;
        if amount == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "source ended before the requested range was read",
            ));
        }
        filled += amount;
    }
    Ok(())
}

#[cfg(unix)]
fn positioned_read(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, offset)
}

#[cfg(windows)]
fn positioned_read(file: &File, buffer: &mut [u8], offset: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(buffer, offset)
}

#[cfg(not(any(unix, windows)))]
fn positioned_read(_file: &File, _buffer: &mut [u8], _offset: u64) -> io::Result<usize> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "positioned source reads are unsupported on this platform",
    ))
}
