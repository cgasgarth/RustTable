use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::package::{ModelPackage, PackageError, PackageLimits};
use crate::{ModelIdentity, ModelTask};
use rusttable_ai_native::RuntimeAdapter;

static INSTALL_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const STATE_FILE: &str = "registry-state.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledModel {
    identity: ModelIdentity,
    id: String,
    version: String,
    task: ModelTask,
    root: PathBuf,
    enabled: bool,
}

impl InstalledModel {
    #[must_use]
    pub const fn identity(&self) -> ModelIdentity {
        self.identity
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    #[must_use]
    pub const fn task(&self) -> ModelTask {
        self.task
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub fn storage_root(&self) -> &Path {
        &self.root
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRegistrySnapshot {
    models: Vec<InstalledModel>,
}

impl ModelRegistrySnapshot {
    #[must_use]
    pub fn models(&self) -> &[InstalledModel] {
        &self.models
    }

    pub fn enabled(&self) -> impl Iterator<Item = &InstalledModel> {
        self.models.iter().filter(|model| model.enabled)
    }
}

#[derive(Debug)]
pub struct ModelRegistry {
    root: PathBuf,
    limits: PackageLimits,
    models: BTreeMap<ModelIdentity, InstalledModel>,
    active: BTreeMap<ModelIdentity, u32>,
}

impl ModelRegistry {
    pub fn open(root: impl Into<PathBuf>, limits: PackageLimits) -> Result<Self, RegistryError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|source| RegistryError::Io {
            operation: "create registry",
            source,
        })?;
        let mut registry = Self {
            root,
            limits,
            models: BTreeMap::new(),
            active: BTreeMap::new(),
        };
        registry.reconcile()?;
        Ok(registry)
    }

    pub fn install_bytes(&mut self, bytes: &[u8]) -> Result<InstalledModel, RegistryError> {
        let package =
            ModelPackage::from_rtmodel(bytes, self.limits).map_err(RegistryError::Package)?;
        self.install_package(&package)
    }

    pub fn install_bytes_with_adapter(
        &mut self,
        bytes: &[u8],
        adapter: &dyn RuntimeAdapter,
    ) -> Result<InstalledModel, RegistryError> {
        let package =
            ModelPackage::from_rtmodel(bytes, self.limits).map_err(RegistryError::Package)?;
        let graph = adapter
            .inspect_model(package.model_bytes())
            .map_err(RegistryError::Adapter)?;
        package
            .validate_graph(&graph)
            .map_err(RegistryError::Contract)?;
        self.install_package(&package)
    }

    pub fn install_package(
        &mut self,
        package: &ModelPackage,
    ) -> Result<InstalledModel, RegistryError> {
        let identity = package.identity();
        if let Some(existing) = self.models.get(&identity) {
            return Ok(existing.clone());
        }
        let final_root = self.root.join(identity.to_hex());
        let sequence = INSTALL_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = self
            .root
            .join(format!(".install-{}-{sequence}", identity.to_hex()));
        if temporary.exists() {
            fs::remove_dir_all(&temporary).map_err(|source| RegistryError::Io {
                operation: "remove stale install",
                source,
            })?;
        }
        fs::create_dir(&temporary).map_err(|source| RegistryError::Io {
            operation: "create model staging",
            source,
        })?;
        let result = write_package(&temporary, package);
        if let Err(error) = result {
            let _ = fs::remove_dir_all(&temporary);
            return Err(error);
        }
        if final_root.exists() {
            fs::remove_dir_all(&temporary).map_err(|source| RegistryError::Io {
                operation: "remove duplicate staging",
                source,
            })?;
        } else {
            fs::rename(&temporary, &final_root).map_err(|source| RegistryError::Io {
                operation: "commit model",
                source,
            })?;
        }
        let installed = InstalledModel {
            identity,
            id: package.manifest().id.clone(),
            version: package.manifest().version.clone(),
            task: package.manifest().task,
            root: final_root,
            enabled: true,
        };
        self.models.insert(identity, installed.clone());
        self.persist_state()?;
        Ok(installed)
    }

    pub fn set_enabled(
        &mut self,
        identity: ModelIdentity,
        enabled: bool,
    ) -> Result<(), RegistryError> {
        let model = self
            .models
            .get_mut(&identity)
            .ok_or(RegistryError::NotInstalled)?;
        model.enabled = enabled;
        self.persist_state()
    }

    pub fn acquire(&mut self, identity: ModelIdentity) -> Result<(), RegistryError> {
        let model = self
            .models
            .get(&identity)
            .ok_or(RegistryError::NotInstalled)?;
        if !model.enabled {
            return Err(RegistryError::Disabled);
        }
        let count = self.active.entry(identity).or_default();
        *count = count.checked_add(1).ok_or(RegistryError::ResourceLimit)?;
        Ok(())
    }

    pub fn release(&mut self, identity: ModelIdentity) {
        if let Some(count) = self.active.get_mut(&identity) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.active.remove(&identity);
            }
        }
    }

    pub fn remove(&mut self, identity: ModelIdentity) -> Result<(), RegistryError> {
        if self.active.get(&identity).copied().unwrap_or(0) > 0 {
            return Err(RegistryError::InUse);
        }
        let model = self
            .models
            .remove(&identity)
            .ok_or(RegistryError::NotInstalled)?;
        fs::remove_dir_all(model.root).map_err(|source| RegistryError::Io {
            operation: "remove model",
            source,
        })?;
        self.persist_state()
    }

    #[must_use]
    pub fn snapshot(&self) -> ModelRegistrySnapshot {
        ModelRegistrySnapshot {
            models: self.models.values().cloned().collect(),
        }
    }

    pub fn reconcile(&mut self) -> Result<(), RegistryError> {
        let state = read_state(&self.root)?;
        self.models.clear();
        for entry in fs::read_dir(&self.root).map_err(|source| RegistryError::Io {
            operation: "read registry",
            source,
        })? {
            let entry = entry.map_err(|source| RegistryError::Io {
                operation: "read registry entry",
                source,
            })?;
            let path = entry.path();
            if !entry
                .file_type()
                .map_err(|source| RegistryError::Io {
                    operation: "inspect registry entry",
                    source,
                })?
                .is_dir()
                || path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with(".install-"))
            {
                continue;
            }
            let manifest_text = fs::read_to_string(path.join("model.toml")).map_err(|source| {
                RegistryError::Io {
                    operation: "read installed manifest",
                    source,
                }
            })?;
            let manifest = crate::contracts::RegistryManifest::from_toml(&manifest_text)
                .map_err(RegistryError::Contract)?;
            let model = fs::read(path.join("model.onnx")).map_err(|source| RegistryError::Io {
                operation: "read installed model",
                source,
            })?;
            let assets = manifest
                .data_assets
                .iter()
                .map(|asset| {
                    fs::read(path.join(&asset.name))
                        .map(|bytes| (asset.name.clone(), bytes))
                        .map_err(|source| RegistryError::Io {
                            operation: "read installed asset",
                            source,
                        })
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let identity = ModelIdentity::from_canonical(&manifest, &model, &assets)
                .map_err(RegistryError::Package)?;
            if path.file_name().and_then(|name| name.to_str()) != Some(identity.to_hex().as_str()) {
                return Err(RegistryError::IdentityMismatch);
            }
            self.models.insert(
                identity,
                InstalledModel {
                    identity,
                    id: manifest.id,
                    version: manifest.version,
                    task: manifest.task,
                    root: path,
                    enabled: state.get(&identity.to_hex()).copied().unwrap_or(true),
                },
            );
        }
        Ok(())
    }
}

fn write_package(root: &Path, package: &ModelPackage) -> Result<(), RegistryError> {
    let manifest =
        toml::to_string(package.manifest()).map_err(|_| RegistryError::ManifestEncoding)?;
    write_file(&root.join("model.toml"), manifest.as_bytes())?;
    write_file(&root.join("model.onnx"), package.model_bytes())?;
    for (name, bytes) in package.data_assets() {
        let path = root.join(name);
        if path
            .parent()
            .is_some_and(|parent| !parent.starts_with(root))
        {
            return Err(RegistryError::UnsafePath);
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| RegistryError::Io {
                operation: "create installed asset directory",
                source,
            })?;
        }
        write_file(&path, bytes)?;
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| RegistryError::Io {
            operation: "write installed model",
            source,
        })?;
    file.write_all(bytes).map_err(|source| RegistryError::Io {
        operation: "write installed model",
        source,
    })?;
    file.sync_all().map_err(|source| RegistryError::Io {
        operation: "sync installed model",
        source,
    })
}

fn read_state(root: &Path) -> Result<BTreeMap<String, bool>, RegistryError> {
    let path = root.join(STATE_FILE);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let bytes = fs::read(path).map_err(|source| RegistryError::Io {
        operation: "read registry state",
        source,
    })?;
    serde_json::from_slice(&bytes).map_err(|_| RegistryError::StateEncoding)
}

impl ModelRegistry {
    fn persist_state(&self) -> Result<(), RegistryError> {
        let state = self
            .models
            .iter()
            .map(|(identity, model)| (identity.to_hex(), model.enabled))
            .collect::<BTreeMap<_, _>>();
        let bytes = serde_json::to_vec(&state).map_err(|_| RegistryError::StateEncoding)?;
        let temporary = self.root.join(".registry-state.tmp");
        write_file_replace(&temporary, &bytes)?;
        fs::rename(temporary, self.root.join(STATE_FILE)).map_err(|source| RegistryError::Io {
            operation: "commit registry state",
            source,
        })
    }
}

fn write_file_replace(path: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|source| RegistryError::Io {
            operation: "write registry state",
            source,
        })?;
    file.write_all(bytes).map_err(|source| RegistryError::Io {
        operation: "write registry state",
        source,
    })?;
    file.sync_all().map_err(|source| RegistryError::Io {
        operation: "sync registry state",
        source,
    })
}

#[derive(Debug)]
pub enum RegistryError {
    Package(PackageError),
    Contract(crate::contracts::ContractError),
    Adapter(rusttable_ai_native::AdapterError),
    Io {
        operation: &'static str,
        source: io::Error,
    },
    ManifestEncoding,
    StateEncoding,
    UnsafePath,
    IdentityMismatch,
    NotInstalled,
    Disabled,
    InUse,
    ResourceLimit,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "AI model registry error: {self:?}")
    }
}

impl std::error::Error for RegistryError {}
