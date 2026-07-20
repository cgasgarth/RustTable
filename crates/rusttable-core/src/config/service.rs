use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, mpsc};

use super::model::{Configuration, GpuMode};
use super::parse::{
    ConfigError, ConfigFinding, ConfigFindingKind, EnvironmentOverrides, IoStage, OverrideValue,
    UnknownFields, canonical_document, document_hash, merge_unknown, resolve_layered,
};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConfigRevision(String);

impl ConfigRevision {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct LoadReport {
    pub snapshot: Arc<ConfigSnapshot>,
    pub findings: Vec<ConfigFinding>,
}

/// Immutable, shareable configuration state. The effective configuration contains all layers;
/// `persisted_configuration` deliberately excludes environment and startup overrides.
#[derive(Debug, Clone)]
pub struct ConfigSnapshot {
    pub configuration: Configuration,
    pub persisted_configuration: Configuration,
    pub resolved: ResolvedConfiguration,
    pub revision: ConfigRevision,
    pub unknown: UnknownFields,
    source_path: PathBuf,
    loaded_from_disk: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeBackend {
    Cpu,
    Gpu,
    CpuFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedConfiguration {
    pub cpu_threads: u16,
    pub host_memory_mib: u32,
    pub compute_backend: ComputeBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeCapabilities {
    pub physical_memory_mib: Option<u32>,
    pub qualified_gpu: bool,
}

#[derive(Debug, Clone)]
pub struct ConfigChange {
    pub revision: ConfigRevision,
    pub changed_fields: Vec<String>,
    pub snapshot: Arc<ConfigSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveResult {
    pub revision: ConfigRevision,
}

pub struct ConfigurationService {
    path: PathBuf,
    catalog_default: Option<PathBuf>,
    capabilities: RuntimeCapabilities,
    active: RwLock<Option<Arc<ConfigSnapshot>>>,
    subscribers: Mutex<Vec<mpsc::SyncSender<ConfigChange>>>,
    write_guard: Mutex<()>,
}

impl ConfigurationService {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            catalog_default: None,
            capabilities: RuntimeCapabilities::default(),
            active: RwLock::new(None),
            subscribers: Mutex::new(Vec::new()),
            write_guard: Mutex::new(()),
        }
    }

    /// Supplies the platform-owned catalog location without making `rusttable-core` resolve paths.
    #[must_use]
    pub fn with_catalog_default(mut self, path: impl Into<PathBuf>) -> Self {
        self.catalog_default = Some(path.into());
        self
    }

    #[must_use]
    pub fn with_runtime_capabilities(mut self, capabilities: RuntimeCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Loads defaults and all configured layers from disk and process state.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when the candidate cannot be safely parsed or validated.
    pub fn load_initial(&self) -> Result<LoadReport, ConfigError> {
        let environment = EnvironmentOverrides::from_pairs(std::env::vars())?;
        let path = environment
            .config_file
            .as_deref()
            .map_or_else(|| self.path.clone(), PathBuf::from);
        self.load_path(&path, None, &environment, &BTreeMap::new())
    }

    /// Loads an explicitly supplied candidate and override set. This is the deterministic test and
    /// bootstrap entry point; `RUSTTABLE_CONFIG_FILE` is not consulted for an explicit candidate.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] when schema, bounds, or validation fail.
    pub fn load_from(
        &self,
        user_text: Option<&str>,
        environment: &EnvironmentOverrides,
        startup: &BTreeMap<String, OverrideValue>,
    ) -> Result<LoadReport, ConfigError> {
        self.load_path(&self.path, user_text, environment, startup)
    }

    fn load_path(
        &self,
        path: &Path,
        user_text: Option<&str>,
        environment: &EnvironmentOverrides,
        startup: &BTreeMap<String, OverrideValue>,
    ) -> Result<LoadReport, ConfigError> {
        let text = match user_text {
            Some(text) => Some(text.to_owned()),
            None => read_optional(path)?,
        };
        let loaded_from_disk = user_text.is_none() && text.is_some();
        let mut layers = resolve_layered(text.as_deref(), environment, startup)?;
        apply_catalog_default(
            &mut layers.persisted,
            &mut layers.effective,
            self.catalog_default.as_deref(),
            environment,
            startup,
        );
        let resolved = resolve_runtime(&layers.effective, self.capabilities, &mut layers.findings);
        let revision = ConfigRevision(document_hash(&layers.persisted, &layers.unknown));
        let snapshot = Arc::new(ConfigSnapshot {
            configuration: layers.effective,
            persisted_configuration: layers.persisted,
            revision,
            unknown: layers.unknown,
            source_path: path.to_path_buf(),
            loaded_from_disk,
            resolved,
        });
        self.replace_active(Arc::clone(&snapshot))?;
        Ok(LoadReport {
            snapshot,
            findings: layers.findings,
        })
    }

    /// Reloads the file while retaining the last valid active snapshot on failure.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for a rejected candidate or read failure.
    pub fn reload(&self) -> Result<LoadReport, ConfigError> {
        let report = self.load_initial()?;
        self.publish_change(&report.snapshot)?;
        Ok(report)
    }

    #[must_use]
    pub fn snapshot(&self) -> Option<Arc<ConfigSnapshot>> {
        self.active.read().ok().and_then(|active| active.clone())
    }

    /// Atomically saves the persisted portion of a snapshot. Runtime overrides are never written.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for conflicts, serialization, or filesystem failures.
    pub fn save(&self, snapshot: &ConfigSnapshot) -> Result<SaveResult, ConfigError> {
        let expected = self.snapshot().map(|value| value.revision.clone());
        self.save_if_revision(expected.as_ref(), snapshot)
    }

    /// Atomically saves a snapshot only when both the active and on-disk revisions match.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] for conflicts, serialization, or filesystem failures.
    pub fn save_if_revision(
        &self,
        expected: Option<&ConfigRevision>,
        snapshot: &ConfigSnapshot,
    ) -> Result<SaveResult, ConfigError> {
        let _guard = self.write_guard.lock().map_err(|_| ConfigError::Poisoned)?;
        if let Some(expected) = expected {
            let active = self.snapshot().ok_or(ConfigError::Conflict)?;
            if &active.revision != expected || active.source_path != snapshot.source_path {
                return Err(ConfigError::Conflict);
            }
        }
        let disk_revision =
            revision_from_disk(&snapshot.source_path, self.catalog_default.as_deref())?;
        let expected_disk = snapshot.loaded_from_disk.then_some(expected).flatten();
        if disk_revision.as_ref() != expected_disk
            && !(expected.is_some() && !snapshot.loaded_from_disk && disk_revision.is_none())
        {
            return Err(ConfigError::Conflict);
        }
        let text = canonical_document(&snapshot.persisted_configuration, &snapshot.unknown);
        atomic_write(&snapshot.source_path, text.as_bytes())?;
        let revision = ConfigRevision(document_hash(
            &snapshot.persisted_configuration,
            &snapshot.unknown,
        ));
        let stored = Arc::new(ConfigSnapshot {
            configuration: snapshot.configuration.clone(),
            persisted_configuration: snapshot.persisted_configuration.clone(),
            resolved: snapshot.resolved,
            revision: revision.clone(),
            unknown: snapshot.unknown.clone(),
            source_path: snapshot.source_path.clone(),
            loaded_from_disk: true,
        });
        self.replace_active(Arc::clone(&stored))?;
        self.publish_change(&stored)?;
        Ok(SaveResult { revision })
    }

    #[must_use]
    pub fn subscribe(&self) -> mpsc::Receiver<ConfigChange> {
        let (sender, receiver) = mpsc::sync_channel(32);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            subscribers.push(sender);
        }
        receiver
    }

    fn replace_active(&self, snapshot: Arc<ConfigSnapshot>) -> Result<(), ConfigError> {
        let mut active = self.active.write().map_err(|_| ConfigError::Poisoned)?;
        *active = Some(snapshot);
        Ok(())
    }

    fn publish_change(&self, snapshot: &Arc<ConfigSnapshot>) -> Result<(), ConfigError> {
        let mut subscribers = self.subscribers.lock().map_err(|_| ConfigError::Poisoned)?;
        subscribers.retain(|sender| {
            match sender.try_send(ConfigChange {
                revision: snapshot.revision.clone(),
                changed_fields: vec!["configuration".to_owned()],
                snapshot: Arc::clone(snapshot),
            }) {
                Ok(()) | Err(mpsc::TrySendError::Full(_)) => true,
                Err(mpsc::TrySendError::Disconnected(_)) => false,
            }
        });
        Ok(())
    }
}

fn apply_catalog_default(
    persisted: &mut Configuration,
    effective: &mut Configuration,
    default_path: Option<&Path>,
    environment: &EnvironmentOverrides,
    startup: &BTreeMap<String, OverrideValue>,
) {
    let startup_has_path = startup.contains_key("catalog.path");
    if persisted.catalog.path.is_none()
        && effective.catalog.path.is_none()
        && environment.catalog_file.is_none()
        && !startup_has_path
        && let Some(path) = default_path.and_then(Path::to_str)
    {
        persisted.catalog.path = Some(path.to_owned());
        effective.catalog.path = Some(path.to_owned());
    }
}

fn read_optional(path: &Path) -> Result<Option<String>, ConfigError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(ConfigError::invalid(
            "config.toml",
            "symlink paths are not accepted",
        )),
        Ok(_) => fs::read_to_string(path)
            .map(Some)
            .map_err(|_| ConfigError::Io(IoStage::Read)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ConfigError::Io(IoStage::Read)),
    }
}

fn revision_from_disk(
    path: &Path,
    catalog_default: Option<&Path>,
) -> Result<Option<ConfigRevision>, ConfigError> {
    let Some(text) = read_optional(path)? else {
        return Ok(None);
    };
    let environment = EnvironmentOverrides::default();
    let mut layers = resolve_layered(Some(&text), &environment, &BTreeMap::new())
        .map_err(|_| ConfigError::Conflict)?;
    apply_catalog_default(
        &mut layers.persisted,
        &mut layers.effective,
        catalog_default,
        &environment,
        &BTreeMap::new(),
    );
    Ok(Some(ConfigRevision(document_hash(
        &layers.persisted,
        &layers.unknown,
    ))))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| ConfigError::invalid("config.toml", "path has no parent"))?;
    fs::create_dir_all(parent).map_err(|_| ConfigError::Io(IoStage::CreateDirectory))?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(ConfigError::invalid(
            "config.toml",
            "symlink paths are not accepted",
        ));
    }
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("config"),
        std::process::id()
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| ConfigError::Io(IoStage::CreateTemporary))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| ConfigError::Io(IoStage::Permissions))?;
    }
    let result = (|| {
        file.write_all(bytes)
            .map_err(|_| ConfigError::Io(IoStage::Write))?;
        file.flush().map_err(|_| ConfigError::Io(IoStage::Flush))?;
        file.sync_all()
            .map_err(|_| ConfigError::Io(IoStage::Sync))?;
        fs::rename(&temporary, path).map_err(|_| ConfigError::Io(IoStage::Replace))?;
        sync_parent(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn sync_parent(parent: &Path) -> Result<(), ConfigError> {
    #[cfg(unix)]
    {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| ConfigError::Io(IoStage::SyncDirectory))?;
    }
    #[cfg(not(unix))]
    let _ = parent;
    Ok(())
}

fn resolve_runtime(
    configuration: &Configuration,
    capabilities: RuntimeCapabilities,
    findings: &mut Vec<ConfigFinding>,
) -> ResolvedConfiguration {
    let available = u16::try_from(
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .clamp(1, 32),
    )
    .expect("parallelism is clamped to u16");
    let cpu_threads = if configuration.processing.cpu_threads == 0 {
        available
    } else {
        if usize::from(configuration.processing.cpu_threads) > usize::from(available) {
            findings.push(ConfigFinding {
                kind: ConfigFindingKind::PerformanceWarning,
                code: "configuration.cpu_threads_exceed_available".to_owned(),
                field: "processing.cpu_threads".to_owned(),
            });
        }
        configuration.processing.cpu_threads
    };
    let host_memory_mib = if configuration.processing.host_memory_mib == 0 {
        capabilities.physical_memory_mib.map_or_else(
            || {
                findings.push(ConfigFinding {
                    kind: ConfigFindingKind::Fallback,
                    code: "configuration.memory_detection_unavailable".to_owned(),
                    field: "processing.host_memory_mib".to_owned(),
                });
                1024
            },
            |memory| (memory / 4).clamp(512, 8192),
        )
    } else {
        configuration.processing.host_memory_mib
    };
    let compute_backend = match configuration.gpu.mode {
        GpuMode::Cpu => ComputeBackend::Cpu,
        GpuMode::Auto => {
            if capabilities.qualified_gpu {
                ComputeBackend::Gpu
            } else {
                ComputeBackend::Cpu
            }
        }
        GpuMode::Gpu if capabilities.qualified_gpu => ComputeBackend::Gpu,
        GpuMode::Gpu => {
            findings.push(ConfigFinding {
                kind: ConfigFindingKind::GpuUnavailable,
                code: "configuration.gpu_unavailable".to_owned(),
                field: "gpu.mode".to_owned(),
            });
            ComputeBackend::CpuFallback
        }
    };
    ResolvedConfiguration {
        cpu_threads,
        host_memory_mib,
        compute_backend,
    }
}

#[allow(dead_code)]
fn _merge_check(document: &mut toml::Value, unknown: &UnknownFields) {
    merge_unknown(document, unknown);
}
