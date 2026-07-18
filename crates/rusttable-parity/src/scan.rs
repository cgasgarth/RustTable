use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::mapping::map_discovered;
use crate::model::{Capability, Discovered, Manifest, OverrideFile};
use crate::validate::{summary_for, validate_capability_fields, validate_manifest};

#[derive(Debug, PartialEq, Eq)]
pub enum ScanError {
    Io { path: String, message: String },
    InvalidOverrides { message: String },
    InvalidManifest { message: String },
    MissingReferencePath { path: String },
    MissingSurface { path: String },
    InvalidStatus { value: String, id: String },
    StaleIssueSequence { sequence: String, id: String },
    DuplicateCapabilityId { id: String },
    UnmappedDiscoveredModule { id: String, path: String },
    MaskingOverride { id: String },
    InvalidOverride { id: String, message: String },
    UnregisteredOpenclProgram { id: String, path: String },
    Serialization { message: String },
}

impl Display for ScanError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, message } => write!(formatter, "I/O error at {path}: {message}"),
            Self::InvalidOverrides { message } => write!(formatter, "invalid overrides: {message}"),
            Self::InvalidManifest { message } => write!(formatter, "invalid manifest: {message}"),
            Self::MissingReferencePath { path } => {
                write!(formatter, "missing reference path: {path}")
            }
            Self::MissingSurface { path } => write!(formatter, "missing reference surface: {path}"),
            Self::InvalidStatus { value, id } => {
                write!(formatter, "invalid status {value:?} for {id}")
            }
            Self::StaleIssueSequence { sequence, id } => {
                write!(formatter, "stale issue sequence {sequence:?} for {id}")
            }
            Self::DuplicateCapabilityId { id } => {
                write!(formatter, "duplicate capability ID: {id}")
            }
            Self::UnmappedDiscoveredModule { id, path } => {
                write!(formatter, "unmapped discovered module {id} at {path}")
            }
            Self::MaskingOverride { id } => {
                write!(formatter, "override masks discoverable capability: {id}")
            }
            Self::InvalidOverride { id, message } => {
                write!(formatter, "invalid override {id}: {message}")
            }
            Self::UnregisteredOpenclProgram { id, path } => {
                write!(formatter, "unregistered OpenCL program {id} at {path}")
            }
            Self::Serialization { message } => {
                write!(formatter, "manifest serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for ScanError {}

/// Scans the pinned reference tree using the reviewed override file.
///
/// # Errors
///
/// Returns a bounded diagnostic when a registration surface, source path,
/// mapping, override, or manifest invariant is invalid.
pub fn scan_darktable(source: &Path, overrides: &Path) -> Result<Manifest, ScanError> {
    let contents = read_text(overrides)?;
    scan_darktable_with_overrides(source, &contents)
}

/// Scans a reference tree with override TOML supplied by the caller.
///
/// # Errors
///
/// Returns a bounded diagnostic when discovery, mapping, override, or
/// deterministic-manifest validation fails.
pub fn scan_darktable_with_overrides(
    source: &Path,
    overrides: &str,
) -> Result<Manifest, ScanError> {
    require_directory(source)?;
    let mut capabilities = Vec::new();
    discover_cmake_surface(source, "iop", "src/iop", &mut capabilities)?;
    discover_cmake_surface(source, "lib", "src/libs", &mut capabilities)?;
    discover_cmake_surface(source, "view", "src/views", &mut capabilities)?;
    discover_cmake_surface(source, "format", "src/imageio/format", &mut capabilities)?;
    discover_storage_surface(source, &mut capabilities)?;
    discover_lua_surface(source, &mut capabilities)?;
    discover_build_options(source, &mut capabilities)?;
    discover_opencl_surface(source, &mut capabilities)?;

    let override_file: OverrideFile = if overrides.trim().is_empty() {
        OverrideFile {
            override_entries: Vec::new(),
        }
    } else {
        toml::from_str(overrides).map_err(|error| ScanError::InvalidOverrides {
            message: error.to_string(),
        })?
    };
    apply_overrides(source, &mut capabilities, override_file.override_entries)?;

    normalize_capabilities(&mut capabilities);
    capabilities.sort_by(|left, right| left.id.cmp(&right.id));
    let mut manifest = Manifest {
        schema_version: 1,
        source_commit: reference_commit(source),
        summary: Vec::new(),
        capabilities,
    };
    manifest.summary = summary_for(&manifest.capabilities);
    validate_manifest(&manifest)?;
    Ok(manifest)
}

fn discover_cmake_surface(
    root: &Path,
    kind: &str,
    relative_directory: &str,
    capabilities: &mut Vec<Capability>,
) -> Result<(), ScanError> {
    let directory = root.join(relative_directory);
    let cmake_path = directory.join("CMakeLists.txt");
    let contents = read_text(&cmake_path)?;
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        let tokens = cmake_tokens(trimmed);
        let (name, source) =
            if kind == "iop" && tokens.first().map(String::as_str) == Some("add_iop") {
                (tokens.get(1), tokens.get(2))
            } else if kind != "iop" && tokens.first().map(String::as_str) == Some("add_library") {
                (tokens.get(1), tokens.get(3))
            } else {
                continue;
            };
        let (Some(name), Some(source)) = (name, source) else {
            continue;
        };
        let relative_path = normalize_relative_path(relative_directory, source);
        require_file(root, &relative_path)?;
        add_discovered(capabilities, kind, name, &relative_path)?;
    }
    Ok(())
}

fn discover_storage_surface(
    root: &Path,
    capabilities: &mut Vec<Capability>,
) -> Result<(), ScanError> {
    let relative_directory = "src/imageio/storage";
    let cmake_path = root.join(relative_directory).join("CMakeLists.txt");
    let contents = read_text(&cmake_path)?;
    for line in contents.lines() {
        let tokens = cmake_tokens(line.trim());
        if tokens.first().map(String::as_str) != Some("set")
            || tokens.get(1).map(String::as_str) != Some("MODULES")
        {
            continue;
        }
        for name in tokens.iter().skip(2) {
            if name.starts_with('$') {
                continue;
            }
            let relative_path = format!("{relative_directory}/{name}.c");
            require_file(root, &relative_path)?;
            add_discovered(capabilities, "storage", name, &relative_path)?;
        }
    }
    Ok(())
}

fn discover_lua_surface(root: &Path, capabilities: &mut Vec<Capability>) -> Result<(), ScanError> {
    let relative_path = "src/lua/init.c";
    let contents = read_text(&root.join(relative_path))?;
    let mut in_registry = false;
    let mut names = Vec::new();
    for line in contents.lines() {
        if line.contains("early_init_funcs") || line.contains("init_funcs") {
            in_registry = true;
        }
        if in_registry {
            for token in
                line.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            {
                if let Some(name) = token.strip_prefix("dt_lua_init_")
                    && !name.is_empty()
                    && !names.iter().any(|known| known == name)
                {
                    names.push(name.to_owned());
                }
            }
        }
        if in_registry && line.contains("};") {
            in_registry = false;
        }
    }
    names.sort();
    for name in names {
        add_discovered(capabilities, "lua", &name, relative_path)?;
    }
    Ok(())
}

fn discover_build_options(
    root: &Path,
    capabilities: &mut Vec<Capability>,
) -> Result<(), ScanError> {
    let relative_path = "DefineOptions.cmake";
    let contents = read_text(&root.join(relative_path))?;
    for line in contents.lines() {
        let tokens = cmake_tokens(line.trim());
        if tokens.first().map(String::as_str) != Some("option") {
            continue;
        }
        if let Some(name) = tokens.get(1) {
            add_discovered(capabilities, "build-option", name, relative_path)?;
        }
    }
    Ok(())
}

fn discover_opencl_surface(
    root: &Path,
    capabilities: &mut Vec<Capability>,
) -> Result<(), ScanError> {
    let relative_registry = "data/kernels/programs.conf";
    let contents = read_text(&root.join(relative_registry))?;
    let mut registered = Vec::new();
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(file) = trimmed.split_whitespace().next() else {
            continue;
        };
        let Some(name) = file.strip_suffix(".cl") else {
            continue;
        };
        let relative_path = format!("data/kernels/{file}");
        require_file(root, &relative_path)?;
        registered.push(file.to_owned());
        add_discovered(capabilities, "opencl", name, &relative_path)?;
    }
    let directory = root.join("data/kernels");
    let mut paths = read_directory(&directory)?;
    paths.sort();
    for path in paths {
        if path.extension().and_then(|extension| extension.to_str()) != Some("cl") {
            continue;
        }
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        if !registered.iter().any(|known| known == file) {
            let name = file.strip_suffix(".cl").unwrap_or(file);
            return Err(ScanError::UnregisteredOpenclProgram {
                id: format!("opencl.{name}"),
                path: format!("data/kernels/{file}"),
            });
        }
    }
    Ok(())
}

fn add_discovered(
    capabilities: &mut Vec<Capability>,
    kind: &str,
    name: &str,
    relative_path: &str,
) -> Result<(), ScanError> {
    let Some(mapped) = map_discovered(kind, name, relative_path) else {
        return Err(ScanError::UnmappedDiscoveredModule {
            id: format!("{kind}.{name}"),
            path: relative_path.to_owned(),
        });
    };
    let capability = capability_from_discovered(mapped);
    if let Some(existing) = capabilities.iter().find(|known| known.id == capability.id) {
        if existing.reference_path == capability.reference_path {
            return Ok(());
        }
        return Err(ScanError::DuplicateCapabilityId { id: capability.id });
    }
    capabilities.push(capability);
    Ok(())
}

fn apply_overrides(
    root: &Path,
    capabilities: &mut Vec<Capability>,
    overrides: Vec<crate::model::Override>,
) -> Result<(), ScanError> {
    let mut override_ids = Vec::new();
    for entry in overrides {
        validate_capability_fields(&entry.id, &entry.status, &entry.issue_sequences)?;
        if entry.reason.trim().is_empty() {
            return Err(ScanError::InvalidOverride {
                id: entry.id,
                message: "reason is required".to_owned(),
            });
        }
        require_file(root, &entry.reference_path)?;
        if override_ids.iter().any(|id| id == &entry.id) {
            return Err(ScanError::DuplicateCapabilityId { id: entry.id });
        }
        override_ids.push(entry.id.clone());
        if capabilities.iter().any(|capability| {
            capability.id == entry.id || capability.reference_path == entry.reference_path
        }) {
            return Err(ScanError::MaskingOverride { id: entry.id });
        }
        let capability = Capability {
            id: entry.id,
            reference_path: entry.reference_path,
            reference_symbol: entry
                .reference_symbol
                .unwrap_or_else(|| "cross-cutting".to_owned()),
            category: entry.category,
            status: entry.status,
            issue_sequences: entry.issue_sequences,
            test_evidence: entry.test_evidence,
            redesign_note: entry.redesign_note,
        };
        capabilities.push(capability);
    }
    Ok(())
}

fn capability_from_discovered(discovered: Discovered) -> Capability {
    Capability {
        id: discovered.id,
        reference_path: discovered.reference_path,
        reference_symbol: discovered.reference_symbol,
        category: discovered.category.to_owned(),
        status: discovered.status.to_owned(),
        issue_sequences: discovered.issue_sequences,
        test_evidence: discovered.test_evidence,
        redesign_note: discovered.redesign_note,
    }
}

fn normalize_capabilities(capabilities: &mut [Capability]) {
    for capability in capabilities {
        capability.issue_sequences.sort();
        capability.issue_sequences.dedup();
        capability.test_evidence.sort();
        capability.test_evidence.dedup();
    }
}

fn cmake_tokens(line: &str) -> Vec<String> {
    let Some(open) = line.find('(') else {
        return Vec::new();
    };
    let command = line[..open].trim();
    let body = line[open + 1..].trim_end_matches(')').trim();
    std::iter::once(command.to_owned())
        .chain(
            body.split_whitespace()
                .map(|token| token.trim_matches('"').to_owned()),
        )
        .collect()
}

fn normalize_relative_path(directory: &str, source: &str) -> String {
    let directory_path = PathBuf::from(directory);
    let mut components = directory_path.components().collect::<Vec<_>>();
    let source_path = PathBuf::from(source);
    for component in source_path.components() {
        match component {
            Component::ParentDir => {
                components.pop();
            }
            Component::CurDir => {}
            other => components.push(other),
        }
    }
    components
        .into_iter()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str().map(str::to_owned),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn require_directory(path: &Path) -> Result<(), ScanError> {
    if path.is_dir() {
        Ok(())
    } else {
        Err(ScanError::MissingSurface {
            path: path.display().to_string(),
        })
    }
}

fn require_file(root: &Path, relative_path: &str) -> Result<(), ScanError> {
    if root.join(relative_path).is_file() {
        Ok(())
    } else {
        Err(ScanError::MissingReferencePath {
            path: relative_path.to_owned(),
        })
    }
}

fn read_text(path: &Path) -> Result<String, ScanError> {
    fs::read_to_string(path).map_err(|error| ScanError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

fn read_directory(path: &Path) -> Result<Vec<PathBuf>, ScanError> {
    fs::read_dir(path)
        .map_err(|error| ScanError::Io {
            path: path.display().to_string(),
            message: error.to_string(),
        })?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|error| ScanError::Io {
                    path: path.display().to_string(),
                    message: error.to_string(),
                })
        })
        .collect()
}

fn reference_commit(source: &Path) -> String {
    let source_text = source.display().to_string();
    let output = Command::new("git")
        .args(["-C", &source_text, "rev-parse", "--verify", "HEAD"])
        .output();
    output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map_or_else(|| "fixture".to_owned(), |commit| commit.trim().to_owned())
}
