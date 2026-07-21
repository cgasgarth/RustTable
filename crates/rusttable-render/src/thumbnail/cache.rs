#![allow(
    clippy::format_collect,
    clippy::manual_let_else,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::single_match_else,
    clippy::struct_field_names
)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusttable_image::{ColorEncoding, DecodedImage, ImageDimensions};
use sha2::{Digest, Sha256};

use crate::{ThumbnailKey, ThumbnailKeyError};

pub const CURRENT_CACHE_SCHEMA: u16 = 1;
const MAGIC: &[u8; 8] = b"RTTHMB01";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CacheTime(u64);

impl CacheTime {
    #[must_use]
    pub const fn from_seconds(value: u64) -> Self {
        Self(value)
    }

    pub fn now() -> Result<Self, CacheError> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| Self(value.as_secs()))
            .map_err(|_| CacheError::Clock)
    }

    #[must_use]
    pub const fn seconds(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLimits {
    max_total_bytes: u64,
    max_entry_bytes: u64,
    max_age_seconds: Option<u64>,
}

impl CacheLimits {
    pub const fn new(max_total_bytes: u64, max_entry_bytes: u64) -> Result<Self, CacheError> {
        if max_total_bytes == 0 || max_entry_bytes == 0 {
            return Err(CacheError::InvalidLimits);
        }
        Ok(Self {
            max_total_bytes,
            max_entry_bytes,
            max_age_seconds: None,
        })
    }

    #[must_use]
    pub const fn with_max_age(mut self, seconds: u64) -> Self {
        self.max_age_seconds = Some(seconds);
        self
    }
    #[must_use]
    pub const fn max_total_bytes(self) -> u64 {
        self.max_total_bytes
    }
    #[must_use]
    pub const fn max_entry_bytes(self) -> u64 {
        self.max_entry_bytes
    }
    #[must_use]
    pub const fn max_age_seconds(self) -> Option<u64> {
        self.max_age_seconds
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheEntry {
    key: ThumbnailKey,
    image: DecodedImage,
    created_at: CacheTime,
}

impl CacheEntry {
    #[must_use]
    pub const fn key(&self) -> ThumbnailKey {
        self.key
    }
    #[must_use]
    pub const fn image(&self) -> &DecodedImage {
        &self.image
    }
    #[must_use]
    pub const fn created_at(&self) -> CacheTime {
        self.created_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReconciliationReport {
    pub valid_entries: usize,
    pub removed_corrupt: usize,
    pub removed_temporary: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    Io {
        operation: &'static str,
        path: PathBuf,
        message: String,
    },
    Clock,
    InvalidLimits,
    EntryTooLarge {
        bytes: u64,
        limit: u64,
    },
    CacheFull {
        required: u64,
        available: u64,
    },
    Protected,
    CorruptEntry,
    UnsupportedSchema(u16),
    Key(ThumbnailKeyError),
    Image,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "thumbnail cache error: {self:?}")
    }
}
impl std::error::Error for CacheError {}

#[derive(Debug, Clone)]
struct CacheMetadata {
    key: ThumbnailKey,
    path: PathBuf,
    bytes: u64,
    created_at: CacheTime,
    last_access: CacheTime,
}

#[derive(Debug)]
pub struct CacheStore {
    root: PathBuf,
    limits: CacheLimits,
    entries: BTreeMap<[u8; 32], CacheMetadata>,
    leases: BTreeMap<[u8; 32], u32>,
    pins: BTreeMap<[u8; 32], u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheLease {
    digest: [u8; 32],
}

impl CacheLease {
    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachePin {
    digest: [u8; 32],
}

impl CachePin {
    #[must_use]
    pub const fn digest(self) -> [u8; 32] {
        self.digest
    }
}

impl CacheStore {
    /// Opens a cache directory and reconstructs its minimal index from headers.
    pub fn open(
        root: impl Into<PathBuf>,
        limits: CacheLimits,
    ) -> Result<(Self, ReconciliationReport), CacheError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|error| io_error("create cache directory", &root, error))?;
        let mut store = Self {
            root,
            limits,
            entries: BTreeMap::new(),
            leases: BTreeMap::new(),
            pins: BTreeMap::new(),
        };
        let report = store.reconcile()?;
        Ok((store, report))
    }

    #[must_use]
    pub const fn limits(&self) -> CacheLimits {
        self.limits
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn keys(&self) -> impl Iterator<Item = ThumbnailKey> + '_ {
        self.entries.values().map(|entry| entry.key)
    }

    /// Reads and independently verifies the header, key, dimensions, length, and payload digest.
    pub fn get(
        &mut self,
        key: ThumbnailKey,
        now: CacheTime,
    ) -> Result<Option<CacheEntry>, CacheError> {
        let digest = key.digest();
        let Some(metadata) = self.entries.get_mut(&digest) else {
            return Ok(None);
        };
        let path = metadata.path.clone();
        match read_entry(&path, key, self.limits) {
            Ok(entry) => {
                metadata.last_access = now;
                Ok(Some(entry))
            }
            Err(CacheError::CorruptEntry | CacheError::UnsupportedSchema(_)) => {
                self.remove_digest(digest)?;
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    /// Publishes a complete entry using a same-directory staged file and atomic rename.
    pub fn put(
        &mut self,
        key: ThumbnailKey,
        image: &DecodedImage,
        created_at: CacheTime,
    ) -> Result<(), CacheError> {
        let payload = image.pixels();
        let key_bytes = key.canonical_bytes();
        let file_bytes = serialized_size(key_bytes.len(), payload.len())?;
        if file_bytes > self.limits.max_entry_bytes {
            return Err(CacheError::EntryTooLarge {
                bytes: file_bytes,
                limit: self.limits.max_entry_bytes,
            });
        }
        let digest = key.digest();
        let previous = self.entries.get(&digest).map_or(0, |entry| entry.bytes);
        self.evict_to_fit(file_bytes.saturating_sub(previous), created_at)?;
        let available = self
            .limits
            .max_total_bytes
            .saturating_sub(self.total_bytes().saturating_sub(previous));
        if file_bytes > available {
            return Err(CacheError::CacheFull {
                required: file_bytes,
                available,
            });
        }
        let final_path = self.path_for(digest);
        let temporary_path = self.root.join(format!(".{}.tmp", key.digest_hex()));
        let result = write_entry(&temporary_path, key, image, created_at);
        if result.is_ok() {
            fs::rename(&temporary_path, &final_path)
                .map_err(|error| io_error("publish cache entry", &final_path, error))?;
        } else {
            let _ = fs::remove_file(&temporary_path);
        }
        result?;
        self.entries.insert(
            digest,
            CacheMetadata {
                key,
                path: final_path,
                bytes: file_bytes,
                created_at,
                last_access: created_at,
            },
        );
        Ok(())
    }

    /// Removes one exact key. A missing key is a successful no-op.
    pub fn invalidate(&mut self, key: ThumbnailKey) -> Result<bool, CacheError> {
        if self.is_protected(key.digest()) {
            return Err(CacheError::Protected);
        }
        self.remove_digest(key.digest())
    }

    /// Acquires a lease that prevents eviction or invalidation until released.
    pub fn lease(&mut self, key: ThumbnailKey) -> Result<Option<CacheLease>, CacheError> {
        let digest = key.digest();
        if !self.entries.contains_key(&digest) {
            return Ok(None);
        }
        *self.leases.entry(digest).or_default() += 1;
        Ok(Some(CacheLease { digest }))
    }

    pub fn release_lease(&mut self, lease: CacheLease) -> Result<(), CacheError> {
        decrement(&mut self.leases, lease.digest)
    }

    /// Acquires a durable-session pin that survives ordinary eviction passes.
    pub fn pin(&mut self, key: ThumbnailKey) -> Result<Option<CachePin>, CacheError> {
        let digest = key.digest();
        if !self.entries.contains_key(&digest) {
            return Ok(None);
        }
        *self.pins.entry(digest).or_default() += 1;
        Ok(Some(CachePin { digest }))
    }

    pub fn unpin(&mut self, pin: CachePin) -> Result<(), CacheError> {
        decrement(&mut self.pins, pin.digest)
    }

    /// Removes expired entries and least-recently-used entries until the budget holds.
    pub fn evict(&mut self, now: CacheTime) -> Result<usize, CacheError> {
        let before = self.entries.len();
        if let Some(age) = self.limits.max_age_seconds {
            let expired = self
                .entries
                .iter()
                .filter_map(|(digest, metadata)| {
                    (now.seconds().saturating_sub(metadata.created_at.seconds()) > age)
                        .then_some(*digest)
                })
                .collect::<Vec<_>>();
            for digest in expired {
                self.remove_digest(digest)?;
            }
        }
        self.evict_to_fit(0, now)?;
        Ok(before - self.entries.len())
    }

    fn evict_to_fit(&mut self, additional: u64, _now: CacheTime) -> Result<(), CacheError> {
        while self.total_bytes().saturating_add(additional) > self.limits.max_total_bytes {
            let Some(digest) = self
                .entries
                .iter()
                .filter(|(digest, _)| !self.is_protected(**digest))
                .min_by_key(|(digest, metadata)| (metadata.last_access, **digest))
                .map(|(digest, _)| *digest)
            else {
                break;
            };
            self.remove_digest(digest)?;
        }
        Ok(())
    }

    fn total_bytes(&self) -> u64 {
        self.entries.values().map(|entry| entry.bytes).sum()
    }

    fn is_protected(&self, digest: [u8; 32]) -> bool {
        self.leases.get(&digest).copied().unwrap_or(0) > 0
            || self.pins.get(&digest).copied().unwrap_or(0) > 0
    }

    fn path_for(&self, digest: [u8; 32]) -> PathBuf {
        self.root.join(format!(
            "{}.mip",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        ))
    }

    fn remove_digest(&mut self, digest: [u8; 32]) -> Result<bool, CacheError> {
        let Some(metadata) = self.entries.remove(&digest) else {
            return Ok(false);
        };
        self.leases.remove(&digest);
        self.pins.remove(&digest);
        match fs::remove_file(&metadata.path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
            Err(error) => Err(io_error("remove cache entry", &metadata.path, error)),
        }
    }

    fn reconcile(&mut self) -> Result<ReconciliationReport, CacheError> {
        let mut report = ReconciliationReport::default();
        let directory = fs::read_dir(&self.root)
            .map_err(|error| io_error("read cache directory", &self.root, error))?;
        for item in directory {
            let item =
                item.map_err(|error| io_error("read cache directory entry", &self.root, error))?;
            let path = item.path();
            if path.extension().is_some_and(|extension| extension == "tmp") {
                fs::remove_file(&path)
                    .map_err(|error| io_error("remove temporary cache entry", &path, error))?;
                report.removed_temporary += 1;
                continue;
            }
            if path.extension().is_none_or(|extension| extension != "mip") {
                continue;
            }
            match read_header(&path, self.limits) {
                Ok(header) => {
                    let key = match ThumbnailKey::from_canonical_bytes(&header.key_bytes) {
                        Ok(key) => key,
                        Err(_) => {
                            fs::remove_file(&path).map_err(|error| {
                                io_error("remove invalid cache key", &path, error)
                            })?;
                            report.removed_corrupt += 1;
                            continue;
                        }
                    };
                    self.entries.insert(
                        header.digest,
                        CacheMetadata {
                            key,
                            path,
                            bytes: header.file_bytes,
                            created_at: header.created_at,
                            last_access: header.created_at,
                        },
                    );
                    report.valid_entries += 1;
                }
                Err(CacheError::CorruptEntry | CacheError::UnsupportedSchema(_)) => {
                    fs::remove_file(&path)
                        .map_err(|error| io_error("remove corrupt cache entry", &path, error))?;
                    report.removed_corrupt += 1;
                }
                Err(error) => return Err(error),
            }
        }
        self.evict_to_fit(0, CacheTime::from_seconds(0))?;
        Ok(report)
    }
}

fn decrement(counts: &mut BTreeMap<[u8; 32], u32>, digest: [u8; 32]) -> Result<(), CacheError> {
    let Some(count) = counts.get_mut(&digest) else {
        return Err(CacheError::Protected);
    };
    if *count == 1 {
        counts.remove(&digest);
    } else {
        *count -= 1;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct Header {
    digest: [u8; 32],
    created_at: CacheTime,
    key_bytes: Vec<u8>,
    width: u32,
    height: u32,
    payload_len: u64,
    payload_digest: [u8; 32],
    file_bytes: u64,
}

fn write_entry(
    path: &Path,
    key: ThumbnailKey,
    image: &DecodedImage,
    created_at: CacheTime,
) -> Result<(), CacheError> {
    let key_bytes = key.canonical_bytes();
    let payload = image.pixels();
    let payload_digest: [u8; 32] = Sha256::digest(payload).into();
    let mut header = Vec::with_capacity(8 + 2 + 4 + 8 + 4 + 4 + 8 + 32 + 32 + key_bytes.len() + 32);
    header.extend_from_slice(MAGIC);
    header.extend_from_slice(&CURRENT_CACHE_SCHEMA.to_be_bytes());
    header.extend_from_slice(
        &(u32::try_from(key_bytes.len()).map_err(|_| CacheError::CorruptEntry)?).to_be_bytes(),
    );
    header.extend_from_slice(&created_at.seconds().to_be_bytes());
    header.extend_from_slice(&image.dimensions().width().to_be_bytes());
    header.extend_from_slice(&image.dimensions().height().to_be_bytes());
    header.extend_from_slice(
        &(u64::try_from(payload.len()).map_err(|_| CacheError::CorruptEntry)?).to_be_bytes(),
    );
    header.extend_from_slice(&key.digest());
    header.extend_from_slice(&payload_digest);
    header.extend_from_slice(&key_bytes);
    let header_digest: [u8; 32] = Sha256::digest(&header).into();
    header.extend_from_slice(&header_digest);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| io_error("stage cache entry", path, error))?;
    let result = file
        .write_all(&header)
        .and_then(|()| file.write_all(payload))
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("write cache entry", path, error));
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn read_entry(
    path: &Path,
    key: ThumbnailKey,
    limits: CacheLimits,
) -> Result<CacheEntry, CacheError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| io_error("open cache entry", path, error))?
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read cache entry", path, error))?;
    let header = parse_header(&bytes, limits)?;
    if header.digest != key.digest() || header.key_bytes != key.canonical_bytes() {
        return Err(CacheError::CorruptEntry);
    }
    let payload_start = bytes
        .len()
        .checked_sub(usize::try_from(header.payload_len).map_err(|_| CacheError::CorruptEntry)?)
        .ok_or(CacheError::CorruptEntry)?;
    let payload = &bytes[payload_start..];
    if Sha256::digest(payload).as_slice() != header.payload_digest {
        return Err(CacheError::CorruptEntry);
    }
    let dimensions =
        ImageDimensions::new(header.width, header.height).map_err(|_| CacheError::CorruptEntry)?;
    let image =
        DecodedImage::new_with_color_encoding(dimensions, payload.to_vec(), ColorEncoding::Srgb)
            .map_err(|_| CacheError::Image)?;
    Ok(CacheEntry {
        key,
        image,
        created_at: header.created_at,
    })
}

fn read_header(path: &Path, limits: CacheLimits) -> Result<Header, CacheError> {
    let mut bytes = Vec::new();
    File::open(path)
        .map_err(|error| io_error("open cache entry", path, error))?
        .read_to_end(&mut bytes)
        .map_err(|error| io_error("read cache entry", path, error))?;
    parse_header(&bytes, limits)
}

fn parse_header(bytes: &[u8], limits: CacheLimits) -> Result<Header, CacheError> {
    if u64::try_from(bytes.len()).map_err(|_| CacheError::CorruptEntry)? > limits.max_entry_bytes {
        return Err(CacheError::CorruptEntry);
    }
    let mut reader = Reader { bytes, offset: 0 };
    if reader.take(8)? != MAGIC {
        return Err(CacheError::CorruptEntry);
    }
    let schema = reader.u16()?;
    if schema != CURRENT_CACHE_SCHEMA {
        return Err(CacheError::UnsupportedSchema(schema));
    }
    let key_len = usize::try_from(reader.u32()?).map_err(|_| CacheError::CorruptEntry)?;
    let created_at = CacheTime::from_seconds(reader.u64()?);
    let width = reader.u32()?;
    let height = reader.u32()?;
    let payload_len = reader.u64()?;
    let digest = reader.array::<32>()?;
    let payload_digest = reader.array::<32>()?;
    let key_bytes = reader.take(key_len)?.to_vec();
    let header_end = reader.offset;
    let stored_header_digest = reader.array::<32>()?;
    if Sha256::digest(&bytes[..header_end]).as_slice() != stored_header_digest {
        return Err(CacheError::CorruptEntry);
    }
    let payload_start = header_end.checked_add(32).ok_or(CacheError::CorruptEntry)?;
    let expected_end = payload_start
        .checked_add(usize::try_from(payload_len).map_err(|_| CacheError::CorruptEntry)?)
        .ok_or(CacheError::CorruptEntry)?;
    if expected_end != bytes.len() {
        return Err(CacheError::CorruptEntry);
    }
    let payload = &bytes[payload_start..expected_end];
    if Sha256::digest(payload).as_slice() != payload_digest {
        return Err(CacheError::CorruptEntry);
    }
    Ok(Header {
        digest,
        created_at,
        key_bytes,
        width,
        height,
        payload_len,
        payload_digest,
        file_bytes: u64::try_from(bytes.len()).map_err(|_| CacheError::CorruptEntry)?,
    })
}

fn serialized_size(key_len: usize, payload_len: usize) -> Result<u64, CacheError> {
    u64::try_from(
        8usize
            .checked_add(2)
            .and_then(|value| value.checked_add(4))
            .and_then(|value| {
                value.checked_add(8 + 4 + 4 + 8 + 32 + 32 + key_len + 32 + payload_len)
            })
            .ok_or(CacheError::CorruptEntry)?,
    )
    .map_err(|_| CacheError::CorruptEntry)
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], CacheError> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(CacheError::CorruptEntry)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(CacheError::CorruptEntry)?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, CacheError> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn u32(&mut self) -> Result<u32, CacheError> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn u64(&mut self) -> Result<u64, CacheError> {
        Ok(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| CacheError::CorruptEntry)?,
        ))
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], CacheError> {
        self.take(N)?
            .try_into()
            .map_err(|_| CacheError::CorruptEntry)
    }
}

fn io_error(operation: &'static str, path: &Path, error: std::io::Error) -> CacheError {
    CacheError::Io {
        operation,
        path: path.to_owned(),
        message: error.to_string(),
    }
}
