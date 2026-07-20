use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use rusttable_core::ByteLength;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    path: PathBuf,
    bytes: Vec<u8>,
    length: ByteLength,
}

impl SourceSnapshot {
    fn new(path: PathBuf, bytes: Vec<u8>) -> Result<Self, SourceSnapshotError> {
        let length =
            u64::try_from(bytes.len()).map_err(|_| SourceSnapshotError::LengthConversion)?;
        if length == 0 {
            return Err(SourceSnapshotError::EmptySource);
        }
        Ok(Self {
            path,
            bytes,
            length: ByteLength::from_bytes(length),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub const fn byte_length(&self) -> ByteLength {
        self.length
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
        if current.bytes() != snapshot.bytes() {
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
        let read_limit = limits
            .max_source_bytes()
            .checked_add(1)
            .ok_or(SourceSnapshotError::MaxPlusOneOverflow)?;
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
        let mut file = File::open(path).map_err(|_| SourceSnapshotError::Io {
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
        let reserve = usize::try_from(metadata.len().min(read_limit))
            .map_err(|_| SourceSnapshotError::LengthConversion)?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(reserve)
            .map_err(|_| SourceSnapshotError::AllocationFailure {
                path: path.to_owned(),
            })?;
        file.by_ref()
            .take(read_limit)
            .read_to_end(&mut bytes)
            .map_err(|_| SourceSnapshotError::Io {
                stage: SourceReadStage::Read,
                path: path.to_owned(),
            })?;
        let actual =
            u64::try_from(bytes.len()).map_err(|_| SourceSnapshotError::LengthConversion)?;
        if actual > limits.max_source_bytes() {
            return Err(SourceSnapshotError::SourceTooLarge {
                path: path.to_owned(),
                limit: limits.max_source_bytes(),
                actual,
            });
        }
        SourceSnapshot::new(path.to_owned(), bytes)
    }
}
