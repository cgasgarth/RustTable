use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    api::WorldVersion,
    errors::{ErrorCode, ScriptError},
    receipt::digest,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CacheKey {
    pub wasmtime: String,
    pub target: String,
    pub engine_config: String,
    pub component: String,
    pub world: WorldVersion,
    pub feature_policy: String,
}

impl CacheKey {
    #[must_use]
    pub fn stable_id(&self) -> String {
        digest(&serde_json::to_vec(self).unwrap_or_default())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct CacheReceipt {
    pub key: CacheKey,
    pub artifact: String,
    pub provenance: String,
    pub accepted: bool,
}

#[derive(Debug, Clone)]
pub struct CompiledArtifactCache {
    root: PathBuf,
}

impl CompiledArtifactCache {
    /// Opens or creates the RustTable-owned cache directory.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the directory cannot be created.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ScriptError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|_| {
            ScriptError::new(ErrorCode::CacheRejected, "cache directory is unavailable")
        })?;
        Ok(Self { root })
    }

    #[must_use]
    pub fn memory() -> Self {
        Self {
            root: std::env::temp_dir().join("rusttable-extension-cache"),
        }
    }

    /// Stores a Wasmtime-created serialized artifact with its provenance receipt.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the cache write fails.
    pub fn write(&self, key: CacheKey, artifact: &[u8]) -> Result<CacheReceipt, ScriptError> {
        let artifact_hash = digest(artifact);
        let name = format!("{}.bin", key.stable_id().trim_start_matches("sha256:"));
        let path = self.root.join(name);
        fs::write(&path, artifact).map_err(|_| {
            ScriptError::new(
                ErrorCode::CacheRejected,
                "compiled artifact cache write failed",
            )
        })?;
        Ok(CacheReceipt {
            key,
            artifact: artifact_hash,
            provenance: "rusttable-created".to_owned(),
            accepted: true,
        })
    }

    /// Reads an artifact only when its expected `RustTable` provenance hash matches.
    ///
    /// # Errors
    ///
    /// Returns [`ScriptError`] when the stored bytes fail provenance verification.
    pub fn read(
        &self,
        key: &CacheKey,
        expected_hash: &str,
    ) -> Result<Option<Vec<u8>>, ScriptError> {
        let path = self.path(key);
        let Ok(bytes) = fs::read(path) else {
            return Ok(None);
        };
        if digest(&bytes) != expected_hash {
            return Err(ScriptError::new(
                ErrorCode::CacheRejected,
                "compiled artifact provenance hash mismatch",
            ));
        }
        Ok(Some(bytes))
    }

    #[must_use]
    pub fn path(&self, key: &CacheKey) -> PathBuf {
        self.root.join(format!(
            "{}.bin",
            key.stable_id().trim_start_matches("sha256:")
        ))
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
}
