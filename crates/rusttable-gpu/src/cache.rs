use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::PathBuf;

const CACHE_SCHEMA: u16 = 1;
const MAX_CACHE_ENTRY: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PipelineCacheIdentity {
    pub adapter: [u8; 32],
    pub shader: [u8; 32],
    pub layout: [u8; 32],
    pub rusttable_version: u16,
}

impl PipelineCacheIdentity {
    #[allow(clippy::missing_panics_doc)]
    #[must_use]
    pub fn key(&self) -> [u8; 32] {
        let bytes = postcard::to_allocvec(self).expect("cache identity is serializable");
        Sha256::digest(bytes).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheEnvelope {
    schema: u16,
    identity: PipelineCacheIdentity,
    payload: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheError {
    Io(String),
    Invalid(String),
    TooLarge,
}

impl fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "pipeline cache I/O failed: {error}"),
            Self::Invalid(error) => write!(formatter, "pipeline cache is invalid: {error}"),
            Self::TooLarge => formatter.write_str("pipeline cache entry is too large"),
        }
    }
}

impl std::error::Error for CacheError {}

#[derive(Debug, Clone)]
pub struct PipelineCacheStore {
    root: PathBuf,
    max_bytes: u64,
}

impl PipelineCacheStore {
    pub fn new(root: impl Into<PathBuf>, max_bytes: u64) -> Result<Self, CacheError> {
        if max_bytes == 0 || max_bytes > MAX_CACHE_ENTRY {
            return Err(CacheError::Invalid(
                "cache budget is outside the bounded range".to_owned(),
            ));
        }
        Ok(Self {
            root: root.into(),
            max_bytes,
        })
    }

    pub fn load(&self, identity: &PipelineCacheIdentity) -> Result<Option<Vec<u8>>, CacheError> {
        let path = self.path(identity);
        if !path.is_file() {
            return Ok(None);
        }
        let bytes = fs::read(&path).map_err(|error| CacheError::Io(error.to_string()))?;
        if bytes.len() as u64 > self.max_bytes {
            let _ = fs::remove_file(path);
            return Ok(None);
        }
        let envelope = postcard::from_bytes::<CacheEnvelope>(&bytes)
            .map_err(|error| CacheError::Invalid(error.to_string()))?;
        if envelope.schema != CACHE_SCHEMA || envelope.identity != *identity {
            let _ = fs::remove_file(self.path(identity));
            return Ok(None);
        }
        Ok(Some(envelope.payload))
    }

    pub fn save(&self, identity: &PipelineCacheIdentity, payload: &[u8]) -> Result<(), CacheError> {
        if payload.len() as u64 > self.max_bytes || payload.len() as u64 > MAX_CACHE_ENTRY {
            return Err(CacheError::TooLarge);
        }
        fs::create_dir_all(&self.root).map_err(|error| CacheError::Io(error.to_string()))?;
        let envelope = CacheEnvelope {
            schema: CACHE_SCHEMA,
            identity: identity.clone(),
            payload: payload.to_vec(),
        };
        let bytes = postcard::to_allocvec(&envelope)
            .map_err(|error| CacheError::Invalid(error.to_string()))?;
        if bytes.len() as u64 > self.max_bytes {
            return Err(CacheError::TooLarge);
        }
        let path = self.path(identity);
        let temporary = path.with_extension("tmp");
        {
            let mut file =
                fs::File::create(&temporary).map_err(|error| CacheError::Io(error.to_string()))?;
            std::io::Write::write_all(&mut file, &bytes)
                .map_err(|error| CacheError::Io(error.to_string()))?;
            file.sync_all()
                .map_err(|error| CacheError::Io(error.to_string()))?;
        }
        fs::rename(&temporary, &path).map_err(|error| CacheError::Io(error.to_string()))
    }

    pub fn evict_to_budget(&self) -> Result<usize, CacheError> {
        let mut entries = fs::read_dir(&self.root)
            .map_err(|error| CacheError::Io(error.to_string()))?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                (path.extension().and_then(|value| value.to_str()) == Some("cache"))
                    .then(|| {
                        let metadata = entry.metadata().ok()?;
                        Some((path, metadata.len(), metadata.modified().ok()?))
                    })
                    .flatten()
            })
            .collect::<Vec<_>>();
        let mut total = entries.iter().map(|entry| entry.1).sum::<u64>();
        if total <= self.max_bytes {
            return Ok(0);
        }
        entries.sort_by_key(|entry| entry.2);
        let mut removed = 0;
        for (path, size, _) in entries {
            fs::remove_file(path).map_err(|error| CacheError::Io(error.to_string()))?;
            total = total.saturating_sub(size);
            removed += 1;
            if total <= self.max_bytes {
                break;
            }
        }
        Ok(removed)
    }

    fn path(&self, identity: &PipelineCacheIdentity) -> PathBuf {
        self.root.join(hex(identity.key())).with_extension("cache")
    }
}

fn hex(bytes: [u8; 32]) -> String {
    let mut text = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> PipelineCacheIdentity {
        PipelineCacheIdentity {
            adapter: [1; 32],
            shader: [2; 32],
            layout: [3; 32],
            rusttable_version: 1,
        }
    }

    #[test]
    fn cache_round_trip_is_identity_bound_and_atomic() {
        let root = std::env::temp_dir().join(format!("rusttable-gpu-cache-{}", std::process::id()));
        let store = PipelineCacheStore::new(&root, 4096).expect("valid budget");
        let key = identity();
        store
            .save(&key, b"trusted only as a performance artifact")
            .expect("save");
        assert_eq!(
            store.load(&key).expect("load"),
            Some(b"trusted only as a performance artifact".to_vec())
        );
        let mut changed = key.clone();
        changed.shader[0] = 9;
        assert_eq!(store.load(&changed).expect("mismatch is a miss"), None);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn cache_rejects_oversized_payloads() {
        let store = PipelineCacheStore::new(std::env::temp_dir(), 8).expect("valid budget");
        assert_eq!(store.save(&identity(), &[0; 32]), Err(CacheError::TooLarge));
    }
}
