#![allow(clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use sha2::{Digest, Sha256};

use crate::CollisionPolicy;
use crate::queue::{CopyDestination, DestinationError};

const STAGING_DIR: &str = ".rusttable-staging";
const COMMIT_DIR: &str = ".rusttable-export-commits";
const OWNER_MARKER: &[u8] = b"rusttable-owned-staging-v1\n";
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_PATH_BYTES: usize = 4096;
const MAX_EXISTING_HASH_BYTES: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskSettings {
    pub collision: CollisionPolicy,
    pub allow_existing_symlink_root: bool,
    pub require_atomic_replace: bool,
}

impl Default for DiskSettings {
    fn default() -> Self {
        Self {
            collision: CollisionPolicy::Fail,
            allow_existing_symlink_root: false,
            require_atomic_replace: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct DiskCapabilities {
    pub case_sensitive: bool,
    pub unicode_normalization_stable: bool,
    pub atomic_file_replace: bool,
    pub atomic_directory_promotion: bool,
    pub file_sync: bool,
    pub directory_sync: bool,
    pub max_component_bytes: u32,
    pub max_path_bytes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMember {
    pub relative_path: String,
    pub bytes: Vec<u8>,
    pub artifact_hash: [u8; 32],
}

impl BundleMember {
    /// Creates a bundle member after validating its relative path.
    ///
    /// # Errors
    ///
    /// Returns [`DiskError::InvalidTarget`] when the path is absolute,
    /// traverses a parent, contains a separator unsafe for this contract, or
    /// exceeds the destination limits.
    pub fn new(
        relative_path: impl Into<String>,
        bytes: impl Into<Vec<u8>>,
        artifact_hash: [u8; 32],
    ) -> Result<Self, DiskError> {
        let relative_path = relative_path.into();
        validate_relative_path(&relative_path)?;
        Ok(Self {
            relative_path,
            bytes: bytes.into(),
            artifact_hash,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiskReceipt {
    pub logical_target: String,
    pub root_identity_sha256: [u8; 32],
    pub collision: CollisionPolicy,
    pub idempotency_key: String,
    pub idempotency_replayed: bool,
    pub artifact_sha256: [u8; 32],
    pub bytes: u64,
    pub atomic: bool,
    pub capabilities: DiskCapabilities,
}

#[derive(Debug)]
pub enum DiskError {
    InvalidRoot,
    InvalidTarget(&'static str),
    RootSymlink,
    ChildSymlink(PathBuf),
    Collision(PathBuf),
    IdempotencyConflict,
    UnsafeStaging,
    NotAtomic,
    EmptyBundle,
    DuplicateMember(String),
    Io {
        stage: &'static str,
        source: io::Error,
    },
}

impl fmt::Display for DiskError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "secure disk destination failed: {self:?}")
    }
}
impl std::error::Error for DiskError {}

#[derive(Debug)]
pub struct DiskDestination {
    root: PathBuf,
    root_identity_sha256: [u8; 32],
    settings: DiskSettings,
    capabilities: DiskCapabilities,
    reservation: &'static Mutex<()>,
}

impl DiskDestination {
    /// Opens a secure destination rooted at `root` and recovers owned staging.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be created or inspected, is not a
    /// directory, violates the symlink policy, or contains unsafe staging.
    pub fn new(root: impl Into<PathBuf>, settings: DiskSettings) -> Result<Self, DiskError> {
        let selected = root.into();
        if selected.as_os_str().is_empty() {
            return Err(DiskError::InvalidRoot);
        }
        if fs::symlink_metadata(&selected).is_ok_and(|value| value.file_type().is_symlink())
            && !settings.allow_existing_symlink_root
        {
            return Err(DiskError::RootSymlink);
        }
        fs::create_dir_all(&selected).map_err(|source| DiskError::Io {
            stage: "create root",
            source,
        })?;
        let root = fs::canonicalize(&selected).map_err(|source| DiskError::Io {
            stage: "canonicalize root",
            source,
        })?;
        if !root.is_dir() {
            return Err(DiskError::InvalidRoot);
        }
        let root_identity_sha256 = Sha256::digest(root.to_string_lossy().as_bytes()).into();
        fs::create_dir_all(root.join(STAGING_DIR)).map_err(|source| DiskError::Io {
            stage: "create staging",
            source,
        })?;
        fs::create_dir_all(root.join(COMMIT_DIR)).map_err(|source| DiskError::Io {
            stage: "create commit metadata",
            source,
        })?;
        let capabilities = probe_capabilities(&root);
        let destination = Self {
            root,
            root_identity_sha256,
            settings,
            capabilities,
            reservation: reservation(),
        };
        destination.recover()?;
        Ok(destination)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }
    #[must_use]
    pub const fn capabilities(&self) -> DiskCapabilities {
        self.capabilities
    }

    /// Commits one artifact through staging and an atomic rename.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid paths or keys, unsafe symlinks, collisions,
    /// failed staging, or an incomplete atomic publication.
    pub fn commit(
        &self,
        logical_target: &str,
        bytes: &[u8],
        artifact_hash: [u8; 32],
        idempotency_key: &str,
    ) -> Result<DiskReceipt, DiskError> {
        validate_relative_path(logical_target)?;
        validate_idempotency_key(idempotency_key)?;
        let _guard = self
            .reservation
            .lock()
            .map_err(|_| DiskError::UnsafeStaging)?;
        let target = self.safe_path(logical_target)?;
        self.ensure_parent(&target)?;
        if let Some(receipt) =
            self.replay_if_committed(&target, logical_target, artifact_hash, idempotency_key)?
        {
            return Ok(receipt);
        }
        let target = self.resolve_collision(target, artifact_hash)?;
        if self.settings.collision == CollisionPolicy::SkipIfSame
            && fs::symlink_metadata(&target).is_ok()
            && Self::existing_hash(&target)? == Some(artifact_hash)
        {
            return self.file_receipt(
                &target,
                logical_target,
                artifact_hash,
                idempotency_key,
                true,
            );
        }
        let stage = self.stage_file(bytes, idempotency_key)?;
        let atomic = Self::publish_file(&stage, &target)?;
        let receipt = self.write_commit_record(
            &target,
            logical_target,
            artifact_hash,
            idempotency_key,
            bytes.len() as u64,
            atomic,
        )?;
        Ok(receipt)
    }

    /// Commits an artifact and its sidecars as one promoted directory.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid members or keys, duplicate members,
    /// unsafe symlinks, collisions, failed staging, or incomplete promotion.
    pub fn commit_bundle(
        &self,
        logical_target: &str,
        members: &[BundleMember],
        idempotency_key: &str,
    ) -> Result<DiskReceipt, DiskError> {
        validate_relative_path(logical_target)?;
        validate_idempotency_key(idempotency_key)?;
        if members.is_empty() {
            return Err(DiskError::EmptyBundle);
        }
        let mut names = BTreeSet::new();
        for member in members {
            validate_relative_path(&member.relative_path)?;
            if !names.insert(member.relative_path.clone()) {
                return Err(DiskError::DuplicateMember(member.relative_path.clone()));
            }
        }
        let _guard = self
            .reservation
            .lock()
            .map_err(|_| DiskError::UnsafeStaging)?;
        let mut target = self.safe_path(logical_target)?;
        self.ensure_parent(&target)?;
        if target.exists() || fs::symlink_metadata(&target).is_ok() {
            if self.commit_marker_matches(&target, idempotency_key) {
                if Self::bundle_matches(&target, members)? {
                    return Ok(self.bundle_receipt(logical_target, members, idempotency_key, true));
                }
                return Err(DiskError::IdempotencyConflict);
            }
            match self.settings.collision {
                CollisionPolicy::UniqueSuffix => {
                    target = unique_path(&target);
                }
                CollisionPolicy::ReplaceExisting => return Err(DiskError::NotAtomic),
                CollisionPolicy::SkipIfSame
                | CollisionPolicy::CreateNew
                | CollisionPolicy::Fail
                | CollisionPolicy::VersionRevision => {
                    return Err(DiskError::Collision(target));
                }
            }
        }
        let stage = self.root.join(STAGING_DIR).join(format!(
            "bundle-{}",
            hex(&Sha256::digest(idempotency_key.as_bytes()).into())
        ));
        if stage.exists() {
            return Err(DiskError::UnsafeStaging);
        }
        fs::create_dir(&stage).map_err(|source| DiskError::Io {
            stage: "create bundle staging",
            source,
        })?;
        fs::write(stage.join(".owner"), OWNER_MARKER).map_err(|source| DiskError::Io {
            stage: "write bundle owner",
            source,
        })?;
        for member in members {
            let path = stage.join(&member.relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|source| DiskError::Io {
                    stage: "create bundle member parent",
                    source,
                })?;
            }
            write_new_file(&path, &member.bytes, "write bundle member")?;
        }
        let marker = commit_marker(
            idempotency_key,
            members.iter().map(|member| member.artifact_hash).collect(),
            members.iter().map(|member| member.bytes.len() as u64).sum(),
        );
        write_new_file(
            &stage.join(".rusttable-commit"),
            marker.as_bytes(),
            "write bundle marker",
        )?;
        fs::remove_file(stage.join(".owner")).map_err(|source| DiskError::Io {
            stage: "remove bundle owner",
            source,
        })?;
        sync_dir(&stage)?;
        fs::rename(&stage, &target).map_err(|source| DiskError::Io {
            stage: "promote bundle",
            source,
        })?;
        sync_dir(
            target
                .parent()
                .ok_or(DiskError::InvalidTarget("bundle parent"))?,
        )?;
        Ok(self.bundle_receipt(logical_target, members, idempotency_key, false))
    }

    /// Removes only staging owned by this destination contract.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when owned staging cannot be inspected or removed.
    pub fn recover(&self) -> Result<u32, DiskError> {
        let staging = self.root.join(STAGING_DIR);
        let mut removed = 0;
        for entry in fs::read_dir(&staging).map_err(|source| DiskError::Io {
            stage: "scan staging",
            source,
        })? {
            let entry = entry.map_err(|source| DiskError::Io {
                stage: "read staging",
                source,
            })?;
            let path = entry.path();
            if path.is_file() && fs::read(&path).ok().as_deref() == Some(OWNER_MARKER) {
                if let Some(name) = path.file_name().and_then(|name| name.to_str())
                    && let Some(staged_name) = name.strip_suffix(".owner")
                {
                    let staged = path.with_file_name(staged_name);
                    if staged.is_file() {
                        fs::remove_file(staged).map_err(|source| DiskError::Io {
                            stage: "remove abandoned staged file",
                            source,
                        })?;
                    }
                }
                fs::remove_file(path).map_err(|source| DiskError::Io {
                    stage: "remove abandoned staging",
                    source,
                })?;
                removed += 1;
            } else if path.is_dir()
                && fs::read(path.join(".owner")).ok().as_deref() == Some(OWNER_MARKER)
            {
                fs::remove_dir_all(path).map_err(|source| DiskError::Io {
                    stage: "remove abandoned bundle",
                    source,
                })?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn safe_path(&self, logical_target: &str) -> Result<PathBuf, DiskError> {
        let path = self.root.join(logical_target);
        let mut current = self.root.clone();
        for component in Path::new(logical_target).components() {
            if let Component::Normal(name) = component {
                current.push(name);
                if fs::symlink_metadata(&current).is_ok_and(|value| value.file_type().is_symlink())
                {
                    return Err(DiskError::ChildSymlink(current));
                }
            }
        }
        Ok(path)
    }

    fn ensure_parent(&self, target: &Path) -> Result<(), DiskError> {
        let parent = target.parent().ok_or(DiskError::InvalidTarget("parent"))?;
        let relative = parent
            .strip_prefix(&self.root)
            .map_err(|_| DiskError::InvalidTarget("parent containment"))?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(name) = component else {
                return Err(DiskError::InvalidTarget("parent component"));
            };
            current.push(name);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(DiskError::ChildSymlink(current));
                }
                Ok(metadata) if !metadata.is_dir() => {
                    return Err(DiskError::InvalidTarget("parent is not directory"));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|source| DiskError::Io {
                        stage: "create destination directory",
                        source,
                    })?;
                }
                Err(source) => {
                    return Err(DiskError::Io {
                        stage: "inspect destination directory",
                        source,
                    });
                }
            }
        }
        Ok(())
    }

    fn resolve_collision(&self, target: PathBuf, hash: [u8; 32]) -> Result<PathBuf, DiskError> {
        let exists = fs::symlink_metadata(&target).is_ok();
        match self.settings.collision {
            CollisionPolicy::CreateNew | CollisionPolicy::Fail if exists => {
                Err(DiskError::Collision(target))
            }
            CollisionPolicy::SkipIfSame if exists => {
                if Self::existing_hash(&target)? == Some(hash) {
                    Ok(target)
                } else {
                    Err(DiskError::Collision(target))
                }
            }
            CollisionPolicy::ReplaceExisting
                if exists
                    && self.settings.require_atomic_replace
                    && !self.capabilities.atomic_file_replace =>
            {
                Err(DiskError::NotAtomic)
            }
            CollisionPolicy::UniqueSuffix if exists => Ok(unique_path(&target)),
            CollisionPolicy::VersionRevision if exists => Err(DiskError::Collision(target)),
            _ => Ok(target),
        }
    }

    fn stage_file(&self, bytes: &[u8], idempotency_key: &str) -> Result<PathBuf, DiskError> {
        let digest = hex(&Sha256::digest(idempotency_key.as_bytes()).into());
        let path = self.root.join(STAGING_DIR).join(format!("file-{digest}"));
        if path.exists() {
            return Err(DiskError::UnsafeStaging);
        }
        write_new_file(&path, bytes, "write staging file")?;
        write_new_file(
            &self
                .root
                .join(STAGING_DIR)
                .join(format!("file-{digest}.owner")),
            OWNER_MARKER,
            "write staging owner",
        )?;
        sync_dir(&self.root.join(STAGING_DIR))?;
        Ok(path)
    }

    fn publish_file(stage: &Path, target: &Path) -> Result<bool, DiskError> {
        fs::rename(stage, target).map_err(|source| DiskError::Io {
            stage: "atomic publish",
            source,
        })?;
        let owner = stage.with_extension("owner");
        let _ = fs::remove_file(owner);
        if let Some(parent) = target.parent() {
            sync_dir(parent)?;
        }
        Ok(true)
    }

    fn write_commit_record(
        &self,
        target: &Path,
        logical: &str,
        hash: [u8; 32],
        key: &str,
        bytes: u64,
        atomic: bool,
    ) -> Result<DiskReceipt, DiskError> {
        let marker = commit_marker(key, vec![hash], bytes);
        let id = hex(&Sha256::digest(target.to_string_lossy().as_bytes()).into());
        let path = self.root.join(COMMIT_DIR).join(id);
        write_new_file(&path, marker.as_bytes(), "write commit record")?;
        sync_dir(&self.root.join(COMMIT_DIR))?;
        Ok(DiskReceipt {
            logical_target: logical.to_owned(),
            root_identity_sha256: self.root_identity_sha256,
            collision: self.settings.collision,
            idempotency_key: key.to_owned(),
            idempotency_replayed: false,
            artifact_sha256: hash,
            bytes,
            atomic,
            capabilities: self.capabilities,
        })
    }

    fn replay_if_committed(
        &self,
        target: &Path,
        logical: &str,
        hash: [u8; 32],
        key: &str,
    ) -> Result<Option<DiskReceipt>, DiskError> {
        if !target.exists() {
            return Ok(None);
        }
        if self.commit_marker_matches(target, key) && Self::existing_hash(target)? == Some(hash) {
            return Ok(Some(DiskReceipt {
                logical_target: logical.to_owned(),
                root_identity_sha256: self.root_identity_sha256,
                collision: self.settings.collision,
                idempotency_key: key.to_owned(),
                idempotency_replayed: true,
                artifact_sha256: hash,
                bytes: fs::metadata(target)
                    .map_err(|source| DiskError::Io {
                        stage: "stat committed target",
                        source,
                    })?
                    .len(),
                atomic: true,
                capabilities: self.capabilities,
            }));
        }
        Ok(None)
    }

    fn commit_marker_matches(&self, target: &Path, key: &str) -> bool {
        let marker = if target.is_dir() {
            target.join(".rusttable-commit")
        } else {
            self.root
                .join(COMMIT_DIR)
                .join(hex(
                    &Sha256::digest(target.to_string_lossy().as_bytes()).into()
                ))
        };
        fs::read_to_string(marker).is_ok_and(|value| {
            value
                .lines()
                .any(|line| line == format!("idempotency={key}"))
        })
    }

    fn existing_hash(target: &Path) -> Result<Option<[u8; 32]>, DiskError> {
        if !target.is_file() {
            return Ok(None);
        }
        let length = fs::metadata(target)
            .map_err(|source| DiskError::Io {
                stage: "stat existing target",
                source,
            })?
            .len();
        if length > MAX_EXISTING_HASH_BYTES {
            return Err(DiskError::InvalidTarget(
                "existing target exceeds hash limit",
            ));
        }
        let mut file = File::open(target).map_err(|source| DiskError::Io {
            stage: "hash existing target",
            source,
        })?;
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
        loop {
            let count = file.read(&mut buffer).map_err(|source| DiskError::Io {
                stage: "read existing target",
                source,
            })?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        Ok(Some(digest.finalize().into()))
    }

    fn file_receipt(
        &self,
        target: &Path,
        logical: &str,
        hash: [u8; 32],
        key: &str,
        replayed: bool,
    ) -> Result<DiskReceipt, DiskError> {
        Ok(DiskReceipt {
            logical_target: logical.to_owned(),
            root_identity_sha256: self.root_identity_sha256,
            collision: self.settings.collision,
            idempotency_key: key.to_owned(),
            idempotency_replayed: replayed,
            artifact_sha256: hash,
            bytes: fs::metadata(target)
                .map_err(|source| DiskError::Io {
                    stage: "stat committed target",
                    source,
                })?
                .len(),
            atomic: true,
            capabilities: self.capabilities,
        })
    }

    fn bundle_matches(target: &Path, members: &[BundleMember]) -> Result<bool, DiskError> {
        if !target.is_dir() {
            return Ok(false);
        }
        for member in members {
            let path = target.join(&member.relative_path);
            if !path_is_safe_child(target, &member.relative_path)
                || !path.is_file()
                || fs::metadata(&path)
                    .map_err(|source| DiskError::Io {
                        stage: "stat existing bundle member",
                        source,
                    })?
                    .len()
                    != member.bytes.len() as u64
                || Self::existing_hash(&path)? != Some(member.artifact_hash)
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    fn bundle_receipt(
        &self,
        logical: &str,
        members: &[BundleMember],
        key: &str,
        replayed: bool,
    ) -> DiskReceipt {
        let mut digest = Sha256::new();
        let mut bytes = 0;
        for member in members {
            digest.update(member.artifact_hash);
            bytes += member.bytes.len() as u64;
        }
        DiskReceipt {
            logical_target: logical.to_owned(),
            root_identity_sha256: self.root_identity_sha256,
            collision: self.settings.collision,
            idempotency_key: key.to_owned(),
            idempotency_replayed: replayed,
            artifact_sha256: digest.finalize().into(),
            bytes,
            atomic: true,
            capabilities: self.capabilities,
        }
    }
}

impl CopyDestination for DiskDestination {
    fn commit(
        &self,
        staging: &Path,
        logical_target: &str,
        idempotency_key: &str,
    ) -> Result<Vec<u8>, DestinationError> {
        let mut members = Vec::new();
        let entries = fs::read_dir(staging).map_err(|source| DestinationError::Io {
            stage: "read export staging",
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| DestinationError::Io {
                stage: "read export staging entry",
                source,
            })?;
            let path = entry.path();
            if !path.is_file() || entry.file_name() == ".rusttable-export-commit" {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = fs::read(&path).map_err(|source| DestinationError::Io {
                stage: "read staged export",
                source,
            })?;
            let hash = Sha256::digest(&bytes).into();
            members.push(
                BundleMember::new(name, bytes, hash)
                    .map_err(|_| DestinationError::InvalidTarget)?,
            );
        }
        if members.is_empty() {
            return Err(DestinationError::InvalidTarget);
        }
        let receipt = self
            .commit_bundle(logical_target, &members, idempotency_key)
            .map_err(destination_error)?;
        let manifest = format!(
            "schema=rusttable.disk-receipt.v1\nlogical_target={}\nidempotency={}\nbytes={}\nartifact_sha256={}\natomic={}\n",
            receipt.logical_target,
            receipt.idempotency_key,
            receipt.bytes,
            hex(&receipt.artifact_sha256),
            receipt.atomic,
        );
        Ok(manifest.into_bytes())
    }
}

fn destination_error(error: DiskError) -> DestinationError {
    match error {
        DiskError::InvalidTarget(_) | DiskError::ChildSymlink(_) | DiskError::RootSymlink => {
            DestinationError::InvalidTarget
        }
        DiskError::Collision(_) | DiskError::IdempotencyConflict => {
            DestinationError::CommitConflict
        }
        DiskError::Io { stage, source } => DestinationError::Io { stage, source },
        other => DestinationError::Io {
            stage: "disk destination",
            source: io::Error::other(other.to_string()),
        },
    }
}

fn reservation() -> &'static Mutex<()> {
    static RESERVATION: OnceLock<Mutex<()>> = OnceLock::new();
    RESERVATION.get_or_init(|| Mutex::new(()))
}

fn validate_relative_path(value: &str) -> Result<(), DiskError> {
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || value.contains('\0')
        || value.contains('\\')
        || Path::new(value).is_absolute()
    {
        return Err(DiskError::InvalidTarget("absolute or oversized path"));
    }
    for component in Path::new(value).components() {
        match component {
            Component::Normal(name) if !name.is_empty() && name.len() <= MAX_COMPONENT_BYTES => {}
            _ => return Err(DiskError::InvalidTarget("path component")),
        }
    }
    Ok(())
}

fn validate_idempotency_key(value: &str) -> Result<(), DiskError> {
    if value.is_empty()
        || value.len() > 256
        || value
            .bytes()
            .any(|byte| byte == 0 || byte == b'\n' || byte == b'\r')
    {
        return Err(DiskError::InvalidTarget("idempotency key"));
    }
    Ok(())
}

fn unique_path(path: &Path) -> PathBuf {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map_or_else(String::new, |value| format!(".{value}"));
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("export");
    for index in 1..=9999 {
        let candidate = path.with_file_name(format!("{stem}-{index:02}{extension}"));
        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }
    }
    path.with_file_name(format!("{stem}-9999{extension}"))
}

fn write_new_file(path: &Path, bytes: &[u8], stage: &'static str) -> Result<(), DiskError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| DiskError::Io { stage, source })?;
    file.write_all(bytes)
        .map_err(|source| DiskError::Io { stage, source })?;
    file.sync_all()
        .map_err(|source| DiskError::Io { stage, source })?;
    Ok(())
}

fn sync_dir(path: &Path) -> Result<(), DiskError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| DiskError::Io {
            stage: "sync directory",
            source,
        })
}

fn commit_marker(key: &str, hashes: Vec<[u8; 32]>, bytes: u64) -> String {
    format!(
        "schema=rusttable.disk-commit.v1\nidempotency={key}\nbytes={bytes}\nhashes={}\n",
        hashes
            .into_iter()
            .map(|hash| hex(&hash))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn hex(bytes: &[u8; 32]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 0x0f)] as char);
    }
    output
}

fn path_is_safe_child(root: &Path, relative: &str) -> bool {
    let mut current = root.to_owned();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            return false;
        };
        current.push(name);
        if fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return false;
        }
    }
    true
}

fn probe_capabilities(root: &Path) -> DiskCapabilities {
    let probe_dir = root.join(STAGING_DIR);
    let suffix = format!(
        "{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("main")
    );
    let probe = probe_dir.join(format!(".capability-probe-{suffix}"));
    let final_path = probe_dir.join(format!(".capability-probe-final-{suffix}"));
    let atomic_file_replace = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .is_ok()
        && fs::rename(&probe, &final_path).is_ok();
    let _ = fs::remove_file(final_path);
    DiskCapabilities {
        case_sensitive: cfg!(not(target_os = "windows")),
        unicode_normalization_stable: true,
        atomic_file_replace,
        atomic_directory_promotion: atomic_file_replace,
        file_sync: true,
        directory_sync: !cfg!(target_os = "windows"),
        max_component_bytes: 255,
        max_path_bytes: 4096,
    }
}
